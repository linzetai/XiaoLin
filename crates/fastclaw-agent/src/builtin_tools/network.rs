use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolParameterSchema, ToolResult};
use fastclaw_security::ssrf::{ssrf_check_url, ssrf_safe_redirect_policy};

/// HTTP fetch tool — retrieves content from a URL.
pub struct HttpFetchTool {
    client: reqwest::Client,
}

impl HttpFetchTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .redirect(ssrf_safe_redirect_policy())
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl Tool for HttpFetchTool {
    fn name(&self) -> &str {
        "http_fetch"
    }

    fn description(&self) -> &str {
        "HTTP GET one absolute URL and return JSON {status, body} with raw response text (no HTML cleanup). Ideal for small JSON APIs, health checks, version probes, or any machine-readable endpoint when you already have the URL. \
         For long HTML articles or docs you want to read, use web_fetch with extract_mode \"text\"—it strips tags and allows a larger text budget than http_fetch's ~4KB body truncation. \
         When you only have a question or keywords, use web_search first, pick trustworthy URLs, then http_fetch for compact JSON or web_fetch for prose. \
         SSRF rules block localhost, private RFC1918 ranges, link-local addresses, file://, and unsafe redirects; only vetted public http(s) URLs succeed. \
         Responses truncate near 4096 bytes with a total-size suffix; client timeout is 10s—narrow the endpoint, add query limits, or move to web_fetch if you routinely hit limits. \
         Non-2xx HTTP statuses still return JSON with the status field—treat them as logical failures for automation. \
         Anti-pattern: pulling huge HTML landing pages through http_fetch expecting readable text. \
         Example: {\"url\": \"https://api.github.com/zen\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "url".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Full http(s) URL including scheme and host, e.g. 'https://api.github.com/repos/org/repo'. Relative paths ('/v1/status') are invalid. Must pass SSRF policy (no localhost/private/file). If DNS or TLS fails, fix the hostname or trust chain before retrying."
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
                 Pass {{\"url\": \"https://example.com/path\"}} with a string URL, then retry."
            )),
        };

        let url = match args.get("url").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => return ToolResult::err(
                "http_fetch is missing string field 'url'. \
                 Example: {\"url\": \"https://httpbin.org/get\"}. \
                 Relative paths like '/api' are not accepted—include scheme and host."
                    .to_string(),
            ),
        };

        if let Err(e) = ssrf_check_url(url) {
            return ToolResult::err(format!(
                "http_fetch URL was rejected before the HTTP request: {e}. \
                 Use a public http(s) URL that resolves outside private networks; avoid localhost, RFC1918 ranges, link-local addresses, and non-http schemes. \
                 If you believe the URL is legitimate, verify spelling, try web_search for an alternate public endpoint, or ask the operator about SSRF policy."
            ));
        }

        match self.client.get(url).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                match resp.text().await {
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
                            serde_json::json!({
                                "status": status,
                                "body": truncated,
                            })
                            .to_string(),
                        )
                    }
                    Err(e) => ToolResult::err(format!(
                        "http_fetch got HTTP {status} from '{url}' but failed while reading the response body: {e}. \
                         What to do next: retry once for transient transport errors; if the payload is HTML or huge, use web_fetch with extract_mode \"text\" or \"raw\"; if the server streams indefinitely, add Range headers only if the API supports them, or use a smaller endpoint.",
                        status = status,
                        url = url,
                    )),
                }
            }
            Err(e) => ToolResult::err(format!(
                "http_fetch could not complete HTTP GET to '{url}' before a response arrived: {e}. \
                 Verify DNS resolves, TLS certificates are valid for the host, the URL is reachable from this environment, and outbound HTTPS is allowed. \
                 If the site blocks bots, try a documented API URL or ask the user for a mirror."
            )),
        }
    }
}
// --- Web Search Tool ---

// --- Pluggable Search Engine Architecture ---

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
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("FastClaw/0.1.0")
                .build()
                .unwrap_or_default(),
            api_key,
        }
    }
}

#[async_trait]
impl SearchEngine for TavilyEngine {
    fn id(&self) -> &str { "tavily" }
    fn display_name(&self) -> &str { "Tavily" }
    fn requires_api_key(&self) -> bool { true }

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
                    title: item.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    url: item.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    snippet: item.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                });
            }
        }

        Ok(results)
    }
}

/// SearXNG search engine backend.
pub struct SearxngEngine {
    client: reqwest::Client,
    base_url: String,
}

impl SearxngEngine {
    pub fn new(base_url: String) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("FastClaw/0.1.0")
                .build()
                .unwrap_or_default(),
            base_url,
        }
    }
}

#[async_trait]
impl SearchEngine for SearxngEngine {
    fn id(&self) -> &str { "searxng" }
    fn display_name(&self) -> &str { "SearXNG" }
    fn requires_api_key(&self) -> bool { false }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
        let resp = self
            .client
            .get(format!("{}/search", self.base_url.trim_end_matches('/')))
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
                    title: item.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    url: item.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    snippet: item.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                });
            }
        }

        Ok(results)
    }
}

/// DuckDuckGo HTML scraping fallback engine.
pub struct DuckDuckGoEngine {
    client: reqwest::Client,
}

impl DuckDuckGoEngine {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("FastClaw/0.1.0")
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl SearchEngine for DuckDuckGoEngine {
    fn id(&self) -> &str { "duckduckgo" }
    fn display_name(&self) -> &str { "DuckDuckGo" }
    fn requires_api_key(&self) -> bool { false }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
        let resp = self
            .client
            .get("https://html.duckduckgo.com/html/")
            .query(&[("q", query)])
            .send()
            .await
            .map_err(|e| {
                format!(
                    "DuckDuckGo HTML endpoint request failed: {e}. \
                     Network may block automated access—configure Tavily or SearXNG for more reliable web_search."
                )
            })?;

        let html = resp.text().await.map_err(|e| {
            format!(
                "DuckDuckGo response body could not be read: {e}. \
                 Retry later or switch to another web_search backend."
            )
        })?;
        let mut results = Vec::new();

        for (i, chunk) in html.split("class=\"result__a\"").skip(1).enumerate() {
            if i >= max_results {
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
                .nth(1)
                .and_then(|s| s.split('<').next())
                .unwrap_or("")
                .to_string();

            let snippet = chunk
                .split("class=\"result__snippet\"")
                .nth(1)
                .and_then(|s| s.split('>').nth(1))
                .and_then(|s| s.split("</").next())
                .map(|s| strip_html_tags(s))
                .unwrap_or_default();

            if !title.is_empty() || !url.is_empty() {
                results.push(SearchResult { title, url, snippet });
            }
        }

        Ok(results)
    }
}

// --- Backward-compatible WebSearchBackend enum (wraps the trait) ---

#[derive(Clone)]
pub enum WebSearchBackend {
    Tavily { api_key: String },
    SearXNG { base_url: String },
    DuckDuckGo,
}

impl WebSearchBackend {
    pub fn into_engine(self) -> Arc<dyn SearchEngine> {
        match self {
            Self::Tavily { api_key } => Arc::new(TavilyEngine::new(api_key)),
            Self::SearXNG { base_url } => Arc::new(SearxngEngine::new(base_url)),
            Self::DuckDuckGo => Arc::new(DuckDuckGoEngine::new()),
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
        Self { engine: backend.into_engine() }
    }

    pub fn from_engine(engine: Arc<dyn SearchEngine>) -> Self {
        Self { engine }
    }

    pub fn with_defaults() -> Self {
        Self::new(WebSearchBackend::DuckDuckGo)
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the public web and return ranked hits {title, url, snippet}. Backend is deployment-specific: Tavily (API key), SearXNG (instance URL), or DuckDuckGo HTML scraping as a fragile fallback that may rate-limit or break when markup changes—treat DDG as best-effort. \
         Use web_search when facts may be outdated, you need discoverable URLs, or you must narrow sources before web_fetch/http_fetch. For code and configs in this workspace, use read_file, list_directory, and shell_exec+rg—web_search cannot read local files. \
         Optional max_results defaults to 5 and caps at 10; snippets are short summaries—follow 1–3 primary URLs with web_fetch for authoritative text, tables, or API fields. \
         Anti-pattern: treating snippets as full specifications; anti-pattern: web_search for symbols you could rg locally. \
         Example: {\"query\": \"Rust tokio select! vs join! 2024\", \"max_results\": 5}, then web_fetch the official doc URL you pick."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "query".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Keywords or a short question. Examples: 'async rust axum middleware example', 'OpenAI responses API streaming'. Add year, vendor, or version when topics collide ('postgres 16 replication')."
            }),
        );
        props.insert(
            "max_results".to_string(),
            serde_json::json!({
                "type": "integer",
                "description": "Max hits to return (integer; default 5, hard max 10). Non-integer JSON is ignored—send a bare number like 5, not \"5\". Use 3–5 for focused answers; use up to 10 only when comparing many similar pages."
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
            _ => return ToolResult::err(
                "web_search is missing a non-empty string field 'query'. \
                 Example: {\"query\": \"Rust tokio select! example\"}. \
                 Add disambiguating terms (year, vendor, version) instead of a blank string."
                    .to_string(),
            ),
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
    client: reqwest::Client,
    max_content_bytes: usize,
}

impl WebFetchTool {
    pub fn new(max_content_bytes: usize) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .user_agent("FastClaw/0.1.0 (Bot)")
                .redirect(ssrf_safe_redirect_policy())
                .build()
                .unwrap_or_default(),
            max_content_bytes,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(32_768)
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Download one HTTP(S) document and return JSON {url, status, content_type, content, content_length}. Field extract_mode controls shaping: \"text\" (default) strips HTML to readable plain text; \"raw\" keeps truncated HTML; \"markdown\" is a lightweight text pass—not CommonMark-perfect and not a browser. \
         No JavaScript execution—SPAs that render entirely in JS may look empty; in that case use an API, static mirror, or the browser tool if enabled. \
         Use web_fetch after web_search or when the user supplies a URL and you need readable prose, README pages, or docs. Prefer http_fetch for tiny JSON probes; web_fetch is meant for HTML with a larger extracted-text ceiling than http_fetch's raw ~4KB cap. \
         SSRF policy matches http_fetch. Extracted content truncates at max_content_bytes (commonly 32768) with an explicit suffix—use anchors, section URLs, or site search when clipped. \
         HTTP 4xx/5xx still returns tool-success JSON—inspect status before trusting content (error pages are not facts). \
         Anti-pattern: fetching PDFs, archives, or images expecting text. \
         Example: {\"url\": \"https://doc.rust-lang.org/book/ch03-02-data-types.html\", \"extract_mode\": \"text\"}."
    }

    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        props.insert(
            "url".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Full https URL to fetch, e.g. 'https://docs.rs/serde/latest/serde/'. Must satisfy SSRF rules. Prefer URLs from web_search results or the user; do not guess private hostnames."
            }),
        );
        props.insert("extract_mode".to_string(), serde_json::json!({
            "type": "string",
            "enum": ["text", "raw", "markdown"],
            "description": "text (default): strip tags for human reading; raw: keep HTML (truncated); markdown: light cleanup (not spec-perfect). Omit for text. Unknown values behave like text."
        }));
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

        if let Err(e) = ssrf_check_url(url) {
            return ToolResult::err(format!(
                "web_fetch rejected the URL before download: {e}. \
                 Use a public http(s) URL that resolves on the public internet; avoid localhost, RFC1918, link-local, and file://. \
                 If the document lives inside a VPN, ask the user to paste accessible text or expose an approved public mirror."
            ));
        }

        let mode = args
            .get("extract_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        let resp = match self.client.get(url).send().await {
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

        let body = match resp.text().await {
            Ok(t) => t,
            Err(e) => return ToolResult::err(format!(
                "web_fetch received HTTP {status} for '{final_url}' but failed reading body: {e}. \
                 Retry, or switch extract_mode if the payload type is unexpected.",
                status = status,
                final_url = final_url,
            )),
        };

        let extracted = match mode {
            "raw" => {
                if body.len() > self.max_content_bytes {
                    let end = body
                        .char_indices()
                        .map(|(i, _)| i)
                        .take_while(|&i| i <= self.max_content_bytes)
                        .last()
                        .unwrap_or(0);
                    format!(
                        "{}... [truncated, {} bytes total]",
                        &body[..end],
                        body.len()
                    )
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

pub(crate) fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut last_was_whitespace = false;

    let lower = html.to_lowercase();
    let chars: Vec<char> = html.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        if !in_tag && i + 7 < lower_chars.len() {
            let ahead: String = lower_chars[i..i + 7].iter().collect();
            if ahead == "<script" {
                in_script = true;
                in_tag = true;
                i += 1;
                continue;
            }
            if ahead == "<style "
                || (i + 6 < lower_chars.len() && {
                    let s: String = lower_chars[i..i + 6].iter().collect();
                    s == "<style"
                })
            {
                in_style = true;
                in_tag = true;
                i += 1;
                continue;
            }
        }

        if in_script && i + 9 <= lower_chars.len() {
            let ahead: String = lower_chars[i..i + 9].iter().collect();
            if ahead == "</script>" {
                in_script = false;
                i += 9;
                continue;
            }
            i += 1;
            continue;
        }

        if in_style && i + 8 <= lower_chars.len() {
            let ahead: String = lower_chars[i..i + 8].iter().collect();
            if ahead == "</style>" {
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
            let tag: String = lower_chars[tag_start..i.min(lower_chars.len())]
                .iter()
                .collect();
            if tag.starts_with("<br")
                || tag.starts_with("<p")
                || tag.starts_with("</p")
                || tag.starts_with("<div")
                || tag.starts_with("</div")
                || tag.starts_with("<h")
                || tag.starts_with("</h")
                || tag.starts_with("<li")
                || tag.starts_with("<tr")
            {
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
            let entity_end = html[i..].find(';').map(|p| i + p + 1);
            if let Some(end) = entity_end {
                if end - i <= 8 {
                    let entity = &html[i..end];
                    let decoded = match entity {
                        "&amp;" => "&",
                        "&lt;" => "<",
                        "&gt;" => ">",
                        "&quot;" => "\"",
                        "&apos;" | "&#39;" => "'",
                        "&nbsp;" => " ",
                        _ => entity,
                    };
                    result.push_str(decoded);
                    last_was_whitespace = decoded.ends_with(' ');
                    i = end;
                    continue;
                }
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

fn html_to_markdown(html: &str) -> String {
    let text = strip_html_tags(html);
    text
}

pub(crate) fn truncate_text(text: &str, max_bytes: usize) -> String {
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
        "{}... [truncated, {} chars total]",
        &text[..end],
        text.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duckduckgo_engine_metadata() {
        let engine = DuckDuckGoEngine::new();
        assert_eq!(engine.id(), "duckduckgo");
        assert_eq!(engine.display_name(), "DuckDuckGo");
        assert!(!engine.requires_api_key());
    }

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
    fn backend_enum_to_engine_duckduckgo() {
        let engine = WebSearchBackend::DuckDuckGo.into_engine();
        assert_eq!(engine.id(), "duckduckgo");
    }

    #[test]
    fn backend_enum_to_engine_tavily() {
        let engine = WebSearchBackend::Tavily { api_key: "key".into() }.into_engine();
        assert_eq!(engine.id(), "tavily");
    }

    #[test]
    fn backend_enum_to_engine_searxng() {
        let engine = WebSearchBackend::SearXNG { base_url: "http://localhost".into() }.into_engine();
        assert_eq!(engine.id(), "searxng");
    }

    #[test]
    fn web_search_tool_from_engine() {
        let engine: Arc<dyn SearchEngine> = Arc::new(DuckDuckGoEngine::new());
        let tool = WebSearchTool::from_engine(engine);
        assert_eq!(tool.name(), "web_search");
    }

    #[test]
    fn web_search_tool_metadata() {
        let tool = WebSearchTool::with_defaults();
        assert_eq!(tool.name(), "web_search");
        let schema = tool.parameters_schema();
        assert!(schema.properties.contains_key("query"));
        assert!(schema.properties.contains_key("max_results"));
        assert!(schema.required.contains(&"query".to_string()));
    }

    #[tokio::test]
    async fn web_search_rejects_missing_query() {
        let tool = WebSearchTool::with_defaults();
        let result = tool.execute(r#"{"max_results": 3}"#).await;
        assert!(!result.success);
        assert!(result.output.contains("missing"));
    }

    #[tokio::test]
    async fn web_search_rejects_empty_query() {
        let tool = WebSearchTool::with_defaults();
        let result = tool.execute(r#"{"query": ""}"#).await;
        assert!(!result.success);
    }

    #[tokio::test]
    async fn web_search_rejects_bad_json() {
        let tool = WebSearchTool::with_defaults();
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
        fn id(&self) -> &str { "mock" }
        fn display_name(&self) -> &str { "Mock Engine" }
        fn requires_api_key(&self) -> bool { false }
        async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>, String> {
            Ok(vec![SearchResult {
                title: format!("Mock result for: {query}"),
                url: "https://example.com".to_string(),
                snippet: "This is a mock result".to_string(),
            }].into_iter().take(max_results).collect())
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
}
