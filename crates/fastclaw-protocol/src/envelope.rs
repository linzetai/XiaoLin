use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::id::SubmissionId;

/// Transport-agnostic envelope wrapping a typed payload with a correlation ID.
///
/// Used for both requests (`Envelope<ClientOp>`) and event pushes
/// (`Envelope<AgentEvent>`).  The `id` field correlates a submission with
/// its resulting events, mirroring the Codex `Submission.id` / `Event.id`
/// pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Envelope<T> {
    pub id: SubmissionId,
    #[serde(flatten)]
    pub payload: T,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_serde_roundtrip() {
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        struct Ping {
            msg: String,
        }

        let env = Envelope {
            id: SubmissionId::new("sub-1"),
            payload: Ping {
                msg: "hello".into(),
            },
        };
        let json = serde_json::to_value(&env).unwrap();
        assert_eq!(json["id"], "sub-1");
        assert_eq!(json["msg"], "hello");

        let back: Envelope<Ping> = serde_json::from_value(json).unwrap();
        assert_eq!(back.id, env.id);
        assert_eq!(back.payload, env.payload);
    }
}
