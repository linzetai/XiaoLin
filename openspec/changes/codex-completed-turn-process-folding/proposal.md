## Why

The current timeline-backed transcript fixes ordering and replay, but completed turns still expose too much process detail as primary chat content. Codex App and ChatGPT-style conversations should show live progress while the turn is running, then collapse intermediate reasoning and tool activity after completion so the final answer becomes the primary reading path.

## What Changes

- Add a completed-turn presentation policy that folds reasoning, tool activity, approvals, and process-only activity into a single expandable processing summary once a turn finishes normally.
- Keep running turns transparent: active reasoning and currently running tools remain visible in chronological order with clear status while the agent is still working.
- Preserve full chronology inside the expandable process transcript so users can inspect the exact reasoning/tool sequence after completion.
- Keep abnormal terminal states visible enough to explain cancellation, error, tool loop, or interruption instead of hiding important failure context inside the folded process.
- Align user and assistant message layout with Codex App / ChatGPT expectations: concise user prompt treatment, final assistant answer as the main content, and no default iteration labels or raw implementation tool names.
- Do not migrate pre-change/old sessions and do not introduce a new backend timeline model; this change builds on the canonical timeline and display nodes already defined.

## Capabilities

### New Capabilities

- `completed-turn-process-folding`: Defines the turn-level policy for live progress visibility, completed-turn process folding, expansion behavior, abnormal terminal visibility, and final-answer emphasis.

### Modified Capabilities

- `codex-message-stream-ui`: Updates assistant response rendering requirements so the default completed view folds process activity and emphasizes final answers.
- `codex-reasoning-block`: Updates reasoning display requirements so completed reasoning is represented inside the folded process transcript by default while active reasoning remains visible during streaming.
- `tool-step-display`: Updates tool activity requirements so completed tool rows are folded into the turn process summary by default, with semantic summaries and expandable raw details.
- `codex-iteration-divider`: Clarifies that iteration boundaries remain hidden in both default completed views and expanded user-facing process transcripts unless a diagnostic mode is enabled.

## Impact

- Frontend transcript components under `crates/xiaolin-app/src/components/message-stream`, especially assistant response presentation, reasoning blocks, tool/activity groups, turn blocks, and tests.
- Timeline selectors and presentation derivation under `crates/xiaolin-app/src/lib/timeline` where turn completion state and process groups are derived.
- Frontend unit, DOM, and E2E/visual regression coverage for running turns, completed turns, expanded process transcript, abnormal terminal states, and replay equivalence.
- No expected protocol migration, storage migration, or old-session compatibility work.
