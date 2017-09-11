//! Continuations: snapshot and restore of a running machine.

use crate::diag::{Code, Diag, Result};
use crate::frame::Frame;
use crate::interp::Vm;
use crate::value::{Tag, Value};

const SNAP_MAGIC: &[u8; 4] = b"CSNP";
const SNAP_VERSION: u16 = 1;

/// Serialize a running machine. Two memcpys and a hash for the arena plus a
/// frame walk; no graph traversal.
pub fn snapshot(vm: &Vm) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(SNAP_MAGIC);
    out.extend_from_slice(&SNAP_VERSION.to_le_bytes());

    let arena = vm.arena_bytes();
    out.extend_from_slice(&(arena.len() as u32).to_le_bytes());
    out.extend_from_slice(arena);

    let frames = vm.frames_snapshot();
    out.extend_from_slice(&(frames.len() as u32).to_le_bytes());
    for f in &frames {
        out.extend_from_slice(&f.func.to_le_bytes());
        out.extend_from_slice(&f.pc.to_le_bytes());
        out.extend_from_slice(&(f.slots.len() as u32).to_le_bytes());
        for v in &f.slots {
            v.write(&mut out);
        }
        out.extend_from_slice(&(f.sp as u32).to_le_bytes());
        out.extend_from_slice(&f.maxstack.to_le_bytes());
        out.push(f.returns);
    }
    Ok(out)
}
