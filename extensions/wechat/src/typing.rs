use std::time::{Duration, Instant};

use dashmap::DashMap;

use crate::api::client::WechatApiClient;
use crate::api::types::{BaseInfo, SendTypingReq, TYPING_STATUS_CANCEL, TYPING_STATUS_TYPING};

const TICKET_TTL: Duration = Duration::from_secs(600);

struct TicketEntry {
    ticket: String,
    fetched_at: Instant,
}

pub struct TypingManager {
    ticket_cache: DashMap<(String, String), TicketEntry>,
}

impl Default for TypingManager {
    fn default() -> Self {
        Self {
            ticket_cache: DashMap::new(),
        }
    }
}

impl TypingManager {
    pub fn new() -> Self {
        Self::default()
    }

    async fn get_ticket(
        &self,
        client: &WechatApiClient,
        account_id: &str,
        user_id: &str,
        context_token: Option<&str>,
    ) -> anyhow::Result<String> {
        let key = (account_id.to_string(), user_id.to_string());

        if let Some(entry) = self.ticket_cache.get(&key) {
            if entry.fetched_at.elapsed() < TICKET_TTL {
                return Ok(entry.ticket.clone());
            }
        }

        let resp = client.get_config(user_id, context_token).await?;
        let ticket = resp
            .typing_ticket
            .ok_or_else(|| anyhow::anyhow!("no typing_ticket in getConfig response"))?;

        self.ticket_cache.insert(
            key,
            TicketEntry {
                ticket: ticket.clone(),
                fetched_at: Instant::now(),
            },
        );

        Ok(ticket)
    }

    pub async fn start_typing(
        &self,
        client: &WechatApiClient,
        account_id: &str,
        user_id: &str,
        context_token: Option<&str>,
    ) -> anyhow::Result<()> {
        let ticket = self
            .get_ticket(client, account_id, user_id, context_token)
            .await?;

        client
            .send_typing(SendTypingReq {
                ilink_user_id: user_id.to_string(),
                typing_ticket: ticket,
                status: TYPING_STATUS_TYPING,
                base_info: BaseInfo {
                    channel_version: None,
                    bot_agent: None,
                },
            })
            .await?;

        Ok(())
    }

    pub async fn stop_typing(
        &self,
        client: &WechatApiClient,
        account_id: &str,
        user_id: &str,
        context_token: Option<&str>,
    ) -> anyhow::Result<()> {
        let ticket = self
            .get_ticket(client, account_id, user_id, context_token)
            .await?;

        client
            .send_typing(SendTypingReq {
                ilink_user_id: user_id.to_string(),
                typing_ticket: ticket,
                status: TYPING_STATUS_CANCEL,
                base_info: BaseInfo {
                    channel_version: None,
                    bot_agent: None,
                },
            })
            .await?;

        Ok(())
    }
}
