## ADDED Requirements

### Requirement: OutboundMessage attachment support
The `OutboundMessage` struct SHALL include an `attachments` field containing a list of file attachments, each with a local file path and optional MIME type.

#### Scenario: Agent sends a message with an image attachment
- **WHEN** the agent produces an OutboundMessage with an attachment whose path points to a PNG file
- **THEN** the attachment MUST include `file_path` and the MIME type MUST be inferred as `image/png`

#### Scenario: Agent sends a plain text message
- **WHEN** the agent produces an OutboundMessage with no attachments
- **THEN** the `attachments` field MUST be an empty list and message sending proceeds as text-only

### Requirement: Image message construction
The system SHALL construct a `WeixinMessage` with `item_list` containing an `image_item` when the attachment has an `image/*` MIME type.

#### Scenario: Build image message item
- **WHEN** an attachment with MIME type `image/png` is uploaded to CDN
- **THEN** the system constructs a `MessageItem` with `type=2` (IMAGE) containing an `image_item` with CDN media reference (`encrypt_query_param`, `aes_key` as base64) and file size fields

### Requirement: File message construction
The system SHALL construct a `WeixinMessage` with `item_list` containing a `file_item` when the attachment has a non-image, non-video MIME type.

#### Scenario: Build file message item
- **WHEN** an attachment with MIME type `application/pdf` is uploaded to CDN
- **THEN** the system constructs a `MessageItem` with `type=4` (FILE) containing a `file_item` with the file name, CDN media reference, and file size

### Requirement: MIME type routing
The system SHALL route attachments to the correct message item type based on MIME type inference from the file extension.

#### Scenario: Route by file extension
- **WHEN** the attachment file path ends with `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, or `.bmp`
- **THEN** the system routes to `image_item` (type=2)
- **WHEN** the file path ends with any other extension
- **THEN** the system routes to `file_item` (type=4)

### Requirement: Mixed text and media message
The system SHALL support sending a message that contains both text and media attachments in the same `item_list`.

#### Scenario: Text with image
- **WHEN** an OutboundMessage has both `text` content and an image attachment
- **THEN** the `item_list` MUST contain a `text_item` (type=1) followed by an `image_item` (type=2)
