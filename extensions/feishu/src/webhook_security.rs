use std::collections::BTreeMap;

use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
use base64::Engine;
use sha2::{Digest, Sha256};

type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

const SIGNATURE_MAX_AGE_SECS: i64 = 300;

/// Case-insensitive HTTP header lookup.
pub fn header_value<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    if let Some(v) = headers.get(name) {
        return Some(v.as_str());
    }
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Verify Feishu/Lark event subscription signature.
///
/// Algorithm: `hex(SHA256(timestamp + nonce + encrypt_key + raw_body))`
pub fn verify_lark_signature(
    timestamp: &str,
    nonce: &str,
    encrypt_key: &str,
    raw_body: &[u8],
    signature: &str,
) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(timestamp.as_bytes());
    hasher.update(nonce.as_bytes());
    hasher.update(encrypt_key.as_bytes());
    hasher.update(raw_body);
    let expected = hex::encode(hasher.finalize());
    constant_time_eq(expected.as_bytes(), signature.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Reject timestamps outside the ±5 minute replay window.
pub fn validate_timestamp(timestamp: &str) -> anyhow::Result<()> {
    let ts: i64 = timestamp
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid X-Lark-Request-Timestamp"))?;
    let now = chrono::Utc::now().timestamp();
    if (now - ts).abs() > SIGNATURE_MAX_AGE_SECS {
        anyhow::bail!("webhook timestamp outside allowed window");
    }
    Ok(())
}

/// Decrypt an encrypted Feishu event payload (`{"encrypt":"..."}`).
///
/// AES-256-CBC, key = SHA256(encrypt_key), IV = first 16 bytes of ciphertext.
pub fn decrypt_event_body(encrypt_key: &str, encrypted_b64: &str) -> anyhow::Result<Vec<u8>> {
    let key = Sha256::digest(encrypt_key.as_bytes());
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(encrypted_b64)
        .map_err(|_| anyhow::anyhow!("invalid encrypted payload encoding"))?;
    if ciphertext.len() <= 16 {
        anyhow::bail!("encrypted payload too short");
    }
    let (iv, data) = ciphertext.split_at(16);
    let mut buf = data.to_vec();
    let cipher = Aes256CbcDec::new_from_slices(&key, iv)
        .map_err(|_| anyhow::anyhow!("invalid encryption key material"))?;
    let decrypted = cipher
        .decrypt_padded_mut::<NoPadding>(&mut buf)
        .map_err(|_| anyhow::anyhow!("event decryption failed"))?;
    let pad = decrypted
        .last()
        .copied()
        .filter(|&b| b > 0 && (b as usize) <= decrypted.len())
        .unwrap_or(0);
    if pad == 0 || !decrypted[decrypted.len() - pad as usize..].iter().all(|&b| b == pad) {
        return Ok(decrypted.to_vec());
    }
    Ok(decrypted[..decrypted.len() - pad as usize].to_vec())
}

/// Parse webhook body, decrypting when `encrypt_key` is configured and body is encrypted.
pub fn parse_webhook_payload(
    encrypt_key: Option<&str>,
    raw_body: &[u8],
) -> anyhow::Result<serde_json::Value> {
    let outer: serde_json::Value = serde_json::from_slice(raw_body)
        .map_err(|e| anyhow::anyhow!("invalid webhook payload: {e}"))?;

    if let Some(enc) = outer.get("encrypt").and_then(|v| v.as_str()) {
        let key = encrypt_key.filter(|k| !k.is_empty()).ok_or_else(|| {
            anyhow::anyhow!("encrypted webhook received but encrypt_key is not configured")
        })?;
        let plaintext = decrypt_event_body(key, enc)?;
        return serde_json::from_slice(&plaintext)
            .map_err(|e| anyhow::anyhow!("invalid decrypted webhook payload: {e}"));
    }

    Ok(outer)
}

/// Verify signature headers when `encrypt_key` is configured; otherwise warn and skip.
pub fn verify_lark_webhook_headers(
    headers: &BTreeMap<String, String>,
    encrypt_key: Option<&str>,
    raw_body: &[u8],
) -> anyhow::Result<()> {
    let Some(key) = encrypt_key.filter(|k| !k.is_empty()) else {
        tracing::warn!(
            "feishu: encrypt_key not configured — skipping signature verification, using token check only"
        );
        return Ok(());
    };

    let timestamp = header_value(headers, "X-Lark-Request-Timestamp")
        .ok_or_else(|| anyhow::anyhow!("missing X-Lark-Request-Timestamp"))?;
    let nonce = header_value(headers, "X-Lark-Request-Nonce")
        .ok_or_else(|| anyhow::anyhow!("missing X-Lark-Request-Nonce"))?;
    let signature = header_value(headers, "X-Lark-Signature")
        .ok_or_else(|| anyhow::anyhow!("missing X-Lark-Signature"))?;

    validate_timestamp(timestamp)?;

    if !verify_lark_signature(timestamp, nonce, key, raw_body, signature) {
        anyhow::bail!("webhook signature mismatch");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_signature_matches_official_algorithm() {
        let timestamp = "1599360473";
        let nonce = "nonce";
        let encrypt_key = "test_key";
        let body = br#"{"encrypt":"abc"}"#;
        let mut hasher = Sha256::new();
        hasher.update(timestamp.as_bytes());
        hasher.update(nonce.as_bytes());
        hasher.update(encrypt_key.as_bytes());
        hasher.update(body);
        let sig = hex::encode(hasher.finalize());
        assert!(verify_lark_signature(timestamp, nonce, encrypt_key, body, &sig));
        assert!(!verify_lark_signature(timestamp, nonce, encrypt_key, body, "deadbeef"));
    }

    #[test]
    fn timestamp_outside_window_rejected() {
        let old = (chrono::Utc::now().timestamp() - 400).to_string();
        assert!(validate_timestamp(&old).is_err());
    }

    #[test]
    fn parse_plain_json_without_encrypt_key() {
        let body = br#"{"type":"url_verification","challenge":"abc"}"#;
        let v = parse_webhook_payload(None, body).unwrap();
        assert_eq!(v.get("challenge").and_then(|c| c.as_str()), Some("abc"));
    }
}
