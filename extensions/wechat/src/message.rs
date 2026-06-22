use std::path::Path;

use xiaolin_core::channel::{Attachment, InboundMessage, OutboundMessage};
use serde_json::json;

use crate::api::client::WechatApiClient;
use crate::api::types::*;
use crate::media::download::{download_media, media_temp_dir, sanitize_download_filename};
use crate::media::upload::{build_message_item, media_type_from_mime, mime_from_extension, upload_to_cdn};

/// Convert a WeChat inbound message to XiaoLin's InboundMessage.
pub fn weixin_to_inbound(
    msg: &WeixinMessage,
    channel_id: &str,
    account_id: Option<&str>,
) -> Option<InboundMessage> {
    let from = msg.from_user_id.as_deref().unwrap_or_default();
    if from.is_empty() {
        return None;
    }

    // Only process USER messages (not BOT echo)
    if msg.message_type == Some(MESSAGE_TYPE_BOT) {
        return None;
    }

    let items = msg.item_list.as_deref().unwrap_or_default();
    let mut text_parts: Vec<String> = Vec::new();
    let mut msg_type = "text".to_string();
    let mut extra = json!({});

    for item in items {
        match item.item_type {
            Some(MSG_ITEM_TYPE_TEXT) => {
                if let Some(ref ti) = item.text_item {
                    text_parts.push(ti.text.clone().unwrap_or_default());
                }
            }
            Some(MSG_ITEM_TYPE_IMAGE) => {
                msg_type = "image".to_string();
                text_parts.push("[图片]".to_string());
                if let Some(ref img) = item.image_item {
                    extra["image_item"] = serde_json::to_value(img).unwrap_or_default();
                }
            }
            Some(MSG_ITEM_TYPE_VOICE) => {
                msg_type = "voice".to_string();
                if let Some(ref voice) = item.voice_item {
                    let voice_text = voice
                        .text
                        .as_deref()
                        .filter(|t| !t.is_empty())
                        .unwrap_or("[语音]");
                    text_parts.push(voice_text.to_string());
                    extra["voice_item"] = serde_json::to_value(voice).unwrap_or_default();
                } else {
                    text_parts.push("[语音]".to_string());
                }
            }
            Some(MSG_ITEM_TYPE_FILE) => {
                msg_type = "file".to_string();
                if let Some(ref file) = item.file_item {
                    let name = file.file_name.as_deref().unwrap_or("unknown");
                    text_parts.push(format!("[文件: {name}]"));
                    extra["file_item"] = serde_json::to_value(file).unwrap_or_default();
                } else {
                    text_parts.push("[文件]".to_string());
                }
            }
            Some(MSG_ITEM_TYPE_VIDEO) => {
                msg_type = "video".to_string();
                text_parts.push("[视频]".to_string());
                if let Some(ref video) = item.video_item {
                    extra["video_item"] = serde_json::to_value(video).unwrap_or_default();
                }
            }
            _ => {}
        }

        if let Some(ref rm) = item.ref_msg {
            extra["ref_msg"] = serde_json::to_value(rm).unwrap_or_default();
        }
    }

    if let Some(ref ct) = msg.context_token {
        extra["context_token"] = json!(ct);
    }
    extra["item_list"] = serde_json::to_value(items).unwrap_or_default();

    let message_id = msg
        .message_id
        .map(|id| id.to_string())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    Some(InboundMessage {
        channel_id: channel_id.to_string(),
        account_id: account_id.map(String::from),
        sender_id: from.to_string(),
        chat_id: from.to_string(),
        message_id,
        text: text_parts.join("\n"),
        msg_type,
        chat_type: "p2p".to_string(),
        bot_mentioned: true,
        extra,
        attachments: vec![],
    })
}

/// Enrich an InboundMessage by downloading media attachments from CDN.
/// Call this after `weixin_to_inbound` for messages that contain images/files.
pub async fn enrich_inbound_media(
    inbound: &mut InboundMessage,
    msg: &WeixinMessage,
    cdn_base_url: &str,
) {
    let items = msg.item_list.as_deref().unwrap_or_default();
    let dest_dir = media_temp_dir();

    for item in items {
        let result = match item.item_type {
            Some(MSG_ITEM_TYPE_IMAGE) => {
                if let Some(ref img) = item.image_item {
                    if let Some(ref media) = img.media {
                        let filename = format!("img_{}.png", uuid::Uuid::new_v4());
                        download_media(media, cdn_base_url, &dest_dir, &filename)
                            .await
                            .map(|p| Attachment {
                                file_path: p.to_string_lossy().to_string(),
                                mime_type: Some("image/png".to_string()),
                                file_name: Some(filename),
                            })
                    } else {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            Some(MSG_ITEM_TYPE_FILE) => {
                if let Some(ref file) = item.file_item {
                    if let Some(ref media) = file.media {
                        let name = file
                            .file_name
                            .as_deref()
                            .unwrap_or("file");
                        let safe_name = match sanitize_download_filename(name) {
                            Ok(n) => n,
                            Err(e) => {
                                tracing::warn!(error = %e, file_name = name, "rejecting unsafe inbound file name");
                                continue;
                            }
                        };
                        let filename = format!("{}_{}", uuid::Uuid::new_v4(), safe_name);
                        let mime = mime_from_extension(Path::new(&safe_name));
                        download_media(media, cdn_base_url, &dest_dir, &filename)
                            .await
                            .map(|p| Attachment {
                                file_path: p.to_string_lossy().to_string(),
                                mime_type: Some(mime.to_string()),
                                file_name: Some(safe_name),
                            })
                    } else {
                        continue;
                    }
                } else {
                    continue;
                }
            }
            _ => continue,
        };

        match result {
            Ok(attachment) => inbound.attachments.push(attachment),
            Err(e) => tracing::warn!(error = %e, "failed to download inbound media attachment"),
        }
    }
}

/// Convert a XiaoLin OutboundMessage to a WeChat WeixinMessage (text only).
pub fn outbound_to_weixin(
    msg: &OutboundMessage,
    context_token: Option<&str>,
) -> WeixinMessage {
    let mut items = Vec::new();

    if !msg.text.is_empty() {
        items.push(MessageItem {
            item_type: Some(MSG_ITEM_TYPE_TEXT),
            text_item: Some(TextItem {
                text: Some(msg.text.clone()),
            }),
            ..Default::default()
        });
    }

    WeixinMessage {
        from_user_id: Some(String::new()),
        to_user_id: Some(msg.target_id.clone()),
        client_id: Some(generate_client_id()),
        message_type: Some(MESSAGE_TYPE_BOT),
        message_state: Some(MESSAGE_STATE_FINISH),
        context_token: context_token.map(String::from),
        item_list: if items.is_empty() { None } else { Some(items) },
        ..Default::default()
    }
}

/// Convert a XiaoLin OutboundMessage with attachments to WeChat WeixinMessages.
/// Each item (text and each media) is sent as a **separate** message,
/// matching the openclaw-weixin protocol requirement.
pub async fn outbound_to_weixin_with_media(
    msg: &OutboundMessage,
    context_token: Option<&str>,
    client: &WechatApiClient,
    cdn_base_url: &str,
) -> anyhow::Result<Vec<WeixinMessage>> {
    let mut messages = Vec::new();

    if !msg.text.is_empty() {
        messages.push(WeixinMessage {
            from_user_id: Some(String::new()),
            to_user_id: Some(msg.target_id.clone()),
            client_id: Some(generate_client_id()),
            message_type: Some(MESSAGE_TYPE_BOT),
            message_state: Some(MESSAGE_STATE_FINISH),
            context_token: context_token.map(String::from),
            item_list: Some(vec![MessageItem {
                item_type: Some(MSG_ITEM_TYPE_TEXT),
                text_item: Some(TextItem {
                    text: Some(msg.text.clone()),
                }),
                ..Default::default()
            }]),
            ..Default::default()
        });
    }

    for attachment in &msg.attachments {
        let path = Path::new(&attachment.file_path);
        if !path.exists() {
            tracing::warn!(path = %attachment.file_path, "attachment file not found, skipping");
            continue;
        }

        let mime = attachment
            .mime_type
            .as_deref()
            .unwrap_or_else(|| mime_from_extension(path));
        let media_type = media_type_from_mime(mime);

        match upload_to_cdn(client, path, media_type, &msg.target_id, cdn_base_url).await {
            Ok(info) => match build_message_item(media_type, info, path) {
                Ok(item) => {
                    messages.push(WeixinMessage {
                        from_user_id: Some(String::new()),
                        to_user_id: Some(msg.target_id.clone()),
                        client_id: Some(generate_client_id()),
                        message_type: Some(MESSAGE_TYPE_BOT),
                        message_state: Some(MESSAGE_STATE_FINISH),
                        context_token: context_token.map(String::from),
                        item_list: Some(vec![item]),
                        ..Default::default()
                    });
                }
                Err(e) => tracing::warn!(error = %e, path = %attachment.file_path, "failed to build media item"),
            },
            Err(e) => tracing::warn!(error = %e, path = %attachment.file_path, "failed to upload attachment to CDN"),
        }
    }

    Ok(messages)
}

fn generate_client_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("xiaolin-wechat-{ts}-{:04x}", rand::random::<u16>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_inbound() {
        let msg = WeixinMessage {
            from_user_id: Some("user1@im.wechat".into()),
            message_type: Some(MESSAGE_TYPE_USER),
            item_list: Some(vec![MessageItem {
                item_type: Some(MSG_ITEM_TYPE_TEXT),
                text_item: Some(TextItem {
                    text: Some("hello".into()),
                }),
                ..Default::default()
            }]),
            context_token: Some("ctx123".into()),
            ..Default::default()
        };

        let inbound = weixin_to_inbound(&msg, "wechat", Some("acc1")).unwrap();
        assert_eq!(inbound.text, "hello");
        assert_eq!(inbound.msg_type, "text");
        assert_eq!(inbound.sender_id, "user1@im.wechat");
        assert_eq!(inbound.chat_type, "p2p");
    }

    #[test]
    fn test_image_inbound() {
        let msg = WeixinMessage {
            from_user_id: Some("user1@im.wechat".into()),
            message_type: Some(MESSAGE_TYPE_USER),
            item_list: Some(vec![MessageItem {
                item_type: Some(MSG_ITEM_TYPE_IMAGE),
                image_item: Some(ImageItem::default()),
                ..Default::default()
            }]),
            ..Default::default()
        };

        let inbound = weixin_to_inbound(&msg, "wechat", None).unwrap();
        assert_eq!(inbound.msg_type, "image");
        assert_eq!(inbound.text, "[图片]");
    }

    #[test]
    fn test_outbound_text() {
        let msg = OutboundMessage {
            target_id: "user1@im.wechat".into(),
            target_type: "p2p".into(),
            text: "Hello!".into(),
            reply_to: None,
            image_key: None,
            attachments: vec![],
        };

        let weixin = outbound_to_weixin(&msg, Some("ctx123"));
        assert_eq!(weixin.to_user_id.as_deref(), Some("user1@im.wechat"));
        assert_eq!(weixin.context_token.as_deref(), Some("ctx123"));
        let items = weixin.item_list.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].item_type, Some(MSG_ITEM_TYPE_TEXT));
    }

    #[test]
    fn test_bot_message_filtered() {
        let msg = WeixinMessage {
            from_user_id: Some("bot@im.wechat".into()),
            message_type: Some(MESSAGE_TYPE_BOT),
            item_list: Some(vec![]),
            ..Default::default()
        };
        assert!(weixin_to_inbound(&msg, "wechat", None).is_none());
    }
}
