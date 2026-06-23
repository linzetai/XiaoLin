use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use futures::StreamExt;
use xiaolin_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolResult};
use xiaolin_security::ssrf::{
    build_pinned_client, ssrf_check_parsed_url_pinned, ssrf_safe_redirect_policy,
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

const HTTP_FETCH_MAX_BODY_BYTES: usize = 5 * 1024 * 1024;

fn build_reqwest_client(
    name: &'static str,
    configure: impl FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder,
) -> reqwest::Client {
    match configure(reqwest::Client::builder()).build() {
        Ok(client) => client,
        Err(e) => {
            tracing::error!(
                client = name,
                error = %e,
                "failed to build reqwest client; using default"
            );
            reqwest::Client::new()
        }
    }
}

fn shared_api_search_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            build_reqwest_client("api_search", |b| {
                b.timeout(std::time::Duration::from_secs(15))
                    .user_agent("XiaoLin/0.1.0")
            })
        })
        .clone()
}

async fn read_response_body_limited(
    resp: reqwest::Response,
    max_bytes: usize,
) -> Result<String, reqwest::Error> {
    let mut body = String::new();
    let mut total = 0usize;
    let mut truncated = false;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        total += chunk.len();
        if total > max_bytes {
            let already = body.len();
            let remaining = max_bytes.saturating_sub(already);
            if remaining > 0 {
                body.push_str(&String::from_utf8_lossy(&chunk[..remaining.min(chunk.len())]));
            }
            truncated = true;
            break;
        }
        body.push_str(&String::from_utf8_lossy(&chunk));
    }

    if truncated {
        body.push_str(&format!("... [truncated, >={total} bytes total]"));
    }
    Ok(body)
}

fn ssrf_pinned_client_for_url(
    url: &str,
    configure: impl FnOnce(reqwest::ClientBuilder) -> reqwest::ClientBuilder,
) -> Result<reqwest::Client, String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;
    let host = parsed.host_str().ok_or("URL has no host")?;
    let verified_addrs = ssrf_check_parsed_url_pinned(&parsed)?;
    build_pinned_client(host, &verified_addrs, configure)
}

/// HTTP fetch tool — performs one HTTP request to a URL.
pub struct HttpFetchTool;

fn parse_http_fetch_method(
    v: Option<&serde_json::Value>,
) -> std::result::Result<reqwest::Method, String> {
    let s = match v {
        None | Some(serde_json::Value::Null) => {
            return Ok(reqwest::Method::GET);
        }
        Some(serde_json::Value::String(s)) => s.as_str(),
        _ => {
            return Err(
                "http_fetch: 'method' must be a string, or omitted. \
                 What to do next: use \"GET\", \"POST\", etc., e.g. {\"url\": \"https://api.example.com/v1/r\", \"method\": \"POST\"}."
                    .to_string(),
            );
        }
    };
    let t = s.trim();
    if t.is_empty() {
        return Ok(reqwest::Method::GET);
    }
    match t.to_uppercase().as_str() {
        "GET" => Ok(reqwest::Method::GET),
        "POST" => Ok(reqwest::Method::POST),
        "PUT" => Ok(reqwest::Method::PUT),
        "DELETE" => Ok(reqwest::Method::DELETE),
        "PATCH" => Ok(reqwest::Method::PATCH),
        "HEAD" => Ok(reqwest::Method::HEAD),
        _ => Err(
            "http_fetch: 'method' must be one of GET, POST, PUT, DELETE, PATCH, or HEAD, or omitted (defaults to GET). \
             What to do next: set \"method\" to a supported token (uppercase in schema; matching is case-insensitive) or omit it."
                .to_string(),
        ),
    }
}

impl Default for HttpFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpFetchTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for HttpFetchTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Fetch
    }
    fn supports_parallel(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "http_fetch"
    }

    fn description(&self) -> &str {
        "Raw HTTP client for REST API calls, webhooks, and JSON/XML responses. \
         Supports GET/POST/PUT/DELETE/PATCH/HEAD with custom headers, auth tokens, and request body. \
         Returns raw response (status, headers, body). Body truncated at ~5MB; timeout 10s. \
         SSRF protection blocks localhost and private IPs. \
         DO NOT use for reading web pages — use web_fetch (which extracts readable text from HTML)."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "url".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Absolute http(s) URL."
            }),
        );
        props.insert(
            "method".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD"],
                "default": "GET",
                "description": "HTTP method (default GET)."
            }),
        );
        props.insert(
            "headers".to_string(),
            serde_json::json!({
                "type": "object",
                "description": "Optional request headers as {name: value} map."
            }),
        );
        props.insert(
            "body".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional request body (POST/PUT/PATCH only)."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["url".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "http_fetch arguments are not valid JSON: {e}. \
                 Pass {{\"url\": \"https://example.com/path\"}} with a string URL; optional \"method\", \"headers\", and \"body\" as needed, then retry."
            )),
        };

        let url = match args.get("url").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return ToolResult::err(
                    "http_fetch is missing string field 'url'. \
                 Example: {\"url\": \"https://httpbin.org/get\"}. \
                 Relative paths like '/api' are not accepted—include scheme and host."
                        .to_string(),
                )
            }
        };

        let method = match parse_http_fetch_method(args.get("method")) {
            Ok(m) => m,
            Err(e) => return ToolResult::err(e),
        };

        let mut request_headers = HeaderMap::new();
        match args.get("headers") {
            None | Some(serde_json::Value::Null) => {}
            Some(serde_json::Value::Object(map)) => {
                for (k, v) in map {
                    let val = match v.as_str() {
                        Some(s) => s,
                        None => {
                            return ToolResult::err(
                                "http_fetch: 'headers' values must be strings. \
                                 What to do next: use only string values, e.g. {\"Content-Type\": \"application/json\"}."
                                    .to_string(),
                            );
                        }
                    };
                    let name = match HeaderName::from_str(k) {
                        Ok(n) => n,
                        Err(e) => {
                            return ToolResult::err(format!(
                                "http_fetch: invalid header name '{k}': {e}. \
                                 What to do next: use a valid header field name, or remove the key."
                            ));
                        }
                    };
                    let value = match HeaderValue::from_str(val) {
                        Ok(v) => v,
                        Err(e) => {
                            return ToolResult::err(format!(
                                "http_fetch: invalid header value for '{k}': {e}. \
                                 What to do next: use ASCII-only header values or encodings the API documents (e.g. base64 in Authorization)."
                            ));
                        }
                    };
                    if request_headers.insert(name, value).is_some() {
                        return ToolResult::err(format!(
                            "http_fetch: duplicate header '{k}'. \
                             What to do next: send each header name once, or merge values per RFC if the API allows."
                        ));
                    }
                }
            }
            _ => {
                return ToolResult::err(
                    "http_fetch: 'headers' must be a JSON object or omitted. \
                     What to do next: use {{\"Content-Type\": \"application/json\"}} with string values, or omit 'headers'."
                        .to_string(),
                );
            }
        }

        let client = match ssrf_pinned_client_for_url(url, |b| {
            b.timeout(std::time::Duration::from_secs(10)).redirect(ssrf_safe_redirect_policy())
        }) {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::err(format!(
                    "http_fetch URL was rejected before the HTTP request: {e}. \
                     Use a public http(s) URL that resolves outside private networks; avoid localhost, RFC1918 ranges, link-local addresses, and non-http schemes. \
                     If you believe the URL is legitimate, verify spelling, try web_search for an alternate public endpoint, or ask the operator about SSRF policy."
                ));
            }
        };

        let method_name = method.to_string();
        let is_head = method == reqwest::Method::HEAD;
        let with_body = method == reqwest::Method::POST
            || method == reqwest::Method::PUT
            || method == reqwest::Method::PATCH;
        let mut request = client.request(method, url);
        if !request_headers.is_empty() {
            request = request.headers(request_headers);
        }
        if with_body {
            match args.get("body") {
                None | Some(serde_json::Value::Null) => {}
                Some(serde_json::Value::String(s)) => {
                    request = request.body(s.clone());
                }
                _ => {
                    return ToolResult::err(
                        "http_fetch: 'body' must be a string, or omitted. \
                         What to do next: pass raw text (JSON you stringify yourself) as a string, e.g. \"body\": \"{}\" or omit 'body'."
                            .to_string(),
                    );
                }
            }
        }

        match request.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                if is_head {
                    return ToolResult::ok(
                        serde_json::json!({ "status": status, "body": "" }).to_string(),
                    );
                }
                match read_response_body_limited(resp, HTTP_FETCH_MAX_BODY_BYTES).await {
                    Ok(body) => {
                        let truncated = if body.len() > 4096 {
                            let end = body
                                .char_indices()
                                .map(|(i, _)| i)
                                .take_while(|&i| i <= 4096)
                                .last()
                                .unwrap_or(0);
                            format!(
                                "{}... [truncated, {} bytes total]",
                                &body[..end],
                                body.len()
                            )
                        } else {
                            body
                        };
                        ToolResult::ok(
                            serde_json::json!({ "status": status, "body": truncated })
                                .to_string(),
                        )
                    }
                    Err(e) => ToolResult::err(format!(
                        "http_fetch: failed while reading the response body after HTTP {status} from '{url}': {e}. \
                         What to do next: retry once for transient transport errors; if the payload is HTML or huge, use web_fetch with extract_mode \"text\" or \"raw\"; if the server streams indefinitely, add Range headers only if the API supports them, or use a smaller endpoint.",
                    )),
                }
            }
            Err(e) => ToolResult::err(format!(
                "http_fetch: could not complete HTTP {method_name} to '{url}' before a response arrived: {e}. \
                 What to do next: verify DNS resolves, TLS certificates are valid for the host, the URL is reachable from this environment, and outbound HTTPS is allowed. \
                 If the site blocks bots, try a documented API URL or ask the user for a mirror."
            )),
        }
    }
}
// --- Web Search Tool ---

// --- Pluggable Search Engine Architecture ---

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Trait for pluggable search engine backends.
/// Implement this trait to add a new search engine.
#[async_trait]
pub trait SearchEngine: Send + Sync {
    fn id(&self) -> &str;
    fn display_name(&self) -> &str;
    fn requires_api_key(&self) -> bool;
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>, String>;
}

/// Tavily search engine backend.
pub struct TavilyEngine {
    client: reqwest::Client,
    api_key: String,
}

impl TavilyEngine {
    pub fn new(api_key: String) -> Self {
        Self {
            client: shared_api_search_client(),
            api_key,
        }
    }
}

#[async_trait]
impl SearchEngine for TavilyEngine {
    fn id(&self) -> &str {
        "tavily"
    }
    fn display_name(&self) -> &str {
        "Tavily"
    }
    fn requires_api_key(&self) -> bool {
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
        let body = serde_json::json!({
            "query": query,
            "max_results": max_results,
            "search_depth": "basic",
            "include_answer": true,
        });

        let resp = self
            .client
            .post("https://api.tavily.com/search")
            .json(&body)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| {
                format!(
                    "Tavily HTTP request failed (network/TLS/DNS): {e}. \
                     Verify the Tavily API key and outbound HTTPS access, then retry with a shorter query."
                )
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!(
                "Tavily returned HTTP {status}. Body: {text}. \
                 If 401/403, fix the API key; if 429, reduce max_results or wait; if 5xx, retry later."
            ));
        }

        let json: serde_json::Value = resp.json().await.map_err(|e| {
            format!(
                "Tavily response was not valid JSON: {e}. \
                 The service may be degraded—retry or switch web_search backend in configuration."
            )
        })?;
        let mut results = Vec::new();

        if let Some(answer) = json.get("answer").and_then(|v| v.as_str()) {
            if !answer.is_empty() {
                results.push(SearchResult {
                    title: "AI Answer".to_string(),
                    url: String::new(),
                    snippet: answer.to_string(),
                });
            }
        }

        if let Some(arr) = json.get("results").and_then(|v| v.as_array()) {
            for item in arr {
                results.push(SearchResult {
                    title: item
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    url: item
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    snippet: item
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                });
            }
        }

        Ok(results)
    }
}

/// SearXNG search engine backend.
pub struct SearxngEngine {
    base_url: String,
}

impl SearxngEngine {
    pub fn new(base_url: String) -> Self {
        Self { base_url }
    }
}

#[async_trait]
impl SearchEngine for SearxngEngine {
    fn id(&self) -> &str {
        "searxng"
    }
    fn display_name(&self) -> &str {
        "SearXNG"
    }
    fn requires_api_key(&self) -> bool {
        false
    }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
        let search_url = format!("{}/search", self.base_url.trim_end_matches('/'));
        let client = match ssrf_pinned_client_for_url(&search_url, |b| {
            b.timeout(std::time::Duration::from_secs(15)).user_agent("XiaoLin/0.1.0")
        }) {
            Ok(c) => c,
            Err(e) => {
                return Err(format!(
                    "searxng search URL rejected before request: {e}. \
                     Configure a public http(s) SearXNG base URL that resolves outside private networks."
                ));
            }
        };
        let resp = client
            .get(&search_url)
            .query(&[("q", query), ("format", "json"), ("categories", "general")])
            .send()
            .await
            .map_err(|e| {
                format!(
                    "SearXNG request failed: {e}. \
                     Confirm base_url is reachable and returns JSON search results."
                )
            })?;

        if !resp.status().is_success() {
            return Err(format!(
                "SearXNG HTTP {}. \
                 Check instance health, authentication if required, and that /search?format=json is enabled.",
                resp.status()
            ));
        }

        let json: serde_json::Value = resp.json().await.map_err(|e| {
            format!(
                "SearXNG JSON parse failed: {e}. \
                 Ensure format=json and the instance version matches expectations."
            )
        })?;
        let mut results = Vec::new();

        if let Some(arr) = json.get("results").and_then(|v| v.as_array()) {
            for item in arr.iter().take(max_results) {
                results.push(SearchResult {
                    title: item
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    url: item
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    snippet: item
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                });
            }
        }

        Ok(results)
    }
}

// --- Built-in HTML Scraper Engines ---

const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36";

fn build_scraper_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            build_reqwest_client("scraper", |b| {
                b.timeout(std::time::Duration::from_secs(10))
                    .user_agent(BROWSER_UA)
                    .redirect(ssrf_safe_redirect_policy())
            })
        })
        .clone()
}

/// Google web search scraper.
pub struct GoogleEngine {
    client: reqwest::Client,
}

impl Default for GoogleEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl GoogleEngine {
    pub fn new() -> Self {
        Self {
            client: build_scraper_client(),
        }
    }
}

#[async_trait]
impl SearchEngine for GoogleEngine {
    fn id(&self) -> &str {
        "google"
    }
    fn display_name(&self) -> &str {
        "Google"
    }
    fn requires_api_key(&self) -> bool {
        false
    }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
        let resp = self
            .client
            .get("https://www.google.com/search")
            .query(&[
                ("q", query),
                ("num", &max_results.to_string()),
                ("hl", "en"),
            ])
            .send()
            .await
            .map_err(|e| format!("Google request failed: {e}"))?;
        let html = read_response_body_limited(resp, HTTP_FETCH_MAX_BODY_BYTES)
            .await
            .map_err(|e| format!("Google body read failed: {e}"))?;
        let mut results = Vec::new();

        for chunk in html.split("<div class=\"g\"").skip(1) {
            if results.len() >= max_results {
                break;
            }
            let url = chunk
                .split("href=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or("")
                .to_string();
            if url.is_empty() || url.starts_with('#') || url.starts_with('/') {
                continue;
            }

            let title = chunk
                .split("<h3")
                .nth(1)
                .and_then(|s| s.split('>').nth(1))
                .and_then(|s| s.split("</").next())
                .map(strip_html_tags)
                .unwrap_or_default();

            let snippet = chunk
                .split("data-sncf=\"")
                .nth(1)
                .or_else(|| chunk.split("<span class=\"").nth(2))
                .and_then(|s| s.split('>').nth(1).or_else(|| s.split('>').next()))
                .and_then(|s| s.split("</").next())
                .map(strip_html_tags)
                .unwrap_or_default();

            if !title.is_empty() || !url.is_empty() {
                results.push(SearchResult {
                    title,
                    url,
                    snippet,
                });
            }
        }

        // Fallback: try <a href="..."> pattern if class="g" didn't match
        if results.is_empty() {
            for chunk in html.split("<a href=\"/url?q=").skip(1) {
                if results.len() >= max_results {
                    break;
                }
                let url = chunk
                    .split('&')
                    .next()
                    .or_else(|| chunk.split('"').next())
                    .unwrap_or("")
                    .to_string();
                if url.is_empty() || url.contains("google.com") {
                    continue;
                }

                let title = chunk
                    .split('>')
                    .nth(1)
                    .and_then(|s| s.split('<').next())
                    .map(strip_html_tags)
                    .unwrap_or_default();

                if !title.is_empty() {
                    results.push(SearchResult {
                        title,
                        url,
                        snippet: String::new(),
                    });
                }
            }
        }

        Ok(results)
    }
}

/// Baidu (百度) web search scraper.
pub struct BaiduEngine {
    client: reqwest::Client,
}

impl Default for BaiduEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl BaiduEngine {
    pub fn new() -> Self {
        Self {
            client: build_scraper_client(),
        }
    }
}

#[async_trait]
impl SearchEngine for BaiduEngine {
    fn id(&self) -> &str {
        "baidu"
    }
    fn display_name(&self) -> &str {
        "百度"
    }
    fn requires_api_key(&self) -> bool {
        false
    }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
        let resp = self
            .client
            .get("https://www.baidu.com/s")
            .query(&[("wd", query), ("rn", &max_results.to_string())])
            .send()
            .await
            .map_err(|e| format!("Baidu request failed: {e}"))?;
        let html = read_response_body_limited(resp, HTTP_FETCH_MAX_BODY_BYTES)
            .await
            .map_err(|e| format!("Baidu body read failed: {e}"))?;
        let mut results = Vec::new();

        for chunk in html.split("class=\"result c-container").skip(1) {
            if results.len() >= max_results {
                break;
            }

            let url = chunk
                .split("href=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or("")
                .to_string();

            let title = chunk
                .split("class=\"t\"")
                .nth(1)
                .or_else(|| chunk.split("<h3").nth(1))
                .and_then(|s| {
                    let after_tag = s.split('>').nth(1)?;
                    // May have an <a> inside
                    if after_tag.starts_with("<a") {
                        after_tag.split('>').nth(1)?.split("</").next()
                    } else {
                        after_tag.split("</").next()
                    }
                })
                .map(strip_html_tags)
                .unwrap_or_default();

            let snippet = chunk
                .split("class=\"c-abstract\"")
                .nth(1)
                .or_else(|| chunk.split("class=\"content-right_").nth(1))
                .and_then(|s| s.split('>').nth(1))
                .and_then(|s| s.split("</div").next())
                .map(strip_html_tags)
                .unwrap_or_default();

            if !title.is_empty() || !url.is_empty() {
                results.push(SearchResult {
                    title,
                    url,
                    snippet,
                });
            }
        }

        // Fallback: simpler h3 > a pattern
        if results.is_empty() {
            for chunk in html.split("<h3").skip(1) {
                if results.len() >= max_results {
                    break;
                }
                let url = chunk
                    .split("href=\"")
                    .nth(1)
                    .and_then(|s| s.split('"').next())
                    .unwrap_or("")
                    .to_string();
                let title = chunk
                    .split('>')
                    .nth(2)
                    .and_then(|s| s.split('<').next())
                    .map(strip_html_tags)
                    .unwrap_or_default();
                if !title.is_empty() && !url.is_empty() {
                    results.push(SearchResult {
                        title,
                        url,
                        snippet: String::new(),
                    });
                }
            }
        }

        Ok(results)
    }
}

/// Bing web search scraper.
pub struct BingEngine {
    client: reqwest::Client,
}

impl Default for BingEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl BingEngine {
    pub fn new() -> Self {
        Self {
            client: build_scraper_client(),
        }
    }
}

#[async_trait]
impl SearchEngine for BingEngine {
    fn id(&self) -> &str {
        "bing"
    }
    fn display_name(&self) -> &str {
        "Bing"
    }
    fn requires_api_key(&self) -> bool {
        false
    }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
        let resp = self
            .client
            .get("https://www.bing.com/search")
            .query(&[("q", query), ("count", &max_results.to_string())])
            .send()
            .await
            .map_err(|e| format!("Bing request failed: {e}"))?;
        let html = read_response_body_limited(resp, HTTP_FETCH_MAX_BODY_BYTES)
            .await
            .map_err(|e| format!("Bing body read failed: {e}"))?;
        let mut results = Vec::new();

        for chunk in html.split("<li class=\"b_algo\"").skip(1) {
            if results.len() >= max_results {
                break;
            }

            let url = chunk
                .split("href=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or("")
                .to_string();

            let title = chunk
                .split("<h2")
                .nth(1)
                .and_then(|s| s.split('>').nth(1).or_else(|| s.split('>').nth(2)))
                .and_then(|s| s.split("</").next())
                .map(strip_html_tags)
                .unwrap_or_default();

            let snippet = chunk
                .split("<p")
                .nth(1)
                .and_then(|s| s.split('>').nth(1))
                .and_then(|s| s.split("</p").next())
                .map(strip_html_tags)
                .unwrap_or_default();

            if !title.is_empty() || !url.is_empty() {
                results.push(SearchResult {
                    title,
                    url,
                    snippet,
                });
            }
        }

        Ok(results)
    }
}

/// Sogou (搜狗) web search scraper.
pub struct SogouEngine {
    client: reqwest::Client,
}

impl Default for SogouEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SogouEngine {
    pub fn new() -> Self {
        Self {
            client: build_scraper_client(),
        }
    }
}

#[async_trait]
impl SearchEngine for SogouEngine {
    fn id(&self) -> &str {
        "sogou"
    }
    fn display_name(&self) -> &str {
        "搜狗"
    }
    fn requires_api_key(&self) -> bool {
        false
    }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
        let resp = self
            .client
            .get("https://www.sogou.com/web")
            .query(&[("query", query)])
            .send()
            .await
            .map_err(|e| format!("Sogou request failed: {e}"))?;
        let html = resp
            .text()
            .await
            .map_err(|e| format!("Sogou body read failed: {e}"))?;
        let mut results = Vec::new();

        for chunk in html
            .split("class=\"vrwrap\"")
            .chain(html.split("class=\"rb\""))
            .skip(1)
        {
            if results.len() >= max_results {
                break;
            }

            let url = chunk
                .split("href=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or("")
                .to_string();

            let title = chunk
                .split("<h3")
                .nth(1)
                .and_then(|s| {
                    let after = s.split('>').nth(1)?;
                    if after.starts_with("<a") {
                        after.split('>').nth(1)?.split("</").next()
                    } else {
                        after.split("</").next()
                    }
                })
                .map(strip_html_tags)
                .unwrap_or_default();

            let snippet = chunk
                .split("class=\"space-txt\"")
                .nth(1)
                .or_else(|| chunk.split("class=\"star-wiki\"").nth(1))
                .and_then(|s| s.split('>').nth(1))
                .and_then(|s| s.split("</").next())
                .map(strip_html_tags)
                .unwrap_or_default();

            if !title.is_empty() || !url.is_empty() {
                results.push(SearchResult {
                    title,
                    url,
                    snippet,
                });
            }
        }

        Ok(results)
    }
}

/// 360 Search (好搜) web search scraper.
pub struct Search360Engine {
    client: reqwest::Client,
}

impl Default for Search360Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Search360Engine {
    pub fn new() -> Self {
        Self {
            client: build_scraper_client(),
        }
    }
}

#[async_trait]
impl SearchEngine for Search360Engine {
    fn id(&self) -> &str {
        "360"
    }
    fn display_name(&self) -> &str {
        "360搜索"
    }
    fn requires_api_key(&self) -> bool {
        false
    }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
        let resp = self
            .client
            .get("https://www.so.com/s")
            .query(&[("q", query)])
            .send()
            .await
            .map_err(|e| format!("360 Search request failed: {e}"))?;
        let html = resp
            .text()
            .await
            .map_err(|e| format!("360 Search body read failed: {e}"))?;
        let mut results = Vec::new();

        for chunk in html
            .split("class=\"res-list\"")
            .chain(html.split("class=\"result\""))
            .skip(1)
        {
            if results.len() >= max_results {
                break;
            }

            let url = chunk
                .split("href=\"")
                .nth(1)
                .and_then(|s| s.split('"').next())
                .unwrap_or("")
                .to_string();

            let title = chunk
                .split("<h3")
                .nth(1)
                .and_then(|s| {
                    let after = s.split('>').nth(1)?;
                    if after.starts_with("<a") {
                        after.split('>').nth(1)?.split("</").next()
                    } else {
                        after.split("</").next()
                    }
                })
                .map(strip_html_tags)
                .unwrap_or_default();

            let snippet = chunk
                .split("class=\"res-desc\"")
                .nth(1)
                .or_else(|| chunk.split("class=\"res-rich\"").nth(1))
                .and_then(|s| s.split('>').nth(1))
                .and_then(|s| s.split("</").next())
                .map(strip_html_tags)
                .unwrap_or_default();

            if !title.is_empty() || !url.is_empty() {
                results.push(SearchResult {
                    title,
                    url,
                    snippet,
                });
            }
        }

        Ok(results)
    }
}

// --- Built-in Meta Engine (aggregates multiple scrapers in parallel) ---

/// Resolve an engine ID string to a concrete SearchEngine instance.
pub fn engine_by_id(id: &str) -> Option<Arc<dyn SearchEngine>> {
    match id {
        "google" => Some(Arc::new(GoogleEngine::new())),
        "baidu" => Some(Arc::new(BaiduEngine::new())),
        "bing" => Some(Arc::new(BingEngine::new())),
        "sogou" => Some(Arc::new(SogouEngine::new())),
        "360" => Some(Arc::new(Search360Engine::new())),
        _ => None,
    }
}

/// All available built-in engine IDs.
pub const BUILTIN_ENGINE_IDS: &[&str] = &["google", "baidu", "bing", "sogou", "360"];

/// Meta search engine that queries multiple built-in scrapers in parallel and
/// merges their results, deduplicating by URL.
pub struct BuiltinMetaEngine {
    engines: Vec<Arc<dyn SearchEngine>>,
}

impl BuiltinMetaEngine {
    pub fn new(engine_ids: &[String]) -> Self {
        let engines: Vec<Arc<dyn SearchEngine>> = engine_ids
            .iter()
            .filter_map(|id| engine_by_id(id))
            .collect();
        Self { engines }
    }

    pub fn all() -> Self {
        Self::new(
            &BUILTIN_ENGINE_IDS
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        )
    }
}

#[async_trait]
impl SearchEngine for BuiltinMetaEngine {
    fn id(&self) -> &str {
        "builtin"
    }
    fn display_name(&self) -> &str {
        "Built-in Meta Search"
    }
    fn requires_api_key(&self) -> bool {
        false
    }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
        if self.engines.is_empty() {
            return Err("No built-in search engines are enabled. Open Settings → 联网搜索 and enable at least one engine.".to_string());
        }

        let futs: Vec<_> = self.engines.iter().map(|engine| {
            let engine = engine.clone();
            let query = query.to_string();
            async move {
                let id = engine.id().to_string();
                match engine.search(&query, max_results).await {
                    Ok(results) => {
                        tracing::debug!(engine = %id, count = results.len(), "built-in engine returned results");
                        results
                    }
                    Err(e) => {
                        tracing::warn!(engine = %id, error = %e, "built-in engine failed, skipping");
                        Vec::new()
                    }
                }
            }
        }).collect();

        let all_results = futures::future::join_all(futs).await;

        let mut seen_urls = std::collections::HashSet::new();
        let mut merged = Vec::new();
        for batch in all_results {
            for result in batch {
                let key = result.url.trim().to_lowercase();
                if key.is_empty() || seen_urls.contains(&key) {
                    continue;
                }
                seen_urls.insert(key);
                merged.push(result);
                if merged.len() >= max_results {
                    break;
                }
            }
            if merged.len() >= max_results {
                break;
            }
        }

        if merged.is_empty() {
            return Err("All built-in search engines failed to return results. Check your network connection.".to_string());
        }

        Ok(merged)
    }
}

/// Placeholder engine returned when no search backend is configured.
struct UnconfiguredSearchEngine;

#[async_trait]
impl SearchEngine for UnconfiguredSearchEngine {
    fn id(&self) -> &str {
        "unconfigured"
    }
    fn display_name(&self) -> &str {
        "Unconfigured"
    }
    fn requires_api_key(&self) -> bool {
        false
    }
    async fn search(&self, _query: &str, _max_results: usize) -> Result<Vec<SearchResult>, String> {
        Err(
            "web_search is not configured. Open Settings → 联网搜索 and set up Tavily, SearXNG, or enable built-in search engines.".to_string()
        )
    }
}

// --- Backward-compatible WebSearchBackend enum (wraps the trait) ---

#[derive(Clone)]
pub enum WebSearchBackend {
    Tavily { api_key: String },
    SearXNG { base_url: String },
    Builtin { engines: Vec<String> },
}

impl WebSearchBackend {
    pub fn into_engine(self) -> Arc<dyn SearchEngine> {
        match self {
            Self::Tavily { api_key } => Arc::new(TavilyEngine::new(api_key)),
            Self::SearXNG { base_url } => Arc::new(SearxngEngine::new(base_url)),
            Self::Builtin { engines } => Arc::new(BuiltinMetaEngine::new(&engines)),
        }
    }
}

/// Web search tool that queries search APIs and returns structured results.
/// Uses the pluggable `SearchEngine` trait — swap the engine to switch providers.
pub struct WebSearchTool {
    engine: Arc<dyn SearchEngine>,
}

impl WebSearchTool {
    pub fn new(backend: WebSearchBackend) -> Self {
        Self {
            engine: backend.into_engine(),
        }
    }

    pub fn from_engine(engine: Arc<dyn SearchEngine>) -> Self {
        Self { engine }
    }

    pub fn unconfigured() -> Self {
        Self {
            engine: Arc::new(UnconfiguredSearchEngine),
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Search
    }
    fn supports_parallel(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the public web. Returns ranked hits {title, url, snippet}. \
         max_results defaults to 5, caps at 10. Follow up with web_fetch for full content."
    }

    fn prompt(&self) -> String {
        "Search the web for real-time information.\n\n\
## When to Use\n\
- Up-to-date information not in your training data\n\
- Current library/framework documentation and best practices\n\
- Technology news, release notes, changelogs\n\
- Error messages you don't recognize\n\
- Verifying current facts (API endpoints, pricing, availability)\n\n\
## Query Construction\n\
- Be specific: include version numbers, dates, exact error text\n\
- Use the current year in queries about recent topics\n\
- Include language/framework name for technical queries\n\
- Use quotes for exact phrases: '\"exact error message\"'\n\n\
## Anti-Patterns\n\
- Don't search for things you already know well\n\
- Don't search when the answer is in the local codebase\n\
- Don't use web_search for code examples — search_in_files is better for local patterns"
            .to_string()
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "query".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Search query string."
            }),
        );
        props.insert(
            "max_results".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Max results (default 5, max 10)."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["query".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "web_search arguments are not valid JSON: {e}. \
                 Pass {{\"query\": \"your keywords\", \"max_results\": 5}} with double-quoted keys; max_results is optional."
            )),
        };

        let query = match args.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.trim().is_empty() => q,
            _ => {
                return ToolResult::err(
                    "web_search is missing a non-empty string field 'query'. \
                 Example: {\"query\": \"Rust tokio select! example\"}. \
                 Add disambiguating terms (year, vendor, version) instead of a blank string."
                        .to_string(),
                )
            }
        };

        let max_results = args
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5)
            .min(10) as usize;

        let results = self.engine.search(query, max_results).await;

        match results {
            Ok(hits) => {
                let items: Vec<serde_json::Value> = hits
                    .into_iter()
                    .map(|r| {
                        serde_json::json!({
                            "title": r.title,
                            "url": r.url,
                            "snippet": r.snippet,
                        })
                    })
                    .collect();
                ToolResult::ok(
                    serde_json::json!({
                        "query": query,
                        "results": items,
                        "count": items.len(),
                    })
                    .to_string(),
                )
            }
            Err(e) => ToolResult::err(e),
        }
    }
}

// --- Web Fetch Tool ---

/// Fetch a web page and extract its text content (strips HTML tags).
pub struct WebFetchTool {
    max_content_bytes: usize,
}

impl WebFetchTool {
    pub fn new(max_content_bytes: usize) -> Self {
        Self { max_content_bytes }
    }

    pub fn with_defaults() -> Self {
        Self::new(32_768)
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Fetch
    }
    fn supports_parallel(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Read web page content as clean text or markdown. Designed for documentation, articles, \
         READMEs, and any HTML page — extracts human-readable content from HTML. GET-only; \
         extract_mode: 'text' (default, strips tags), 'markdown' (preserves structure), 'raw' (HTML). \
         Content up to ~32KB. No JS execution (won't work for client-rendered SPAs). \
         DO NOT use for API calls — use http_fetch (which supports POST/PUT/headers/auth)."
    }

    fn prompt(&self) -> String {
        "Fetch and extract content from a web URL.\n\n\
## When to Use\n\
- Reading documentation pages found via web_search\n\
- Fetching API responses for inspection\n\
- Reading READMEs from GitHub repos\n\
- Getting current content from a known URL\n\n\
## Parameters\n\
- url: Must be a full http(s) URL\n\
- extract_mode: 'text' (clean text, default), 'markdown' (preserves structure), 'raw' (HTML)\n\n\
## Limitations\n\
- No JavaScript execution — won't work for SPAs that render client-side\n\
- Content truncated at ~32KB — use 'text' mode for large pages\n\
- No authentication — private/gated content will fail\n\
- localhost and private IPs are blocked (SSRF prevention)\n\n\
## Anti-Patterns\n\
- Don't fetch URLs you already have content for\n\
- Don't use for downloading files — redirect output to disk via shell\n\
- Don't fetch multiple pages when web_search results already have enough info"
            .to_string()
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "url".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Absolute http(s) URL to fetch."
            }),
        );
        props.insert(
            "extract_mode".to_string(),
            serde_json::json!({
                "type": "string",
                "enum": ["text", "raw", "markdown"],
                "description": "Content extraction mode (default 'text')."
            }),
        );
        ToolParameterSchema {
            schema_type: "object".to_string(),
            properties: props,
            required: vec!["url".to_string()],
        }
    }

    async fn execute(&self, arguments: &str) -> ToolResult {
        let args: serde_json::Value = match serde_json::from_str(arguments) {
            Ok(v) => v,
            Err(e) => return ToolResult::err(format!(
                "web_fetch arguments are not valid JSON: {e}. \
                 Pass {{\"url\": \"https://example.com/doc\", \"extract_mode\": \"text\"}}; extract_mode is optional (defaults to text)."
            )),
        };

        let url = match args.get("url").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => return ToolResult::err(
                "web_fetch is missing required string field 'url'. \
                 Example: {\"url\": \"https://doc.rust-lang.org/book/\", \"extract_mode\": \"text\"}."
                    .to_string(),
            ),
        };

        let client = match ssrf_pinned_client_for_url(url, |b| {
            b.timeout(std::time::Duration::from_secs(20))
                .user_agent("XiaoLin/0.1.0 (Bot)")
                .redirect(ssrf_safe_redirect_policy())
        }) {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::err(format!(
                    "web_fetch rejected the URL before download: {e}. \
                     Use a public http(s) URL that resolves on the public internet; avoid localhost, RFC1918, link-local, and file://. \
                     If the document lives inside a VPN, ask the user to paste accessible text or expose an approved public mirror."
                ));
            }
        };

        let mode = args
            .get("extract_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        let resp = match client.get(url).send().await {
            Ok(r) => r,
            Err(e) => return ToolResult::err(format!(
                "web_fetch could not complete HTTP GET to '{url}': {e}. \
                 Check spelling/scheme, DNS resolution, TLS trust, and outbound firewall rules; if the site blocks bots, try an official API endpoint or a different mirror from web_search."
            )),
        };

        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let final_url = resp.url().to_string();

        let body = match read_response_body_limited(resp, self.max_content_bytes).await {
            Ok(t) => t,
            Err(e) => {
                return ToolResult::err(format!(
                "web_fetch received HTTP {status} for '{final_url}' but failed reading body: {e}. \
                 Retry, or switch extract_mode if the payload type is unexpected.",
                status = status,
                final_url = final_url,
            ))
            }
        };

        let extracted = match mode {
            "raw" => {
                if body.contains("[truncated,") {
                    body
                } else if body.len() > self.max_content_bytes {
                    truncate_text(&body, self.max_content_bytes)
                } else {
                    body
                }
            }
            "markdown" => {
                let text = html_to_markdown(&body);
                truncate_text(&text, self.max_content_bytes)
            }
            _ => {
                let text = strip_html_tags(&body);
                truncate_text(&text, self.max_content_bytes)
            }
        };

        ToolResult::ok(
            serde_json::json!({
                "url": final_url,
                "status": status,
                "content_type": content_type,
                "content": extracted,
                "content_length": extracted.len(),
            })
            .to_string(),
        )
    }
}

pub fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut last_was_whitespace = false;

    let chars: Vec<char> = html.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        if !in_tag && i + 7 < chars.len() {
            if chars_starts_with_ascii_ignore_case(&chars, i, "<script") {
                in_script = true;
                in_tag = true;
                i += 1;
                continue;
            }
            if chars_starts_with_ascii_ignore_case(&chars, i, "<style ")
                || (i + 6 < chars.len()
                    && chars_starts_with_ascii_ignore_case(&chars, i, "<style"))
            {
                in_style = true;
                in_tag = true;
                i += 1;
                continue;
            }
        }

        if in_script && i + 9 <= chars.len() {
            if chars_starts_with_ascii_ignore_case(&chars, i, "</script>") {
                in_script = false;
                i += 9;
                continue;
            }
            i += 1;
            continue;
        }

        if in_style && i + 8 <= chars.len() {
            if chars_starts_with_ascii_ignore_case(&chars, i, "</style>") {
                in_style = false;
                i += 8;
                continue;
            }
            i += 1;
            continue;
        }

        let ch = chars[i];
        if ch == '<' {
            let tag_start = i;
            while i < chars.len() && chars[i] != '>' {
                i += 1;
            }
            if i < chars.len() {
                i += 1;
            }
            if tag_is_block_break(&chars, tag_start) {
                if !result.ends_with('\n') {
                    result.push('\n');
                }
                last_was_whitespace = true;
            }
            in_tag = false;
            continue;
        }

        if in_tag {
            if ch == '>' {
                in_tag = false;
            }
            i += 1;
            continue;
        }

        if ch == '&' {
            // Scan ahead in char slice for ';' to decode HTML entities
            let mut semi = i + 1;
            while semi < chars.len() && semi - i <= 8 && chars[semi] != ';' {
                semi += 1;
            }
            if semi < chars.len() && chars[semi] == ';' && semi - i <= 8 {
                let entity: String = chars[i..=semi].iter().collect();
                let decoded = match entity.as_str() {
                    "&amp;" => "&",
                    "&lt;" => "<",
                    "&gt;" => ">",
                    "&quot;" => "\"",
                    "&apos;" | "&#39;" => "'",
                    "&nbsp;" => " ",
                    _ => &entity,
                };
                result.push_str(decoded);
                last_was_whitespace = decoded.ends_with(' ');
                i = semi + 1;
                continue;
            }
        }

        if ch.is_whitespace() {
            if !last_was_whitespace {
                result.push(' ');
                last_was_whitespace = true;
            }
        } else {
            result.push(ch);
            last_was_whitespace = false;
        }
        i += 1;
    }

    let cleaned: Vec<&str> = result
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    cleaned.join("\n")
}

fn chars_starts_with_ascii_ignore_case(chars: &[char], pos: usize, prefix: &str) -> bool {
    prefix.chars().enumerate().all(|(j, pc)| {
        chars
            .get(pos + j)
            .is_some_and(|c| c.to_ascii_lowercase() == pc.to_ascii_lowercase())
    })
}

fn tag_is_block_break(chars: &[char], tag_start: usize) -> bool {
    const BLOCK_PREFIXES: &[&str] = &[
        "<br", "<p", "</p", "<div", "</div", "<h", "</h", "<li", "<tr",
    ];
    BLOCK_PREFIXES
        .iter()
        .any(|prefix| chars_starts_with_ascii_ignore_case(chars, tag_start, prefix))
}

fn html_to_markdown(html: &str) -> String {
    strip_html_tags(html)
}

pub fn truncate_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let end = text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= max_bytes)
        .last()
        .unwrap_or(0);
    format!(
        "{}... [truncated, {} bytes total]",
        &text[..end],
        text.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tavily_engine_metadata() {
        let engine = TavilyEngine::new("test-key".to_string());
        assert_eq!(engine.id(), "tavily");
        assert_eq!(engine.display_name(), "Tavily");
        assert!(engine.requires_api_key());
    }

    #[test]
    fn searxng_engine_metadata() {
        let engine = SearxngEngine::new("http://localhost:8888".to_string());
        assert_eq!(engine.id(), "searxng");
        assert_eq!(engine.display_name(), "SearXNG");
        assert!(!engine.requires_api_key());
    }

    #[test]
    fn backend_enum_to_engine_tavily() {
        let engine = WebSearchBackend::Tavily {
            api_key: "key".into(),
        }
        .into_engine();
        assert_eq!(engine.id(), "tavily");
    }

    #[test]
    fn backend_enum_to_engine_searxng() {
        let engine = WebSearchBackend::SearXNG {
            base_url: "http://localhost".into(),
        }
        .into_engine();
        assert_eq!(engine.id(), "searxng");
    }

    #[test]
    fn web_search_tool_from_engine() {
        let engine: Arc<dyn SearchEngine> = Arc::new(MockSearchEngine);
        let tool = WebSearchTool::from_engine(engine);
        assert_eq!(tool.name(), "web_search");
    }

    #[test]
    fn web_search_tool_metadata() {
        let tool = WebSearchTool::unconfigured();
        assert_eq!(tool.name(), "web_search");
        let schema = tool.parameters_schema();
        assert!(schema.properties.contains_key("query"));
        assert!(schema.properties.contains_key("max_results"));
        assert!(schema.required.contains(&"query".to_string()));
    }

    #[tokio::test]
    async fn web_search_rejects_missing_query() {
        let tool = WebSearchTool::unconfigured();
        let result = tool.execute(r#"{"max_results": 3}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[tokio::test]
    async fn web_search_rejects_empty_query() {
        let tool = WebSearchTool::unconfigured();
        let result = tool.execute(r#"{"query": ""}"#).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn web_search_rejects_bad_json() {
        let tool = WebSearchTool::unconfigured();
        let result = tool.execute("not json").await;
        assert!(!result.success);
        assert!(result.output.contains("not valid JSON"));
    }

    #[tokio::test]
    async fn tavily_search_rejects_bad_key() {
        let engine = TavilyEngine::new("invalid-key".to_string());
        let result = engine.search("test query", 3).await;
        assert!(result.is_err());
    }

    #[test]
    fn web_fetch_tool_metadata() {
        let tool = WebFetchTool::with_defaults();
        assert_eq!(tool.name(), "web_fetch");
        let schema = tool.parameters_schema();
        assert!(schema.properties.contains_key("url"));
        assert!(schema.properties.contains_key("extract_mode"));
    }

    #[tokio::test]
    async fn web_fetch_rejects_missing_url() {
        let tool = WebFetchTool::with_defaults();
        let result = tool.execute(r#"{"extract_mode": "text"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[tokio::test]
    async fn web_fetch_rejects_private_url() {
        let tool = WebFetchTool::with_defaults();
        let result = tool.execute(r#"{"url": "http://127.0.0.1/admin"}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("rejected"));
    }

    #[test]
    fn http_fetch_tool_metadata() {
        let tool = HttpFetchTool::new();
        assert_eq!(tool.name(), "http_fetch");
        let schema = tool.parameters_schema();
        assert!(schema.properties.contains_key("url"));
        assert!(schema.properties.contains_key("method"));
        assert!(schema.properties.contains_key("headers"));
        assert!(schema.properties.contains_key("body"));
    }

    #[tokio::test]
    async fn http_fetch_rejects_private_url() {
        let tool = HttpFetchTool::new();
        let result = tool.execute(r#"{"url": "http://192.168.1.1/"}"#).await;
        assert!(!result.success);
    }

    #[test]
    fn strip_html_basic() {
        let html = "<p>Hello <b>World</b></p>";
        let text = strip_html_tags(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(!text.contains("<p>"));
        assert!(!text.contains("<b>"));
    }

    #[test]
    fn strip_html_script_removal() {
        let html = "<p>Before</p><script>alert('xss')</script><p>After</p>";
        let text = strip_html_tags(html);
        assert!(text.contains("Before"));
        assert!(text.contains("After"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn truncate_text_within_limit() {
        let text = "Hello world";
        assert_eq!(truncate_text(text, 100), text);
    }

    #[test]
    fn truncate_text_exceeds_limit() {
        let text = "Hello world, this is a longer text that should be truncated";
        let result = truncate_text(text, 20);
        assert!(result.contains("truncated"));
        assert!(result.len() > 20);
    }

    /// Custom engine example showing the pluggable architecture.
    struct MockSearchEngine;

    #[async_trait]
    impl SearchEngine for MockSearchEngine {
        fn id(&self) -> &str {
            "mock"
        }
        fn display_name(&self) -> &str {
            "Mock Engine"
        }
        fn requires_api_key(&self) -> bool {
            false
        }
        async fn search(
            &self,
            query: &str,
            max_results: usize,
        ) -> Result<Vec<SearchResult>, String> {
            Ok(vec![SearchResult {
                title: format!("Mock result for: {query}"),
                url: "https://example.com".to_string(),
                snippet: "This is a mock result".to_string(),
            }]
            .into_iter()
            .take(max_results)
            .collect())
        }
    }

    #[tokio::test]
    async fn custom_engine_via_from_engine() {
        let engine: Arc<dyn SearchEngine> = Arc::new(MockSearchEngine);
        let tool = WebSearchTool::from_engine(engine);
        let result = tool.execute(r#"{"query": "test"}"#).await;
        assert!(result.success);
        assert!(result.output.contains("Mock result"));
    }

    // --- Built-in scraper engine tests ---

    #[test]
    fn google_engine_metadata() {
        let engine = GoogleEngine::new();
        assert_eq!(engine.id(), "google");
        assert_eq!(engine.display_name(), "Google");
        assert!(!engine.requires_api_key());
    }

    #[test]
    fn baidu_engine_metadata() {
        let engine = BaiduEngine::new();
        assert_eq!(engine.id(), "baidu");
        assert_eq!(engine.display_name(), "百度");
        assert!(!engine.requires_api_key());
    }

    #[test]
    fn bing_engine_metadata() {
        let engine = BingEngine::new();
        assert_eq!(engine.id(), "bing");
        assert_eq!(engine.display_name(), "Bing");
        assert!(!engine.requires_api_key());
    }

    #[test]
    fn sogou_engine_metadata() {
        let engine = SogouEngine::new();
        assert_eq!(engine.id(), "sogou");
        assert_eq!(engine.display_name(), "搜狗");
        assert!(!engine.requires_api_key());
    }

    #[test]
    fn search360_engine_metadata() {
        let engine = Search360Engine::new();
        assert_eq!(engine.id(), "360");
        assert_eq!(engine.display_name(), "360搜索");
        assert!(!engine.requires_api_key());
    }

    #[test]
    fn engine_by_id_resolves_all() {
        for id in BUILTIN_ENGINE_IDS {
            assert!(
                engine_by_id(id).is_some(),
                "engine_by_id should resolve '{id}'"
            );
        }
        assert!(engine_by_id("nonexistent").is_none());
    }

    #[test]
    fn engine_by_id_returns_correct_ids() {
        for id in BUILTIN_ENGINE_IDS {
            let engine = engine_by_id(id).unwrap();
            assert_eq!(engine.id(), *id);
        }
    }

    #[test]
    fn builtin_engine_ids_has_five() {
        assert_eq!(BUILTIN_ENGINE_IDS.len(), 5);
        assert!(BUILTIN_ENGINE_IDS.contains(&"google"));
        assert!(BUILTIN_ENGINE_IDS.contains(&"baidu"));
        assert!(BUILTIN_ENGINE_IDS.contains(&"bing"));
        assert!(BUILTIN_ENGINE_IDS.contains(&"sogou"));
        assert!(BUILTIN_ENGINE_IDS.contains(&"360"));
    }

    // --- BuiltinMetaEngine tests ---

    #[test]
    fn builtin_meta_engine_metadata() {
        let engine = BuiltinMetaEngine::all();
        assert_eq!(engine.id(), "builtin");
        assert_eq!(engine.display_name(), "Built-in Meta Search");
        assert!(!engine.requires_api_key());
    }

    #[test]
    fn builtin_meta_engine_selects_subset() {
        let engine = BuiltinMetaEngine::new(&["google".to_string(), "bing".to_string()]);
        assert_eq!(engine.engines.len(), 2);
    }

    #[test]
    fn builtin_meta_engine_skips_unknown() {
        let engine = BuiltinMetaEngine::new(&["google".to_string(), "invalid_xyz".to_string()]);
        assert_eq!(engine.engines.len(), 1);
    }

    #[test]
    fn builtin_meta_engine_empty_if_all_unknown() {
        let engine = BuiltinMetaEngine::new(&["fake1".to_string(), "fake2".to_string()]);
        assert_eq!(engine.engines.len(), 0);
    }

    #[tokio::test]
    async fn builtin_meta_engine_errors_when_empty() {
        let engine = BuiltinMetaEngine::new(&[]);
        let result = engine.search("test", 5).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("No built-in search engines are enabled"));
    }

    #[test]
    fn backend_enum_to_engine_builtin() {
        let engine = WebSearchBackend::Builtin {
            engines: vec!["bing".to_string(), "sogou".to_string()],
        }
        .into_engine();
        assert_eq!(engine.id(), "builtin");
    }

    #[test]
    fn backend_enum_builtin_all() {
        let engine = WebSearchBackend::Builtin {
            engines: BUILTIN_ENGINE_IDS.iter().map(|s| s.to_string()).collect(),
        }
        .into_engine();
        assert_eq!(engine.id(), "builtin");
    }

    // --- Meta engine dedup/merge via mock ---

    struct FixedResultEngine {
        engine_id: String,
        results: Vec<SearchResult>,
    }

    #[async_trait]
    impl SearchEngine for FixedResultEngine {
        fn id(&self) -> &str {
            &self.engine_id
        }
        fn display_name(&self) -> &str {
            &self.engine_id
        }
        fn requires_api_key(&self) -> bool {
            false
        }
        async fn search(
            &self,
            _query: &str,
            _max_results: usize,
        ) -> Result<Vec<SearchResult>, String> {
            Ok(self.results.clone())
        }
    }

    struct FailingEngine;

    #[async_trait]
    impl SearchEngine for FailingEngine {
        fn id(&self) -> &str {
            "failing"
        }
        fn display_name(&self) -> &str {
            "Failing"
        }
        fn requires_api_key(&self) -> bool {
            false
        }
        async fn search(
            &self,
            _query: &str,
            _max_results: usize,
        ) -> Result<Vec<SearchResult>, String> {
            Err("simulated failure".to_string())
        }
    }

    #[tokio::test]
    async fn meta_engine_merges_results() {
        let e1: Arc<dyn SearchEngine> = Arc::new(FixedResultEngine {
            engine_id: "e1".to_string(),
            results: vec![
                SearchResult {
                    title: "A".into(),
                    url: "https://a.com".into(),
                    snippet: "aa".into(),
                },
                SearchResult {
                    title: "B".into(),
                    url: "https://b.com".into(),
                    snippet: "bb".into(),
                },
            ],
        });
        let e2: Arc<dyn SearchEngine> = Arc::new(FixedResultEngine {
            engine_id: "e2".to_string(),
            results: vec![SearchResult {
                title: "C".into(),
                url: "https://c.com".into(),
                snippet: "cc".into(),
            }],
        });
        let meta = BuiltinMetaEngine {
            engines: vec![e1, e2],
        };
        let results = meta.search("test", 10).await.unwrap();
        assert_eq!(results.len(), 3);
    }

    #[tokio::test]
    async fn meta_engine_deduplicates_by_url() {
        let e1: Arc<dyn SearchEngine> = Arc::new(FixedResultEngine {
            engine_id: "e1".to_string(),
            results: vec![SearchResult {
                title: "A from e1".into(),
                url: "https://same.com".into(),
                snippet: "s1".into(),
            }],
        });
        let e2: Arc<dyn SearchEngine> = Arc::new(FixedResultEngine {
            engine_id: "e2".to_string(),
            results: vec![
                SearchResult {
                    title: "A from e2".into(),
                    url: "https://same.com".into(),
                    snippet: "s2".into(),
                },
                SearchResult {
                    title: "B".into(),
                    url: "https://b.com".into(),
                    snippet: "bb".into(),
                },
            ],
        });
        let meta = BuiltinMetaEngine {
            engines: vec![e1, e2],
        };
        let results = meta.search("test", 10).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "A from e1"); // first occurrence wins
    }

    #[tokio::test]
    async fn meta_engine_respects_max_results() {
        let e1: Arc<dyn SearchEngine> = Arc::new(FixedResultEngine {
            engine_id: "e1".to_string(),
            results: vec![
                SearchResult {
                    title: "A".into(),
                    url: "https://a.com".into(),
                    snippet: "aa".into(),
                },
                SearchResult {
                    title: "B".into(),
                    url: "https://b.com".into(),
                    snippet: "bb".into(),
                },
                SearchResult {
                    title: "C".into(),
                    url: "https://c.com".into(),
                    snippet: "cc".into(),
                },
            ],
        });
        let meta = BuiltinMetaEngine { engines: vec![e1] };
        let results = meta.search("test", 2).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn meta_engine_tolerates_partial_failure() {
        let ok_engine: Arc<dyn SearchEngine> = Arc::new(FixedResultEngine {
            engine_id: "ok".to_string(),
            results: vec![SearchResult {
                title: "Works".into(),
                url: "https://ok.com".into(),
                snippet: "good".into(),
            }],
        });
        let fail_engine: Arc<dyn SearchEngine> = Arc::new(FailingEngine);
        let meta = BuiltinMetaEngine {
            engines: vec![fail_engine, ok_engine],
        };
        let results = meta.search("test", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Works");
    }

    #[tokio::test]
    async fn meta_engine_errors_when_all_fail() {
        let f1: Arc<dyn SearchEngine> = Arc::new(FailingEngine);
        let f2: Arc<dyn SearchEngine> = Arc::new(FailingEngine);
        let meta = BuiltinMetaEngine {
            engines: vec![f1, f2],
        };
        let result = meta.search("test", 5).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("All built-in search engines failed"));
    }

    #[test]
    fn strip_html_handles_multibyte_entities() {
        let html = "<p>中文&amp;测试</p>";
        let text = strip_html_tags(html);
        assert!(text.contains("中文"));
        assert!(text.contains("&"));
        assert!(text.contains("测试"));
    }

    #[test]
    fn strip_html_handles_unicode_heavy() {
        let html = "2022年12月 — 从Tauri官网的宣传语可以看出";
        let text = strip_html_tags(html);
        assert!(text.contains("2022年12月"));
        assert!(text.contains("Tauri"));
    }
}
