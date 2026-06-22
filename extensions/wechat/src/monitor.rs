use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use xiaolin_core::channel::InboundMessage;

use crate::api::client::WechatApiClient;
use crate::api::types::*;
use crate::message::{enrich_inbound_media, outbound_to_weixin, weixin_to_inbound};
use crate::plugin::{ContextTokenCache, ReplyCache};

const SESSION_EXPIRED_ERRCODE: i32 = -14;
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
const BACKOFF_DELAY: Duration = Duration::from_secs(30);
const RETRY_DELAY: Duration = Duration::from_secs(2);
const SESSION_PAUSE_DELAY: Duration = Duration::from_secs(300);
const MEDIA_WAIT_TIMEOUT: Duration = Duration::from_secs(300);
const MEDIA_WAIT_POLL: Duration = Duration::from_secs(5);

struct PendingMedia {
    inbound: InboundMessage,
    buffered_at: Instant,
}

pub struct WechatMonitor {
    client: WechatApiClient,
    account_id: String,
    inbound_tx: mpsc::UnboundedSender<InboundMessage>,
    cancel: CancellationToken,
    poll_timeout: Duration,
    sync_file: PathBuf,
    reply_cache: Arc<ReplyCache>,
    context_tokens: Arc<ContextTokenCache>,
    cdn_base_url: String,
}

impl WechatMonitor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: WechatApiClient,
        account_id: String,
        inbound_tx: mpsc::UnboundedSender<InboundMessage>,
        cancel: CancellationToken,
        poll_timeout: Duration,
        reply_cache: Arc<ReplyCache>,
        context_tokens: Arc<ContextTokenCache>,
        cdn_base_url: String,
    ) -> Self {
        let sync_file = xiaolin_core::paths::resolve_state_dir()
            .join("data")
            .join(format!("wechat-sync-{account_id}.buf"));

        Self {
            client,
            account_id,
            inbound_tx,
            cancel,
            poll_timeout,
            sync_file,
            reply_cache,
            context_tokens,
            cdn_base_url,
        }
    }

    pub async fn run(&self) {
        if let Err(e) = self.client.notify_start().await {
            tracing::warn!(account_id = %self.account_id, error = %e, "notifyStart failed (ignored)");
        }

        let mut cursor = self.load_cursor().unwrap_or_default();
        let mut next_timeout = self.poll_timeout;
        let mut consecutive_failures = 0u32;

        tracing::info!(
            account_id = %self.account_id,
            "wechat monitor started"
        );

        let mut pending: HashMap<String, PendingMedia> = HashMap::new();

        loop {
            if self.cancel.is_cancelled() {
                break;
            }

            // Use short poll when we have pending media, so we check for follow-up quickly.
            let poll_dur = if pending.is_empty() {
                next_timeout
            } else {
                MEDIA_WAIT_POLL
            };

            match self
                .client
                .get_updates(&cursor, poll_dur, &self.cancel)
                .await
            {
                Ok(resp) => {
                    if self.cancel.is_cancelled() {
                        break;
                    }

                    let msg_count = resp.msgs.as_ref().map_or(0, |m| m.len());
                    tracing::debug!(
                        account_id = %self.account_id,
                        ret = ?resp.ret,
                        errcode = ?resp.errcode,
                        msg_count,
                        pending_count = pending.len(),
                        "getUpdates poll result"
                    );

                    if let Some(timeout_ms) = resp.longpolling_timeout_ms {
                        if timeout_ms > 0 {
                            next_timeout = Duration::from_millis(timeout_ms);
                        }
                    }

                    let is_error = resp.ret.unwrap_or(0) != 0
                        || (resp.errcode.is_some() && resp.errcode.unwrap_or(0) != 0);

                    if is_error {
                        let errcode = resp.errcode.unwrap_or(0);
                        if errcode == SESSION_EXPIRED_ERRCODE
                            || resp.ret == Some(SESSION_EXPIRED_ERRCODE)
                        {
                            tracing::warn!(
                                account_id = %self.account_id,
                                pause_secs = SESSION_PAUSE_DELAY.as_secs(),
                                "session expired (errcode={errcode}), pausing before retry"
                            );
                            consecutive_failures = 0;
                            self.flush_pending(&mut pending);
                            self.sleep_or_cancel(SESSION_PAUSE_DELAY).await;
                            continue;
                        }

                        consecutive_failures += 1;
                        tracing::error!(
                            account_id = %self.account_id,
                            ret = ?resp.ret,
                            errcode = ?resp.errcode,
                            failures = consecutive_failures,
                            "getUpdates API error"
                        );

                        if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                            consecutive_failures = 0;
                            self.flush_pending(&mut pending);
                            self.sleep_or_cancel(BACKOFF_DELAY).await;
                        } else {
                            self.sleep_or_cancel(RETRY_DELAY).await;
                        }
                        continue;
                    }

                    consecutive_failures = 0;

                    if let Some(ref new_cursor) = resp.get_updates_buf {
                        if !new_cursor.is_empty() {
                            cursor = new_cursor.clone();
                            self.save_cursor(&cursor);
                        }
                    }

                    let processed = self
                        .process_and_merge_batch(resp.msgs.unwrap_or_default())
                        .await;

                    let mut to_send = Vec::new();

                    for inbound in processed {
                        let cid = inbound.chat_id.clone();

                        if let Some(mut buf) = pending.remove(&cid) {
                            if is_media_only(&inbound) {
                                // Another media message — add to buffer
                                buf.inbound.attachments.extend(inbound.attachments);
                                let count = buf.inbound.attachments.len();
                                pending.insert(cid.clone(), buf);
                                self.send_hint(&cid, &format!(
                                    "📎 已收到 {count} 个附件，请发送描述文字后一起处理～"
                                )).await;
                            } else {
                                // Text arrived! Merge with buffered media.
                                buf.inbound.attachments.extend(inbound.attachments);
                                buf.inbound.text = inbound.text;
                                buf.inbound.msg_type = "mixed".to_string();
                                tracing::info!(
                                    chat_id = %cid,
                                    attachment_count = buf.inbound.attachments.len(),
                                    "merged buffered media with follow-up text"
                                );
                                to_send.push(buf.inbound);
                            }
                        } else if is_media_only(&inbound) {
                            // First media-only message — buffer and prompt user
                            let count = inbound.attachments.len();
                            let hint = if count == 1 {
                                "📷 已收到图片/文件，请补充描述文字后一起处理～".to_string()
                            } else {
                                format!("📎 已收到 {count} 个附件，请发送描述文字后一起处理～")
                            };
                            pending.insert(cid.clone(), PendingMedia {
                                inbound,
                                buffered_at: Instant::now(),
                            });
                            self.send_hint(&cid, &hint).await;
                        } else {
                            to_send.push(inbound);
                        }
                    }

                    // Flush expired pending entries (timeout)
                    let expired: Vec<String> = pending
                        .iter()
                        .filter(|(_, p)| p.buffered_at.elapsed() > MEDIA_WAIT_TIMEOUT)
                        .map(|(k, _)| k.clone())
                        .collect();
                    for cid in expired {
                        if let Some(buf) = pending.remove(&cid) {
                            tracing::info!(
                                chat_id = %cid,
                                attachment_count = buf.inbound.attachments.len(),
                                "media buffer timed out, submitting without description"
                            );
                            to_send.push(buf.inbound);
                        }
                    }

                    for inbound in to_send {
                        tracing::info!(
                            account_id = %self.account_id,
                            from = %inbound.sender_id,
                            msg_type = %inbound.msg_type,
                            message_id = %inbound.message_id,
                            attachment_count = inbound.attachments.len(),
                            "inbound wechat message"
                        );
                        if self.inbound_tx.send(inbound).is_err() {
                            tracing::error!("inbound_tx closed, stopping monitor");
                            return;
                        }
                    }
                }
                Err(e) => {
                    if self.cancel.is_cancelled() {
                        break;
                    }
                    consecutive_failures += 1;
                    tracing::error!(
                        account_id = %self.account_id,
                        error = %e,
                        failures = consecutive_failures,
                        "getUpdates network error"
                    );

                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        consecutive_failures = 0;
                        self.flush_pending(&mut pending);
                        self.sleep_or_cancel(BACKOFF_DELAY).await;
                    } else {
                        self.sleep_or_cancel(RETRY_DELAY).await;
                    }
                }
            }
        }

        self.flush_pending(&mut pending);

        if let Err(e) = self.client.notify_stop().await {
            tracing::warn!(account_id = %self.account_id, error = %e, "notifyStop failed (ignored)");
        }

        tracing::info!(account_id = %self.account_id, "wechat monitor stopped");
    }

    /// Send a hint message back to the user (fire-and-forget).
    async fn send_hint(&self, chat_id: &str, text: &str) {
        let ctx_token = self.context_tokens.get(&self.account_id, chat_id);
        let msg = xiaolin_core::channel::OutboundMessage {
            target_id: chat_id.to_string(),
            target_type: "p2p".to_string(),
            text: text.to_string(),
            reply_to: None,
            image_key: None,
            attachments: vec![],
        };
        let weixin_msg = outbound_to_weixin(&msg, ctx_token.as_deref());
        if let Err(e) = self.client.send_message(weixin_msg).await {
            tracing::debug!(error = %e, chat_id, "failed to send media hint (ignored)");
        }
    }

    /// Process a batch of raw WeChat messages: convert, enrich media,
    /// and merge consecutive messages from the same chat_id into one.
    async fn process_and_merge_batch(
        &self,
        msgs: Vec<WeixinMessage>,
    ) -> Vec<InboundMessage> {
        let mut per_chat: HashMap<String, Vec<InboundMessage>> = HashMap::new();
        let mut chat_order: Vec<String> = Vec::new();

        for msg in msgs {
            if let Some(mut inbound) =
                weixin_to_inbound(&msg, "wechat", Some(&self.account_id))
            {
                let ctx_token = msg.context_token.as_deref();

                if let Some(token) = ctx_token {
                    self.context_tokens
                        .update(&self.account_id, &inbound.chat_id, token);
                }

                self.reply_cache
                    .insert(&inbound.message_id, &inbound.chat_id, ctx_token);

                let has_media =
                    matches!(inbound.msg_type.as_str(), "image" | "file" | "video");
                if has_media {
                    enrich_inbound_media(&mut inbound, &msg, &self.cdn_base_url).await;
                }

                let cid = inbound.chat_id.clone();
                if !per_chat.contains_key(&cid) {
                    chat_order.push(cid.clone());
                }
                per_chat.entry(cid).or_default().push(inbound);
            }
        }

        let mut result = Vec::new();
        for cid in chat_order {
            if let Some(group) = per_chat.remove(&cid) {
                result.push(merge_inbound_group(group));
            }
        }
        result
    }

    fn flush_pending(&self, pending: &mut HashMap<String, PendingMedia>) {
        for (_, buf) in pending.drain() {
            tracing::debug!(
                chat_id = %buf.inbound.chat_id,
                "flushing buffered media message"
            );
            if self.inbound_tx.send(buf.inbound).is_err() {
                tracing::error!("inbound_tx closed during flush");
            }
        }
    }

    async fn sleep_or_cancel(&self, dur: Duration) {
        tokio::select! {
            () = tokio::time::sleep(dur) => {}
            () = self.cancel.cancelled() => {}
        }
    }

    fn load_cursor(&self) -> Option<String> {
        std::fs::read_to_string(&self.sync_file).ok()
    }

    fn save_cursor(&self, cursor: &str) {
        if let Some(parent) = self.sync_file.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(&self.sync_file, cursor).ok();
    }
}

/// Merge a group of InboundMessages from the same chat into one.
fn merge_inbound_group(mut group: Vec<InboundMessage>) -> InboundMessage {
    if group.len() == 1 {
        return group.remove(0);
    }

    let mut base = group.remove(0);
    let mut texts: Vec<String> = vec![];
    if !base.text.is_empty() {
        texts.push(base.text.clone());
    }

    for other in group {
        if !other.text.is_empty() && other.text != "[图片]" && other.text != "[文件]" {
            texts.push(other.text);
        }
        base.attachments.extend(other.attachments);
    }

    base.text = texts.join("\n");

    if !base.attachments.is_empty() && base.msg_type == "text" {
        base.msg_type = "mixed".to_string();
    }

    base
}

/// A message is "media only" if it has attachments but only placeholder text.
fn is_media_only(msg: &InboundMessage) -> bool {
    !msg.attachments.is_empty()
        && (msg.text.is_empty()
            || msg.text == "[图片]"
            || msg.text == "[文件]"
            || msg.text == "[视频]")
}
