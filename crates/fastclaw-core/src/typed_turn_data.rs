use std::any::Any;
use std::sync::Arc;

use crate::agent_config::AgentConfig;
use crate::types::ChatRequest;

/// Typed data passed through `SessionOp::UserTurn` to avoid JSON round-trips.
///
/// This struct is stored as `Arc<dyn Any + Send + Sync>` in the session actor's
/// `TurnParams::typed_data` field. The gateway wraps it when submitting a turn,
/// and the session bridge downcasts it to extract the typed request and config.
pub struct TypedTurnData {
    pub enriched_request: ChatRequest,
    pub agent_config: AgentConfig,
}

impl TypedTurnData {
    pub fn wrap(request: ChatRequest, config: AgentConfig) -> Arc<dyn Any + Send + Sync> {
        Arc::new(Self {
            enriched_request: request,
            agent_config: config,
        })
    }

    pub fn extract(data: &Arc<dyn Any + Send + Sync>) -> Option<&Self> {
        data.downcast_ref::<Self>()
    }
}
