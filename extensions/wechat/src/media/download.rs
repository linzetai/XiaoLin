use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::api::types::CDNMedia;
use crate::media::crypto::aes128_ecb_decrypt;

/// Download and decrypt a CDN media file to a local path.
/// Uses `full_url` or falls back to `cdn_base_url + encrypt_query_param`.
pub async fn download_media(
    cdn_media: &CDNMedia,
    cdn_base_url: &str,
    dest_dir: &Path,
    filename: &str,
) -> anyhow::Result<PathBuf> {
    let url = resolve_download_url(cdn_media, cdn_base_url)?;

    let http = reqwest::Client::new();
    let ciphertext = http.get(&url).send().await?.bytes().await?;

    let key_bytes = extract_aes_key(cdn_media)?;
    let plaintext = aes128_ecb_decrypt(&ciphertext, &key_bytes)?;

    tokio::fs::create_dir_all(dest_dir).await?;
    let dest_path = dest_dir.join(filename);
    tokio::fs::write(&dest_path, &plaintext).await?;

    Ok(dest_path)
}

fn resolve_download_url(cdn_media: &CDNMedia, cdn_base_url: &str) -> anyhow::Result<String> {
    if let Some(ref url) = cdn_media.full_url {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(ref param) = cdn_media.encrypt_query_param {
        let base = cdn_base_url.trim_end_matches('/');
        let encoded = urlencoding::encode(param);
        return Ok(format!("{base}/download?encrypted_query_param={encoded}"));
    }
    anyhow::bail!("no CDN download URL available (neither full_url nor encrypt_query_param)")
}

fn extract_aes_key(cdn_media: &CDNMedia) -> anyhow::Result<[u8; 16]> {
    if let Some(ref key_str) = cdn_media.aes_key {
        if let Ok(decoded) = BASE64.decode(key_str) {
            if decoded.len() == 16 {
                return decoded
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("AES key base64 must decode to 16 bytes"));
            }
        }
        if let Ok(decoded) = hex::decode(key_str) {
            if decoded.len() == 16 {
                return decoded
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("AES key hex must decode to 16 bytes"));
            }
        }
        anyhow::bail!("AES key is neither valid base64 nor hex (len={})", key_str.len());
    }
    anyhow::bail!("no AES key available for decryption")
}

/// Return the path to the media temp directory.
pub fn media_temp_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".xiaolin-dev")
        .join("data")
        .join("wechat-media")
}

/// Delete files older than `max_age` in the media temp directory.
pub async fn cleanup_old_media(max_age: Duration) -> anyhow::Result<usize> {
    let dir = media_temp_dir();
    if !dir.exists() {
        return Ok(0);
    }

    let mut count = 0usize;
    let mut entries = tokio::fs::read_dir(&dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default()
            > max_age
            && tokio::fs::remove_file(entry.path()).await.is_ok()
        {
            count += 1;
        }
    }
    Ok(count)
}
