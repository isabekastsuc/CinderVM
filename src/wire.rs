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
