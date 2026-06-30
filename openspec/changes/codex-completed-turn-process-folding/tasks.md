## 1. Presentation Model

- [x] 1.1 Audit existing `TurnDisplayNode` kinds and assistant presentation items to classify nodes as answer content, live process, folded process, terminal status, or diagnostic-only metadata.
- [x] 1.2 Add a turn presentation selector/view model that derives `active`, `completed`, and `abnormal` presentation modes from canonical timeline nodes and terminal status.
- [x] 1.3 Add local expanded/collapsed state keyed by turn id for completed process summaries without persisting UI-only state into the timeline.
- [x] 1.4 Preserve canonical node order inside the expanded process transcript, including reasoning-tool-reasoning and text-tool-text cases.

## 2. Completed Process Folding UI

- [x] 2.1 Update `AssistantResponseBlock` or equivalent turn renderer to show live reasoning/tool rows while a turn is active.
- [x] 2.2 Render a single completed process summary row after normal completion when folded process nodes exist.
- [x] 2.3 Render the final assistant answer as primary completed content, without folding answer text merely because tools occurred earlier.
- [x] 2.4 Implement expand/collapse behavior that reveals folded reasoning, tool activity, approvals, and process statuses in chronological order.
- [x] 2.5 Keep abnormal terminal status visible outside the folded process summary when the turn fails, is cancelled, is interrupted, or ends with tool-loop/budget diagnosis.

## 3. Reasoning, Tools, And Iteration Details

- [x] 3.1 Update `ReasoningBlock` usage so active reasoning remains visible live and completed reasoning appears inside the expanded process transcript by default.
- [x] 3.2 Update tool activity grouping so completed tools fold into the turn summary while running tools remain visible.
- [x] 3.3 Ensure semantic activity titles are used in summaries and expanded rows, with raw tool names only as secondary detail.
- [x] 3.4 Keep iteration boundaries hidden in default completed view and expanded user-facing process transcripts unless diagnostic mode is explicitly enabled.
- [x] 3.5 Ensure adjacent repetitive tool groups preserve individual inspectable details and distinct activity families remain separate.

## 4. Layout And Codex App Alignment

- [x] 4.1 Verify user prompt rendering uses compact Codex App / ChatGPT-like treatment and does not wrap short single-line messages into multiple lines.
- [x] 4.2 Align spacing, typography, summary row affordance, and process transcript density with the Codex App reference screenshots.
- [x] 4.3 Remove or quarantine any unused legacy UI paths or dead presentation code introduced while iterating on the previous timeline UI.
- [x] 4.4 Ensure old sessions without canonical timeline data keep the existing unsupported/legacy behavior and do not re-enable message reconstruction.

## 5. Tests

- [x] 5.1 Add selector tests for active, normally completed, abnormal, folded, expanded, and replayed presentation modes.
- [x] 5.2 Add DOM tests proving active reasoning/tools are visible while running and folded after normal completion.
- [x] 5.3 Add DOM tests proving expanded process transcripts preserve chronology for `reasoning -> tool -> reasoning -> text` and `text -> tool -> text`.
- [x] 5.4 Add tests proving abnormal terminal status remains visible outside the folded process summary.
- [x] 5.5 Add tests proving iteration boundaries and raw tool implementation names are hidden from the default user-facing transcript.
- [x] 5.6 Add regression coverage for short single-line user prompts so they do not wrap into one word or one phrase per line.

## 6. Verification

- [x] 6.1 Run targeted message-stream and timeline selector tests.
- [x] 6.2 Run `cd crates/xiaolin-app && pnpm test`.
- [x] 6.3 Run `cd crates/xiaolin-app && pnpm build`.
- [x] 6.4 Run `cd crates/xiaolin-app && pnpm test:e2e`.
- [x] 6.5 Capture or update visual/DOM evidence comparing running, completed folded, completed expanded, and abnormal turn states against the Codex App reference behavior.
