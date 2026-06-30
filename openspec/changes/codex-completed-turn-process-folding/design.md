## Context

`canonical-turn-timeline-ui` established the canonical timeline and turn-level transcript renderer. That solved live/replay ordering, removed legacy reconstruction from the normal UI path, and introduced assistant activity components for reasoning and tools.

The remaining product gap is presentation state. Codex App and ChatGPT keep live progress visible while work is happening, but once the answer is complete they collapse intermediate process into a small expandable row and make the final answer the primary reading path. XiaoLin currently still exposes too many activity rows as first-class content in completed turns, which makes the UI feel closer to a CLI log than an app conversation.

This change is frontend-focused and builds on existing `TurnDisplayNode[]` data. It should not add a new timeline model, migrate old sessions, or reintroduce legacy message reconstruction.

## Goals / Non-Goals

**Goals:**

- Show running reasoning and running tools in real time while the assistant turn is active.
- Collapse completed reasoning, completed tools, approvals, and process-only activity into a single turn-level process summary row by default after normal completion.
- Let users expand the process summary to inspect the full chronological activity transcript.
- Keep abnormal terminal information visible when a turn is cancelled, fails, hits a tool loop, or is interrupted.
- Align default layout with Codex App / ChatGPT: user prompt is compact, final assistant answer is primary, process is secondary, and raw implementation names are hidden by default.
- Cover live, completed, expanded, abnormal, and replay states with DOM/unit/E2E or visual regression tests.

**Non-Goals:**

- No backend storage migration.
- No compatibility migration for sessions without canonical timeline data.
- No new tool output backend or new output handle scheme.
- No changes to model-visible context projection.
- No diagnostic/debug panel redesign beyond preserving metadata needed for optional diagnostic rendering.

## Decisions

### Derive presentation from timeline nodes, not new persisted state

The UI SHALL derive a `TurnProcessPresentation` or equivalent view model from existing `TurnDisplayNode[]`, terminal status, and active/running state. Expanded/collapsed UI preference can remain local frontend state keyed by turn id.

Alternative considered: persist a new folded/completed presentation event. That would make replay more complex and duplicate information already available from the canonical timeline.

### Use turn lifecycle to switch presentation mode

While a turn is active, reasoning and activity rows remain visible in timeline order. Once the turn completes normally, process nodes move behind a single summary row. For abnormal endings, terminal status remains visible outside the fold when needed to explain the outcome.

Alternative considered: always show only the final answer and hide progress. That fails the real-time visibility requirement and makes long-running turns opaque.

### Keep final answer as the primary completed content

Completed assistant text remains visible as the main response body. Process-only assistant text, if explicitly classified or inferred by existing process activity boundaries, may be included in the folded transcript only when doing so does not hide the final answer or terminal failure context.

Alternative considered: fold every assistant text segment before the last one. That is too risky because some turns intentionally interleave answer content around tools.

### Expanded process transcript preserves chronology

The expanded process transcript SHALL render the same reasoning/tool/approval/status chronology represented by the canonical nodes. It may use semantic grouping for adjacent repetitive tools, but expansion must preserve order and expose individual details.

Alternative considered: show a separate grouped summary that loses exact order. That would make debugging live/replay mismatches and auditing tool behavior harder.

### UI language and visuals stay app-like

Default labels use user-facing summaries such as `已处理 28s`, `正在运行 1 条命令`, or `已运行 4 条命令`, not raw names like `subagent_get` or `Run git diff --stat`. Raw names remain available inside detail rows where useful.

Alternative considered: reuse current raw tool row labels in the folded summary. That keeps the UI noisy and does not match the target Codex App style.

## Risks / Trade-offs

- Process classification can hide useful context if too aggressive. Mitigation: only fold canonical process node kinds by default, keep abnormal terminal status visible, and cover mixed text/tool/text turns with tests.
- Running and completed states can diverge visually. Mitigation: use one presentation selector with explicit `active`, `completed`, and `abnormal` modes, and test live-to-completed transition.
- Expanded transcript can become large. Mitigation: keep existing virtualization/bounded rendering assumptions and only render expanded process details on demand.
- Existing screenshots may still show old sessions or legacy path output. Mitigation: regression tests must exercise canonical timeline sessions and verify legacy fallback does not drive the default transcript.
