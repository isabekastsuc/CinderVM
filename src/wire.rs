//! The supervisor protocol (`wire`): a length-prefixed framing for the
//! control channel between the VM process and the Go supervisor.

use crate::diag::{Code, Diag, Result};

/// Protocol version this crate speaks.
pub const WIRE_VERSION: u8 = 1;

/// One framed message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// `[name]` — the VM announces the image it is running.
    Hello(String),
    /// `[seq, trap-kind]` — the VM reports a pending trap.
    Trap(u64, u8),
    /// `[seq, payload]` — the host returns an answer.
    Answer(u64, Vec<u8>),
    /// Snapshot checkpoint request.
    Snapshot,
    /// The supervisor asked the VM to stop.
    Shutdown,
}

/// Encode a message with a 4-byte length prefix and a version byte.
pub fn encode(m: &Message) -> Vec<u8> {
    let mut body = vec![WIRE_VERSION];
    match m {
        Message::Hello(name) => {
            body.push(0);
            body.extend_from_slice(&(name.len() as u32).to_le_bytes());
            body.extend_from_slice(name.as_bytes());
        }
        Message::Trap(seq, kind) => {
            body.push(1);
            body.extend_from_slice(&seq.to_le_bytes());
            body.push(*kind);
        }
        Message::Answer(seq, payload) => {
            body.push(2);
            body.extend_from_slice(&seq.to_le_bytes());
            body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            body.extend_from_slice(payload);
        }
        Message::Snapshot => body.push(3),
        Message::Shutdown => body.push(4),
    }
    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// Decode one frame; returns `None` when the buffer holds an incomplete
/// message.
pub fn decode(buf: &[u8]) -> Result<Option<(Message, usize)>> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_le_bytes(buf[..4].try_into().expect("4 bytes")) as usize;
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let body = &buf[4..4 + len];
    if body.first() != Some(&WIRE_VERSION) {
        return Err(Diag::new(Code::RecordMalformed, "wire version mismatch"));
    }
    let kind = body[1];
    let msg = match kind {
        0 => {
            let n = u32::from_le_bytes(body[2..6].try_into().expect("4 bytes")) as usize;
            let name = String::from_utf8(body[6..6 + n].to_vec())
                .map_err(|_| Diag::new(Code::RecordMalformed, "hello name not UTF-8"))?;
