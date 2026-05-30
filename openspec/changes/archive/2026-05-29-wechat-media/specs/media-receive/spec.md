## ADDED Requirements

### Requirement: Inbound media detection
The system SHALL detect media items in received `WeixinMessage.item_list` and categorize them by type (image, file, video, voice).

#### Scenario: Receive an image message
- **WHEN** an inbound message contains an `item_list` entry with `type=2` (IMAGE) and a valid `image_item`
- **THEN** the system identifies it as an image attachment and extracts the CDN reference

#### Scenario: Receive a file message
- **WHEN** an inbound message contains an `item_list` entry with `type=4` (FILE) and a valid `file_item`
- **THEN** the system identifies it as a file attachment and extracts the CDN reference and file name

### Requirement: Media download and decryption
The system SHALL download media from the WeChat CDN using the `encrypt_query_param` and decrypt using the provided AES key.

#### Scenario: Download and decrypt an image
- **WHEN** the system processes an inbound image with `encrypt_query_param` and `aes_key`
- **THEN** the system downloads the encrypted content from CDN, decrypts with AES-128-ECB using the base64-decoded `aes_key`, removes PKCS7 padding, and saves the plaintext to a local temporary file

#### Scenario: CDN download URL construction
- **WHEN** building the CDN download URL
- **THEN** the system constructs it from the configured CDN base URL plus the `encrypt_query_param`

### Requirement: InboundMessage attachment propagation
The `InboundMessage` SHALL include an `attachments` field containing local file paths of downloaded media, making them accessible to the agent's tool chain.

#### Scenario: Pass downloaded image to agent
- **WHEN** an inbound image has been downloaded and decrypted to `/tmp/wechat-media/img_123.png`
- **THEN** the `InboundMessage.attachments` MUST contain an entry with that local path and MIME type `image/png`

### Requirement: Temporary file cleanup
The system SHALL clean up downloaded media files older than 24 hours at startup.

#### Scenario: Cleanup on startup
- **WHEN** the WeChat plugin starts
- **THEN** files in the media temp directory (`~/.fastclaw-dev/data/wechat-media/`) older than 24 hours MUST be deleted
