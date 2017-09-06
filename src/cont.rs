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

/// Restore a machine from a snapshot. Validates every handle against the
/// arena and rejects live pendings.
pub fn restore(vm: &mut Vm, bytes: &[u8]) -> Result<()> {
    let mut c = Cursor::new(bytes);
    if c.take(4)? != SNAP_MAGIC {
        return Err(Diag::new(Code::SnapshotCorrupt, "bad snapshot magic"));
    }
    let version = u16::from_le_bytes(c.take(2)?.try_into().expect("2 bytes"));
    if version != SNAP_VERSION {
        return Err(Diag::new(
            Code::SnapshotVersion,
            format!("unsupported snapshot version {version}"),
        ));
    }
    let arena_len = u32::from_le_bytes(c.take(4)?.try_into().expect("4 bytes")) as usize;
    let arena = c.take(arena_len)?;
    vm.restore_arena(arena);

    let n_frames = u32::from_le_bytes(c.take(4)?.try_into().expect("4 bytes")) as usize;
    let mut frames = Vec::with_capacity(n_frames);
    for _ in 0..n_frames {
        let func = u32::from_le_bytes(c.take(4)?.try_into().expect("4 bytes"));
        let pc = u32::from_le_bytes(c.take(4)?.try_into().expect("4 bytes"));
        let n_slots = u32::from_le_bytes(c.take(4)?.try_into().expect("4 bytes")) as usize;
        let mut slots = Vec::with_capacity(n_slots);
        for _ in 0..n_slots {
            let v = Value::read(c.take(16)?)?;
            if v.tag() == Tag::Pending {
                return Err(Diag::new(
                    Code::LivePending,
                    "pending value in restored snapshot",
                ));
            }
            if v.tag().is_arena() {
                let h = v.as_handle()?;
