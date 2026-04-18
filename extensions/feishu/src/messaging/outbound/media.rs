use crate::client::FeishuClient;
use std::sync::Arc;

/// Upload an image to Feishu IM and return `image_key` (requires user OAuth).
///
/// `format_hint` may be a container hint (`message`, `avatar`) or a file kind (`png`, `jpeg`, …).
pub async fn upload_image(
    client: &Arc<FeishuClient>,
    image_data: &[u8],
    format_hint: &str,
) -> anyhow::Result<String> {
    if !client.user_oauth_configured() {
        anyhow::bail!(crate::oauth::OAuthConfig::missing_user_token_message());
    }
    let lower = format_hint.to_ascii_lowercase();
    let feishu_image_type = match lower.as_str() {
        "avatar" => "avatar",
        _ => "message",
    };
    let mime = match lower.as_str() {
        "png" | "image/png" => "image/png",
        "gif" | "image/gif" => "image/gif",
        "webp" | "image/webp" => "image/webp",
        "jpg" | "jpeg" | "image/jpeg" => "image/jpeg",
        _ => "image/jpeg",
    };
    let part = reqwest::multipart::Part::bytes(image_data.to_vec())
        .file_name("upload.bin")
        .mime_str(mime)
        .map_err(|e| anyhow::anyhow!("invalid mime for multipart: {e}"))?;
    let form = reqwest::multipart::Form::new()
        .part("image", part)
        .text("image_type", feishu_image_type.to_string());
    let data = client.user_post_multipart("/im/v1/images", form).await?;
    let key = data
        .get("image_key")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("missing image_key in Feishu response"))?;
    Ok(key)
}

/// Download a resource (image/file) from a Feishu message (streams the HTTP body).
pub async fn download_resource(
    client: &Arc<FeishuClient>,
    message_id: &str,
    file_key: &str,
    resource_type: &str,
) -> anyhow::Result<Vec<u8>> {
    if !client.user_oauth_configured() {
        anyhow::bail!(crate::oauth::OAuthConfig::missing_user_token_message());
    }
    client
        .download_message_resource(message_id, file_key, resource_type)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn media_module_exists() {
        assert!(true);
    }

    #[tokio::test]
    async fn upload_image_without_oauth_returns_clear_error() {
        let client = Arc::new(FeishuClient::new("id", "secret"));
        let err = upload_image(&client, &[1, 2, 3], "png")
            .await
            .expect_err("expected oauth error");
        let msg = err.to_string();
        assert!(
            msg.contains("userAccessToken") || msg.contains("user OAuth"),
            "unexpected: {msg}"
        );
    }

    #[tokio::test]
    async fn download_resource_without_oauth_returns_clear_error() {
        let client = Arc::new(FeishuClient::new("id", "secret"));
        let err = download_resource(&client, "om_1", "fk_1", "image")
            .await
            .expect_err("expected oauth error");
        let msg = err.to_string();
        assert!(
            msg.contains("userAccessToken") || msg.contains("user OAuth"),
            "unexpected: {msg}"
        );
    }
}
