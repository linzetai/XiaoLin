use std::collections::HashMap;

use tokio::sync::oneshot;

use xiaolin_protocol::approval::{ApprovalDecision, PendingAction};

/// Unified interaction port for all "task waiting on external input" scenarios.
///
/// Codex uses 4 separate `HashMap`s (`pending_approvals`, `pending_user_input`,
/// `pending_elicitations`, `pending_dynamic_tools`). We unify them into a single
/// map keyed by interaction ID, with enum variants for the different kinds.
#[derive(Default)]
pub struct TurnInteractionPort {
    pending: HashMap<String, PendingInteraction>,
}

enum PendingInteraction {
    Approval {
        tx: oneshot::Sender<ApprovalDecision>,
    },
    Answer {
        tx: oneshot::Sender<String>,
    },
}

/// Handle given to a turn task for requesting approvals and answers.
///
/// The turn task calls `request_approval` / `request_answer`, which inserts
/// a pending entry into the shared port and returns a receiver. The actor
/// loop later calls `resolve_*` to unblock the task.
#[derive(Clone)]
pub struct InteractionHandle {
    tx: tokio::sync::mpsc::UnboundedSender<InteractionRegistration>,
}

struct InteractionRegistration {
    id: String,
    kind: PendingInteraction,
}

/// Sender half used by the actor loop to register interactions from the turn task.
pub struct InteractionRegistrar {
    rx: tokio::sync::mpsc::UnboundedReceiver<InteractionRegistration>,
}

/// Create a linked pair of [`InteractionHandle`] (for the turn task) and
/// [`InteractionRegistrar`] (for the actor to drain registrations).
pub fn interaction_channel() -> (InteractionHandle, InteractionRegistrar) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    (InteractionHandle { tx }, InteractionRegistrar { rx })
}

impl InteractionHandle {
    /// Request approval from the user/guardian. Returns a future that resolves
    /// when the actor dispatches `ResolveApproval`.
    pub fn request_approval(
        &self,
        interaction_id: String,
        _action: &PendingAction,
    ) -> oneshot::Receiver<ApprovalDecision> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(InteractionRegistration {
            id: interaction_id,
            kind: PendingInteraction::Approval { tx },
        });
        rx
    }

    /// Request an answer to a question. Returns a future that resolves when
    /// the actor dispatches `ResolveAnswer`.
    pub fn request_answer(&self, interaction_id: String) -> oneshot::Receiver<String> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(InteractionRegistration {
            id: interaction_id,
            kind: PendingInteraction::Answer { tx },
        });
        rx
    }
}

impl InteractionRegistrar {
    /// Drain all pending registrations from the turn task into the port.
    /// Called by the actor loop before processing resolve operations.
    pub fn drain_into(&mut self, port: &mut TurnInteractionPort) {
        while let Ok(reg) = self.rx.try_recv() {
            port.pending.insert(reg.id, reg.kind);
        }
    }
}

impl TurnInteractionPort {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve a pending approval. Returns `true` if the interaction existed.
    pub fn resolve_approval(&mut self, interaction_id: &str, decision: ApprovalDecision) -> bool {
        if let Some(PendingInteraction::Approval { tx }) = self.pending.remove(interaction_id) {
            let _ = tx.send(decision);
            true
        } else {
            false
        }
    }

    /// Resolve a pending answer. Returns `true` if the interaction existed.
    pub fn resolve_answer(&mut self, interaction_id: &str, answer: String) -> bool {
        if let Some(PendingInteraction::Answer { tx }) = self.pending.remove(interaction_id) {
            let _ = tx.send(answer);
            true
        } else {
            false
        }
    }

    /// Cancel all pending interactions (e.g. on turn abort). Senders are
    /// dropped, causing receivers to get `RecvError`.
    pub fn cancel_all(&mut self) {
        self.pending.clear();
    }

    /// Number of pending interactions.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn approval_roundtrip() {
        let (handle, mut registrar) = interaction_channel();
        let mut port = TurnInteractionPort::new();

        let rx = handle.request_approval(
            "ap-1".into(),
            &PendingAction::ShellCommand {
                command: "ls".into(),
                cwd: "/tmp".into(),
            },
        );

        registrar.drain_into(&mut port);
        assert_eq!(port.pending_count(), 1);

        assert!(port.resolve_approval("ap-1", ApprovalDecision::Approved));
        let result = rx.await.unwrap();
        assert_eq!(result, ApprovalDecision::Approved);
        assert_eq!(port.pending_count(), 0);
    }

    #[tokio::test]
    async fn answer_roundtrip() {
        let (handle, mut registrar) = interaction_channel();
        let mut port = TurnInteractionPort::new();

        let rx = handle.request_answer("q-1".into());

        registrar.drain_into(&mut port);
        assert!(port.resolve_answer("q-1", "yes".into()));
        let result = rx.await.unwrap();
        assert_eq!(result, "yes");
    }

    #[test]
    fn resolve_unknown_returns_false() {
        let mut port = TurnInteractionPort::new();
        assert!(!port.resolve_approval("nonexistent", ApprovalDecision::Denied));
        assert!(!port.resolve_answer("nonexistent", "x".into()));
    }

    #[tokio::test]
    async fn cancel_all_drops_senders() {
        let (handle, mut registrar) = interaction_channel();
        let mut port = TurnInteractionPort::new();

        let rx = handle.request_approval(
            "ap-2".into(),
            &PendingAction::FileWrite {
                path: "/tmp/x".into(),
            },
        );

        registrar.drain_into(&mut port);
        port.cancel_all();
        assert_eq!(port.pending_count(), 0);

        assert!(rx.await.is_err());
    }
}
