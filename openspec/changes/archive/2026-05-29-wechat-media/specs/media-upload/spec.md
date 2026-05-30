## ADDED Requirements

### Requirement: AES-128-ECB file encryption
The system SHALL encrypt file content using AES-128-ECB with PKCS7 padding before uploading to the WeChat CDN. The AES key MUST be a randomly generated 16-byte key.

#### Scenario: Encrypt a file for CDN upload
- **WHEN** a file is prepared for CDN upload
- **THEN** the system generates a random 16-byte AES key, encrypts the file content with AES-128-ECB + PKCS7 padding, and produces the ciphertext with correct padded size

#### Scenario: Ciphertext size calculation
- **WHEN** computing the ciphertext size for `getUploadUrl`
- **THEN** the size MUST equal the plaintext size rounded up to the next 16-byte boundary (AES block size)

### Requirement: CDN pre-signed URL acquisition
The system SHALL call the `getUploadUrl` API endpoint to obtain CDN upload parameters before uploading any media file.

#### Scenario: Request upload URL for an image
- **WHEN** sending an image file with known size and MD5
- **THEN** the system sends a `getUploadUrl` request with `media_type=1` (IMAGE), `rawsize`, `rawfilemd5`, `filesize` (ciphertext), `no_need_thumb=true`, and the hex-encoded `aeskey`
- **AND** receives `upload_full_url` or `upload_param` in the response

#### Scenario: Request upload URL for a file attachment
- **WHEN** sending a non-image, non-video file
- **THEN** the system sends a `getUploadUrl` request with `media_type=3` (FILE) and receives upload parameters

### Requirement: CDN file upload
The system SHALL PUT the AES-encrypted file content to the CDN URL obtained from `getUploadUrl`.

#### Scenario: Upload encrypted content to CDN
- **WHEN** the system has encrypted file content and a CDN upload URL
- **THEN** the system performs an HTTP PUT with the encrypted bytes to the CDN URL
- **AND** receives an `encrypt_query_param` (download parameter) in the response for use in the message

#### Scenario: CDN upload URL resolution
- **WHEN** `upload_full_url` is present in the `getUploadUrl` response
- **THEN** the system uses `upload_full_url` directly
- **WHEN** only `upload_param` is present
- **THEN** the system constructs the URL from the configured CDN base URL + `upload_param`
