#![cfg(feature = "ts")]

use ts_rs::TS;

#[test]
fn export_all_bindings() {
    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("generated");
    std::fs::create_dir_all(&out_dir).unwrap();
    std::env::set_var("TS_RS_EXPORT_DIR", &out_dir);
    let cfg = ts_rs::Config::from_env();

    // Core types — export_all cascades to nested dependencies.
    fastclaw_protocol::AgentEvent::export_all(&cfg).unwrap();
    fastclaw_protocol::ClientOp::export_all(&cfg).unwrap();
    fastclaw_protocol::HistoryItem::export_all(&cfg).unwrap();
    fastclaw_protocol::TokenUsage::export_all(&cfg).unwrap();
    fastclaw_protocol::TurnSummary::export_all(&cfg).unwrap();
    fastclaw_protocol::ApprovalDecision::export_all(&cfg).unwrap();
    fastclaw_protocol::PendingAction::export_all(&cfg).unwrap();
    fastclaw_protocol::ToolDefinition::export_all(&cfg).unwrap();

    // Types not reachable from the roots above.
    fastclaw_protocol::MessageTarget::export_all(&cfg).unwrap();
    fastclaw_protocol::ToolKind::export_all(&cfg).unwrap();
    fastclaw_protocol::SubmissionId::export_all(&cfg).unwrap();
    fastclaw_protocol::Envelope::<fastclaw_protocol::AgentEvent>::export_all(&cfg).unwrap();
}
