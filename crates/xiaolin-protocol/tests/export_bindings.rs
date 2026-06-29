#![cfg(feature = "ts")]

use ts_rs::TS;

#[test]
fn export_all_bindings() {
    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("generated");
    std::fs::create_dir_all(&out_dir).unwrap();
    std::env::set_var("TS_RS_EXPORT_DIR", &out_dir);
    let cfg = ts_rs::Config::from_env();

    // Core types — export_all cascades to nested dependencies.
    xiaolin_protocol::AgentEvent::export_all(&cfg).unwrap();
    xiaolin_protocol::ClientOp::export_all(&cfg).unwrap();
    xiaolin_protocol::HistoryItem::export_all(&cfg).unwrap();
    xiaolin_protocol::TokenUsage::export_all(&cfg).unwrap();
    xiaolin_protocol::TurnSummary::export_all(&cfg).unwrap();
    xiaolin_protocol::ApprovalDecision::export_all(&cfg).unwrap();
    xiaolin_protocol::PendingAction::export_all(&cfg).unwrap();
    xiaolin_protocol::ToolDefinition::export_all(&cfg).unwrap();

    // Types not reachable from the roots above.
    xiaolin_protocol::MessageTarget::export_all(&cfg).unwrap();
    xiaolin_protocol::ToolKind::export_all(&cfg).unwrap();
    xiaolin_protocol::SubmissionId::export_all(&cfg).unwrap();
    xiaolin_protocol::Envelope::<xiaolin_protocol::AgentEvent>::export_all(&cfg).unwrap();

    // Timeline types
    xiaolin_protocol::TurnTimelineEvent::export_all(&cfg).unwrap();
    xiaolin_protocol::TurnDisplayNode::export_all(&cfg).unwrap();
    xiaolin_protocol::TimelineEventType::export_all(&cfg).unwrap();
    xiaolin_protocol::TimelineEventId::export_all(&cfg).unwrap();
}
