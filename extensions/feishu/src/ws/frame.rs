//! Protobuf Frame codec for the Feishu WebSocket long-connection protocol.
//!
//! Wire format matches `pbbp2.proto` (proto2) used by all official Feishu SDKs.
//! We implement manual encode/decode with `prost` primitives to avoid a build-time
//! protoc dependency while keeping binary compatibility.

use prost::encoding::{self, WireType};

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct Header {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Default)]
pub struct Frame {
    pub seq_id: u64,              // field 1  required uint64
    pub log_id: u64,              // field 2  required uint64
    pub service: i32,             // field 3  required int32
    pub method: i32,              // field 4  required int32
    pub headers: Vec<Header>,     // field 5  repeated Header
    pub payload_encoding: String, // field 6  optional string
    pub payload_type: String,     // field 7  optional string
    pub payload: Vec<u8>,         // field 8  optional bytes
    pub log_id_new: String,       // field 9  optional string
}

// Frame types
pub const FRAME_TYPE_CONTROL: i32 = 0;
pub const FRAME_TYPE_DATA: i32 = 1;

// Message types (header "type" values)
pub const MSG_TYPE_EVENT: &str = "event";
pub const MSG_TYPE_CARD: &str = "card";
pub const MSG_TYPE_PING: &str = "ping";
pub const MSG_TYPE_PONG: &str = "pong";

// Header keys
pub const HEADER_TYPE: &str = "type";
pub const HEADER_MESSAGE_ID: &str = "message_id";
pub const HEADER_SUM: &str = "sum";
pub const HEADER_SEQ: &str = "seq";
pub const HEADER_TRACE_ID: &str = "trace_id";
pub const HEADER_BIZ_RT: &str = "biz_rt";

impl Frame {
    pub fn new_ping(service_id: i32) -> Self {
        Self {
            method: FRAME_TYPE_CONTROL,
            service: service_id,
            headers: vec![Header {
                key: HEADER_TYPE.into(),
                value: MSG_TYPE_PING.into(),
            }],
            ..Default::default()
        }
    }

    pub fn get_header(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
    }

    pub fn set_header(&mut self, key: &str, value: &str) {
        if let Some(h) = self.headers.iter_mut().find(|h| h.key == key) {
            h.value = value.to_string();
        } else {
            self.headers.push(Header {
                key: key.to_string(),
                value: value.to_string(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Manual Protobuf encode / decode (proto2 wire-compatible with pbbp2.proto)
// ---------------------------------------------------------------------------

impl Header {
    fn encoded_len(&self) -> usize {
        let mut len = 0;
        len += encoding::string::encoded_len(1, &self.key);
        len += encoding::string::encoded_len(2, &self.value);
        len
    }

    fn encode_raw(&self, buf: &mut Vec<u8>) {
        encoding::string::encode(1, &self.key, buf);
        encoding::string::encode(2, &self.value, buf);
    }
}

impl Frame {
    pub fn encode_to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.encoded_len());
        self.encode_raw(&mut buf);
        buf
    }

    fn encoded_len(&self) -> usize {
        let mut len = 0;
        len += encoding::uint64::encoded_len(1, &self.seq_id);
        len += encoding::uint64::encoded_len(2, &self.log_id);
        len += encoding::int32::encoded_len(3, &self.service);
        len += encoding::int32::encoded_len(4, &self.method);
        for h in &self.headers {
            let inner = h.encoded_len();
            len += encoding::key_len(5) + encoding::encoded_len_varint(inner as u64) + inner;
        }
        if !self.payload_encoding.is_empty() {
            len += encoding::string::encoded_len(6, &self.payload_encoding);
        }
        if !self.payload_type.is_empty() {
            len += encoding::string::encoded_len(7, &self.payload_type);
        }
        if !self.payload.is_empty() {
            len += encoding::bytes::encoded_len(8, &self.payload);
        }
        if !self.log_id_new.is_empty() {
            len += encoding::string::encoded_len(9, &self.log_id_new);
        }
        len
    }

    fn encode_raw(&self, buf: &mut Vec<u8>) {
        encoding::uint64::encode(1, &self.seq_id, buf);
        encoding::uint64::encode(2, &self.log_id, buf);
        encoding::int32::encode(3, &self.service, buf);
        encoding::int32::encode(4, &self.method, buf);
        for h in &self.headers {
            encoding::encode_key(5, WireType::LengthDelimited, buf);
            let inner_len = h.encoded_len();
            encoding::encode_varint(inner_len as u64, buf);
            h.encode_raw(buf);
        }
        if !self.payload_encoding.is_empty() {
            encoding::string::encode(6, &self.payload_encoding, buf);
        }
        if !self.payload_type.is_empty() {
            encoding::string::encode(7, &self.payload_type, buf);
        }
        if !self.payload.is_empty() {
            encoding::bytes::encode(8, &self.payload, buf);
        }
        if !self.log_id_new.is_empty() {
            encoding::string::encode(9, &self.log_id_new, buf);
        }
    }

    pub fn decode(data: &[u8]) -> anyhow::Result<Self> {
        let mut frame = Frame::default();
        let mut cursor = data;
        while !cursor.is_empty() {
            let (field_number, wire_type) = encoding::decode_key(&mut cursor)?;
            match field_number {
                1 => {
                    let mut val = 0u64;
                    encoding::uint64::merge(wire_type, &mut val, &mut cursor, Default::default())?;
                    frame.seq_id = val;
                }
                2 => {
                    let mut val = 0u64;
                    encoding::uint64::merge(wire_type, &mut val, &mut cursor, Default::default())?;
                    frame.log_id = val;
                }
                3 => {
                    let mut val = 0i32;
                    encoding::int32::merge(wire_type, &mut val, &mut cursor, Default::default())?;
                    frame.service = val;
                }
                4 => {
                    let mut val = 0i32;
                    encoding::int32::merge(wire_type, &mut val, &mut cursor, Default::default())?;
                    frame.method = val;
                }
                5 => {
                    // Length-delimited sub-message
                    if wire_type != WireType::LengthDelimited {
                        anyhow::bail!("unexpected wire type for field 5");
                    }
                    let len = encoding::decode_varint(&mut cursor)? as usize;
                    if cursor.len() < len {
                        anyhow::bail!("truncated header sub-message");
                    }
                    let sub = &cursor[..len];
                    cursor = &cursor[len..];
                    let h = decode_header(sub)?;
                    frame.headers.push(h);
                }
                6 => {
                    encoding::string::merge(
                        wire_type,
                        &mut frame.payload_encoding,
                        &mut cursor,
                        Default::default(),
                    )?;
                }
                7 => {
                    encoding::string::merge(
                        wire_type,
                        &mut frame.payload_type,
                        &mut cursor,
                        Default::default(),
                    )?;
                }
                8 => {
                    encoding::bytes::merge(
                        wire_type,
                        &mut frame.payload,
                        &mut cursor,
                        Default::default(),
                    )?;
                }
                9 => {
                    encoding::string::merge(
                        wire_type,
                        &mut frame.log_id_new,
                        &mut cursor,
                        Default::default(),
                    )?;
                }
                _ => {
                    encoding::skip_field(wire_type, field_number, &mut cursor, Default::default())?;
                }
            }
        }
        Ok(frame)
    }
}

fn decode_header(data: &[u8]) -> anyhow::Result<Header> {
    let mut h = Header::default();
    let mut cursor = data;
    while !cursor.is_empty() {
        let (field_number, wire_type) = encoding::decode_key(&mut cursor)?;
        match field_number {
            1 => encoding::string::merge(wire_type, &mut h.key, &mut cursor, Default::default())?,
            2 => encoding::string::merge(wire_type, &mut h.value, &mut cursor, Default::default())?,
            _ => encoding::skip_field(wire_type, field_number, &mut cursor, Default::default())?,
        }
    }
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ping() {
        let frame = Frame::new_ping(42);
        let bytes = frame.encode_to_vec();
        let decoded = Frame::decode(&bytes).unwrap();
        assert_eq!(decoded.method, FRAME_TYPE_CONTROL);
        assert_eq!(decoded.service, 42);
        assert_eq!(decoded.get_header(HEADER_TYPE), Some(MSG_TYPE_PING));
    }

    #[test]
    fn roundtrip_data_frame() {
        let mut frame = Frame {
            seq_id: 100,
            log_id: 200,
            service: 1,
            method: FRAME_TYPE_DATA,
            payload: b"{\"event\":\"test\"}".to_vec(),
            ..Default::default()
        };
        frame.set_header(HEADER_TYPE, MSG_TYPE_EVENT);
        frame.set_header(HEADER_MESSAGE_ID, "msg_001");
        frame.set_header(HEADER_TRACE_ID, "trace_001");
        frame.set_header(HEADER_SUM, "1");
        frame.set_header(HEADER_SEQ, "0");

        let bytes = frame.encode_to_vec();
        let decoded = Frame::decode(&bytes).unwrap();
        assert_eq!(decoded.seq_id, 100);
        assert_eq!(decoded.log_id, 200);
        assert_eq!(decoded.service, 1);
        assert_eq!(decoded.method, FRAME_TYPE_DATA);
        assert_eq!(decoded.get_header(HEADER_TYPE), Some(MSG_TYPE_EVENT));
        assert_eq!(decoded.get_header(HEADER_MESSAGE_ID), Some("msg_001"));
        assert_eq!(decoded.payload, b"{\"event\":\"test\"}");
    }
}
