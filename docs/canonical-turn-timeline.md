# Canonical Turn Timeline

The chat transcript UI is backed by the canonical turn timeline.

UI-visible live updates, reconnect recovery, and session replay must render from
`TurnTimelineEvent` or backend-materialized `TurnDisplayNode` data. The frontend
must not reconstruct transcript state from legacy message fields such as
`toolCallsJson`, `reasoningContent`, or `segmentOrder`.

Legacy message and history storage can still exist for backend internals and
model-context projection, but those records are not the UI source of truth. If a
pre-change development session has no timeline data, the UI shows an
unsupported-history notice instead of attempting migration or reconstruction.

Small tool outputs should be included inline in `ToolStepNode.output_preview`
when they satisfy the display policy. Large outputs should use the existing
session-scoped `ToolOutputHandle` detail reference and the read-only UI detail
endpoint; the transcript should not fetch details merely to replay small output.
