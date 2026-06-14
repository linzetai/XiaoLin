## ADDED Requirements

### Requirement: Approval panel uses left border risk indicator
The approval panel SHALL display a left vertical border colored according to the risk level:
- `high`: red/danger color
- `medium`: amber/caution color
- `low`: green/safe color (or neutral)

The panel SHALL NOT use colored backgrounds, Shield icons, or rounded border boxes.

#### Scenario: High risk shows red left border
- **WHEN** an approval with `risk_level: "high"` is displayed
- **THEN** the panel has a left border with danger/red color and no background fill

#### Scenario: Medium risk shows amber left border
- **WHEN** an approval with `risk_level: "medium"` is displayed
- **THEN** the panel has a left border with amber/caution color

### Requirement: Intent-first title display
The approval panel SHALL display a natural-language question as its title that describes the intent of the action (e.g., "允许执行此命令？", "允许写入此文件？") rather than technical labels like "CAUTION" or "PERMISSION REQUIRED".

#### Scenario: Shell command shows execution question
- **WHEN** an approval for `ShellCommand` action is displayed
- **THEN** the title reads "允许执行此命令？" or similar intent question

#### Scenario: File write shows write question
- **WHEN** an approval for `FileWrite` action is displayed
- **THEN** the title reads "允许写入此文件？" or similar intent question

### Requirement: Command/path always visible without expand
The approval panel SHALL always display the command string (for shell) or file path (for file operations) in a monospace font without requiring user interaction to reveal it.

For `ShellCommand` actions, the panel SHALL also display the `cwd` (working directory) below the command in a smaller, secondary-colored font.

#### Scenario: Shell command visible immediately
- **WHEN** an approval for `ShellCommand { command: "npm install", cwd: "/project" }` is displayed
- **THEN** "npm install" is visible in monospace font without clicking any expand button
- **AND** "/project" is displayed below in smaller secondary text

#### Scenario: File path visible immediately
- **WHEN** an approval for `FileWrite { path: "/project/src/main.rs" }` is displayed
- **THEN** the path "/project/src/main.rs" is visible in monospace font

### Requirement: Decision options displayed as vertical list
The approval panel SHALL display decision options as a vertical list (one per line) rather than a horizontal row of buttons. Each option SHALL show its keyboard shortcut hint.

#### Scenario: Options rendered vertically
- **WHEN** an approval with 4 decisions is displayed
- **THEN** each decision appears on its own line with a shortcut key indicator

### Requirement: Keyboard shortcuts trigger decisions
The approval panel SHALL support keyboard shortcuts to trigger decisions directly:
- `y` key: Approved (allow once)
- `s` key: ApprovedForSession (allow for session)
- `p` key: ApprovedWithPolicyAmend (remember prefix, only when available)
- `n` key: Denied
- `a` key: Abort

`ApprovedAllForSession` SHALL NOT have a keyboard shortcut (high-risk operation requires deliberate click).

Shortcuts SHALL only fire when no input/textarea element has focus.

#### Scenario: Y key approves
- **WHEN** the approval panel is visible and user presses `y` key
- **THEN** the "approved" decision is sent

#### Scenario: P key triggers policy amend
- **WHEN** the approval panel is visible, the "remember prefix" option is available, and user presses `p` key
- **THEN** the "approved_with_policy_amend" decision is sent with the suggested prefix

#### Scenario: Shortcut ignored during text input
- **WHEN** the approval panel is visible but an input element has focus and user presses `y`
- **THEN** no decision is triggered

### Requirement: Content/diff preview expandable
When `content` or `diff` data is available in the action, the panel SHALL show a collapsible preview section below the command/path. It SHALL default to collapsed for content > 5 lines, expanded for content <= 5 lines.

#### Scenario: Short diff auto-expanded
- **WHEN** approval for ApplyPatch with 3-line diff is displayed
- **THEN** the diff preview is expanded by default

#### Scenario: Long content collapsed
- **WHEN** approval for FileWrite with 50-line content is displayed
- **THEN** the content preview is collapsed with a "显示内容" toggle

### Requirement: Submitted state disables interaction
Once a decision is submitted, the panel SHALL visually indicate the chosen decision and disable all buttons and keyboard shortcuts.

#### Scenario: After approval submitted
- **WHEN** user clicks "允许" or presses `y`
- **THEN** the panel shows which decision was made and all other options are disabled

### Requirement: Remember prefix option displayed conditionally
When `available_decisions` includes an `ApprovedWithPolicyAmend` variant (which carries a pre-filled `prefix`), the panel SHALL display a "记住此命令前缀" option showing the prefix extracted from the variant (e.g., "[P] 记住「npm」前缀，以后自动允许").

#### Scenario: Prefix option shown for medium-risk shell
- **WHEN** approval `available_decisions` contains `ApprovedWithPolicyAmend { prefix: ["npm"] }`
- **THEN** the panel shows an option labeled like "[P] 记住「npm」前缀，以后自动允许"

#### Scenario: Prefix option hidden when not available
- **WHEN** approval `available_decisions` does NOT contain `ApprovedWithPolicyAmend`
- **THEN** no "记住前缀" option is shown
