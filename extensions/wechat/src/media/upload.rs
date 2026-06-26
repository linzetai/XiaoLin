use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use md5::{Digest, Md5};
use rand::RngCore;

use crate::api::client::WechatApiClient;
use crate::api::types::*;
use crate::media::crypto::aes128_ecb_encrypt;

/// Upload result with CDN reference info needed to build message items.
pub struct UploadedFileInfo {
    pub cdn_media: CDNMedia,
    /// AES key in base64 for CDNMedia.aes_key
    pub aes_key_b64: String,
    pub raw_size: u64,
    pub file_size: u64,
    pub raw_md5: String,
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Upload a local file to WeChat CDN and return the CDNMedia reference
/// plus the constructed MessageItem.
pub async fn upload_media(
    client: &WechatApiClient,
    file_path: &Path,
    media_type: u32,
    to_user_id: &str,
    cdn_base_url: &str,
) -> anyhow::Result<MessageItem> {
    let info = upload_to_cdn(client, file_path, media_type, to_user_id, cdn_base_url).await?;
    build_message_item(media_type, info, file_path)
}

/// Upload file bytes to CDN without building a MessageItem.
pub async fn upload_to_cdn(
    client: &WechatApiClient,
    file_path: &Path,
    media_type: u32,
    to_user_id: &str,
    cdn_base_url: &str,
) -> anyhow::Result<UploadedFileInfo> {
    let plaintext = tokio::fs::read(file_path).await?;
    let rawsize = plaintext.len() as u64;
    let rawfilemd5 = hex_md5(&plaintext);

    let mut aes_key = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut aes_key);
    let aeskey_hex = hex_encode(&aes_key);

    let ciphertext = aes128_ecb_encrypt(&plaintext, &aes_key);
    let filesize = ciphertext.len() as u64;
    let filekey = format!("{:032x}", rand::random::<u128>());

    let upload_resp = client
        .get_upload_url(GetUploadUrlReq {
            filekey: Some(filekey.clone()),
            media_type: Some(media_type),
            to_user_id: Some(to_user_id.to_string()),
            rawsize: Some(rawsize),
            rawfilemd5: Some(rawfilemd5.clone()),
            filesize: Some(filesize),
            thumb_rawsize: None,
            thumb_rawfilemd5: None,
            thumb_filesize: None,
            no_need_thumb: Some(true),
            aeskey: Some(aeskey_hex.clone()),
            base_info: BaseInfo {
                channel_version: None,
                bot_agent: None,
            },
        })
        .await?;

    tracing::debug!(
        upload_full_url = ?upload_resp.upload_full_url,
        upload_param = ?upload_resp.upload_param,
        "get_upload_url response"
    );

    let upload_url = resolve_cdn_url(&upload_resp, cdn_base_url, &filekey)?;

    tracing::info!(
        url = %upload_url,
        ciphertext_len = ciphertext.len(),
        "uploading encrypted file to CDN"
    );

    let http = reqwest::Client::new();
    let resp = http
        .post(&upload_url)
        .header("Content-Type", "application/octet-stream")
        .body(ciphertext)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_msg = resp
            .headers()
            .get("x-error-message")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!(
            "CDN upload failed: {status} err={} body={body} url={upload_url}",
            err_msg.as_deref().unwrap_or("none")
        );
    }

    let download_param = resp
        .headers()
        .get("x-encrypted-param")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let aeskey_hex_b64 = BASE64.encode(aeskey_hex.as_bytes());

    let cdn_media = CDNMedia {
        encrypt_query_param: download_param.or(upload_resp.upload_param),
        aes_key: Some(aeskey_hex_b64.clone()),
        encrypt_type: Some(1),
        full_url: upload_resp.upload_full_url,
    };

    Ok(UploadedFileInfo {
        cdn_media,
        aes_key_b64: aeskey_hex_b64,
        raw_size: rawsize,
        file_size: filesize,
        raw_md5: rawfilemd5,
    })
}

/// Resolve the CDN upload URL: prefer `upload_full_url`, fall back to
/// `${cdnBaseUrl}/upload?encrypted_query_param=${uploadParam}&filekey=${filekey}`.
fn resolve_cdn_url(
    resp: &GetUploadUrlResp,
    cdn_base_url: &str,
    filekey: &str,
) -> anyhow::Result<String> {
    if let Some(ref full_url) = resp.upload_full_url {
        let trimmed = full_url.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(ref param) = resp.upload_param {
        let base = cdn_base_url.trim_end_matches('/');
        let encoded_param = urlencoding::encode(param);
        let encoded_key = urlencoding::encode(filekey);
        return Ok(format!(
            "{base}/upload?encrypted_query_param={encoded_param}&filekey={encoded_key}"
        ));
    }
    anyhow::bail!("getUploadUrl returned neither upload_full_url nor upload_param")
}

/// Build a MessageItem from uploaded CDN info.
pub fn build_message_item(
    media_type: u32,
    info: UploadedFileInfo,
    file_path: &Path,
) -> anyhow::Result<MessageItem> {
    let cdn_media = info.cdn_media;
    match media_type {
        UPLOAD_MEDIA_TYPE_IMAGE => Ok(build_image_item(cdn_media, info.file_size)),
        UPLOAD_MEDIA_TYPE_FILE => {
            let name = file_path
                .file_name()
                .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().to_string());
            Ok(build_file_item(
                cdn_media,
                &name,
                info.raw_size,
                &info.raw_md5,
            ))
        }
        UPLOAD_MEDIA_TYPE_VIDEO => Ok(MessageItem {
            item_type: Some(MSG_ITEM_TYPE_VIDEO),
            video_item: Some(VideoItem {
                media: Some(cdn_media),
                ..Default::default()
            }),
            ..Default::default()
        }),
        UPLOAD_MEDIA_TYPE_VOICE => Ok(MessageItem {
            item_type: Some(MSG_ITEM_TYPE_VOICE),
            voice_item: Some(VoiceItem {
                media: Some(cdn_media),
                ..Default::default()
            }),
            ..Default::default()
        }),
        _ => anyhow::bail!("unsupported media_type: {media_type}"),
    }
}

fn build_image_item(cdn_media: CDNMedia, ciphertext_size: u64) -> MessageItem {
    MessageItem {
        item_type: Some(MSG_ITEM_TYPE_IMAGE),
        image_item: Some(ImageItem {
            media: Some(cdn_media),
            mid_size: Some(ciphertext_size),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn build_file_item(cdn_media: CDNMedia, file_name: &str, raw_size: u64, md5: &str) -> MessageItem {
    MessageItem {
        item_type: Some(MSG_ITEM_TYPE_FILE),
        file_item: Some(FileItem {
            media: Some(cdn_media),
            file_name: Some(file_name.to_string()),
            md5: Some(md5.to_string()),
            len: Some(raw_size.to_string()),
        }),
        ..Default::default()
    }
}

/// Infer MIME type from file extension.
pub fn mime_from_extension(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("svg") => "image/svg+xml",
        Some("mp4") => "video/mp4",
        Some("avi") => "video/avi",
        Some("mov") => "video/quicktime",
        Some("pdf") => "application/pdf",
        Some("txt") => "text/plain",
        Some("json") => "application/json",
        Some("csv") => "text/csv",
        Some("zip") => "application/zip",
        Some("doc" | "docx") => "application/msword",
        Some("xls" | "xlsx") => "application/vnd.ms-excel",
        _ => "application/octet-stream",
    }
}

/// Map MIME type to WeChat upload media type constant.
pub fn media_type_from_mime(mime: &str) -> u32 {
    if mime.starts_with("image/") {
        UPLOAD_MEDIA_TYPE_IMAGE
    } else if mime.starts_with("video/") {
        UPLOAD_MEDIA_TYPE_VIDEO
    } else {
        UPLOAD_MEDIA_TYPE_FILE
    }
}

fn hex_md5(data: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}
