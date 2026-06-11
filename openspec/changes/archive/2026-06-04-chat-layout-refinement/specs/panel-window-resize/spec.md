## ADDED Requirements

### Requirement: Panel open triggers window width expansion
When WorkspacePanel opens, the system SHALL increase the Tauri window width by `--panel-w` (360px) via `window.setSize()`, so the chat area width remains unchanged.

#### Scenario: Panel opens on non-maximized window
- **WHEN** user opens the WorkspacePanel and the window is not maximized
- **THEN** the window width increases by 360px to the right, chat area width remains the same as before panel opened

#### Scenario: Panel closes restores window width
- **WHEN** user closes the WorkspacePanel
- **THEN** the window width decreases by 360px (or restores to pre-panel-open width if user manually resized)

### Requirement: Screen boundary detection
The system SHALL detect available screen space before expanding the window. If expanding would cause the window to exceed screen bounds, the system SHALL fall back to internal compression mode.

#### Scenario: Insufficient screen space on the right
- **WHEN** user opens the panel but the window right edge plus 360px exceeds the monitor's available width
- **THEN** the system does not resize the window and the panel opens using internal space (current behavior)

### Requirement: Maximized window fallback
When the window is maximized or fullscreen, the system SHALL NOT attempt to resize. The panel SHALL use internal space from the chat area.

#### Scenario: Panel opens while window is maximized
- **WHEN** user opens the panel while the window is maximized
- **THEN** the window remains maximized and the panel occupies space from the chat area (with minWidth protection)

### Requirement: Non-Tauri environment fallback
In browser mode (non-Tauri), the system SHALL NOT attempt window resize and SHALL use internal compression mode.

#### Scenario: Panel opens in browser mode
- **WHEN** user opens the panel in a browser (non-Tauri) environment
- **THEN** the panel opens using internal space without any window resize attempt

### Requirement: Pre-panel width memory
The system SHALL store the window width before panel open. On panel close, it SHALL restore to the stored width rather than simply subtracting 360px.

#### Scenario: User resizes window while panel is open
- **WHEN** user manually resizes the window while the panel is open, then closes the panel
- **THEN** the window width is set to the pre-panel-open width (not current width minus 360px)
