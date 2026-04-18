use anyhow::Result;
use async_trait::async_trait;

/// A single embedding vector (list of f32).
pub type EmbeddingVec = Vec<f32>;

/// Trait for embedding providers — converts text to dense vectors.
///
/// Implementations can be local (ONNX-based) or remote (API-based).
/// The trait is async to support remote API calls.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Embed a batch of texts and return one vector per input.
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<EmbeddingVec>>;

    /// Embed a single text.
    async fn embed(&self, text: &str) -> Result<EmbeddingVec> {
        let mut results = self.embed_batch(&[text]).await?;
        results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("embedding returned empty result"))
    }

    /// Number of dimensions in the output vectors.
    fn dimensions(&self) -> usize;

    /// Provider name for logging.
    fn name(&self) -> &str;
}

// ---------- Local embedding (hypembed — pure Rust BERT inference) ----------

#[cfg(feature = "local-embedding")]
pub mod local {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    const HF_BASE: &str = "https://huggingface.co";
    const REQUIRED_FILES: &[&str] = &["config.json", "vocab.txt", "model.safetensors"];

    pub struct LocalEmbeddingProvider {
        embedder: Arc<hypembed::Embedder>,
        dims: usize,
        model_name: String,
    }

    impl LocalEmbeddingProvider {
        /// Create a local embedding provider.
        ///
        /// `model_name` is a HuggingFace model identifier
        /// (e.g. "sentence-transformers/all-MiniLM-L6-v2").
        /// Model files are downloaded to `~/.fastclaw/models/<model>/` on first use.
        pub async fn new(model_name: &str) -> Result<Self> {
            let model_dir = resolve_model_dir(model_name);
            ensure_model_downloaded(model_name, &model_dir).await?;

            tracing::info!(model = model_name, dir = %model_dir.display(), "loading local embedding model");
            let dir = model_dir.clone();
            let embedder = tokio::task::spawn_blocking(move || hypembed::Embedder::load(&dir))
                .await?
                .map_err(|e| anyhow::anyhow!("failed to load model '{}': {}", model_name, e))?;

            let dims = probe_dimensions(&embedder)?;
            tracing::info!(model = model_name, dims, "local embedding model ready");

            Ok(Self {
                embedder: Arc::new(embedder),
                dims,
                model_name: model_name.to_string(),
            })
        }

        pub async fn with_defaults() -> Result<Self> {
            Self::new("sentence-transformers/all-MiniLM-L6-v2").await
        }
    }

    pub(crate) fn resolve_model_dir(model_name: &str) -> PathBuf {
        let safe_name = model_name.replace('/', "--");
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".fastclaw")
            .join("models")
            .join(safe_name)
    }

    fn probe_dimensions(embedder: &hypembed::Embedder) -> Result<usize> {
        let opts = hypembed::EmbeddingOptions::default()
            .with_normalize(true)
            .with_pooling(hypembed::PoolingStrategy::Mean);
        let vecs = embedder
            .embed(&["dim probe"], &opts)
            .map_err(|e| anyhow::anyhow!("probe embed failed: {}", e))?;
        Ok(vecs.first().map(|v| v.len()).unwrap_or(384))
    }

    async fn ensure_model_downloaded(model_name: &str, model_dir: &Path) -> Result<()> {
        let all_present = REQUIRED_FILES.iter().all(|f| model_dir.join(f).exists());
        if all_present {
            return Ok(());
        }

        tracing::info!(model = model_name, dir = %model_dir.display(), "downloading model files");
        tokio::fs::create_dir_all(model_dir).await?;

        let client = reqwest::Client::builder()
            .user_agent("FastClaw/0.1.0")
            .timeout(std::time::Duration::from_secs(300))
            .build()?;

        for filename in REQUIRED_FILES {
            let dest = model_dir.join(filename);
            if dest.exists() {
                continue;
            }
            let url = format!("{}/{}/resolve/main/{}", HF_BASE, model_name, filename);
            tracing::info!(file = filename, "downloading {}", url);
            let resp = client.get(&url).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("failed to download {}: HTTP {}", url, resp.status());
            }
            let bytes = resp.bytes().await?;
            tokio::fs::write(&dest, &bytes).await?;
            tracing::info!(file = filename, bytes = bytes.len(), "downloaded");
        }

        Ok(())
    }

    #[async_trait]
    impl EmbeddingProvider for LocalEmbeddingProvider {
        async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<EmbeddingVec>> {
            let texts_owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
            let embedder = self.embedder.clone();

            let result = tokio::task::spawn_blocking(move || {
                let refs: Vec<&str> = texts_owned.iter().map(|s| s.as_str()).collect();
                let opts = hypembed::EmbeddingOptions::default()
                    .with_normalize(true)
                    .with_pooling(hypembed::PoolingStrategy::Mean);
                embedder.embed(&refs, &opts)
            })
            .await?
            .map_err(|e| anyhow::anyhow!("embed error: {}", e))?;

            Ok(result)
        }

        fn dimensions(&self) -> usize {
            self.dims
        }

        fn name(&self) -> &str {
            &self.model_name
        }
    }
}

// ---------- Remote embedding (OpenAI-compatible API) ----------

pub mod remote {
    use super::*;
    use serde::{Deserialize, Serialize};

    pub struct RemoteEmbeddingProvider {
        client: reqwest::Client,
        base_url: String,
        api_key: String,
        model: String,
        dims: usize,
    }

    impl RemoteEmbeddingProvider {
        pub fn new(base_url: &str, api_key: &str, model: &str, dims: usize) -> Self {
            let client = reqwest::Client::builder()
                .user_agent("FastClaw/0.1.0")
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());

            Self {
                client,
                base_url: base_url.trim_end_matches('/').to_string(),
                api_key: api_key.to_string(),
                model: model.to_string(),
                dims,
            }
        }
    }

    #[derive(Serialize)]
    struct EmbeddingRequest<'a> {
        model: &'a str,
        input: &'a [&'a str],
    }

    #[derive(Deserialize)]
    struct EmbeddingResponse {
        data: Vec<EmbeddingData>,
    }

    #[derive(Deserialize)]
    struct EmbeddingData {
        embedding: Vec<f32>,
        index: usize,
    }

    #[async_trait]
    impl EmbeddingProvider for RemoteEmbeddingProvider {
        async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<EmbeddingVec>> {
            let url = format!("{}/embeddings", self.base_url);
            let body = EmbeddingRequest {
                model: &self.model,
                input: texts,
            };

            let resp = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                anyhow::bail!("embedding API returned {status}: {text}");
            }

            let mut result: EmbeddingResponse = resp.json().await?;
            result.data.sort_by_key(|d| d.index);
            Ok(result.data.into_iter().map(|d| d.embedding).collect())
        }

        fn dimensions(&self) -> usize {
            self.dims
        }

        fn name(&self) -> &str {
            &self.model
        }
    }
}

// ---------- Vector norms ----------

/// L2 (Euclidean) norm of a vector. Used for persisting `embedding_norm` in SQLite
/// so search can skip degenerate rows and for future ANN-related heuristics.
pub fn l2_norm(v: &[f32]) -> f32 {
    let mut s = 0.0f32;
    for x in v {
        s += x * x;
    }
    s.sqrt()
}

// ---------- Cosine similarity ----------

/// Compute cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a.len() != b.len() {
        tracing::warn!(
            a_dim = a.len(),
            b_dim = b.len(),
            "cosine_similarity: dimension mismatch, returning 0.0"
        );
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < 1e-10 {
        0.0
    } else {
        dot / denom
    }
}

// ---------- Factory ----------

/// Create an embedding provider from config.
pub async fn create_embedding_provider(
    config: &fastclaw_core::config::EmbeddingConfig,
    credentials: Option<&fastclaw_core::config::CredentialsConfig>,
) -> Result<Box<dyn EmbeddingProvider>> {
    match config.provider.as_str() {
        "local" => {
            #[cfg(feature = "local-embedding")]
            {
                let provider = local::LocalEmbeddingProvider::new(&config.model).await?;
                Ok(Box::new(provider))
            }
            #[cfg(not(feature = "local-embedding"))]
            {
                let _ = (config, credentials);
                anyhow::bail!(
                    "local embedding provider requires the 'local-embedding' feature. \
                     Compile with --features local-embedding or switch to provider: \"remote\""
                );
            }
        }
        "remote" => {
            let base_url = config
                .base_url
                .as_deref()
                .unwrap_or("https://api.openai.com/v1");
            let api_key = config
                .api_key
                .as_deref()
                .or_else(|| credentials.and_then(|c| c.get_api_key("embedding")))
                .or_else(|| credentials.and_then(|c| c.get_api_key("openai")))
                .unwrap_or("");
            let dims = config.dimensions.unwrap_or(1536) as usize;

            let provider =
                remote::RemoteEmbeddingProvider::new(base_url, api_key, &config.model, dims);
            Ok(Box::new(provider))
        }
        "none" | "disabled" => {
            anyhow::bail!("embedding provider is disabled");
        }
        other => {
            anyhow::bail!("unknown embedding provider: '{other}'. Use 'local' or 'remote'");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_opposite_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn l2_norm_unit_vector() {
        let v = vec![1.0f32, 0.0, 0.0];
        assert!((l2_norm(&v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_empty_returns_zero() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn cosine_mismatched_length_returns_zero() {
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
    }

    #[cfg(feature = "local-embedding")]
    #[test]
    fn resolve_model_dir_format() {
        let dir = local::resolve_model_dir("sentence-transformers/all-MiniLM-L6-v2");
        assert!(dir
            .to_string_lossy()
            .contains("sentence-transformers--all-MiniLM-L6-v2"));
    }
}
