//! The VM's value representation and heap handles.
//!
//! # Why 16 bytes and `Copy`
//!
//! Every operand in the machine is a [`Value`]: a one-byte tag, seven bytes of
//! padding, and eight bytes of payload. It is `Copy`, contains no host pointer,
//! and has no `Drop`. That is what makes `cont::snapshot` a memcpy of the
//! operand array instead of a graph traversal — see the module docs in
//! `cont.rs` for the full argument.
//!
//! The cost is that anything larger than eight bytes lives in the arena and is
//! referenced by a [`Handle`], an arena-relative offset. Handles are *not*
//! pointers: relocating an arena is arithmetic on the base, so a restored
//! snapshot needs no pointer patching.

use core::fmt;

use crate::diag::{Code, Diag, Result};
use crate::isa::Ty;

/// Discriminant of a [`Value`]. Encoded as the first byte of a serialized
/// operand slot, so the numbering is part of the snapshot format and additions
/// go at the end.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum Tag {
    /// Absent operand. Only appears in unused frame slots; the verifier
    /// guarantees no instruction ever observes one.
    Void = 0,
    Int = 1,
    /// Arena-resident UTF-8.
    Str = 2,
    /// Arena-resident opaque bytes.
    Bytes = 3,
    /// Arena-resident array of [`Value`].
    List = 4,
    /// An in-flight effect. The payload is an effect id assigned by the host and
    /// recorded in the journal, not a pointer to anything.
    Pending = 5,
}

impl Tag {
    #[must_use]
    pub fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            0 => Self::Void,
            1 => Self::Int,
            2 => Self::Str,
            3 => Self::Bytes,
            4 => Self::List,
            5 => Self::Pending,
            _ => return None,
        })
    }

    /// The static type the verifier assigns to values with this tag.
    #[must_use]
    pub const fn ty(self) -> Ty {
        match self {
            Self::Void => Ty::Bottom,
            Self::Int => Ty::Int,
            Self::Str => Ty::Str,
            Self::Bytes => Ty::Bytes,
            Self::List => Ty::List,
            Self::Pending => Ty::Pending,
        }
    }

    /// Whether the payload is an arena handle rather than an immediate. Restore
    /// validation checks exactly these tags' payloads against the arena extent.
    #[must_use]
    pub const fn is_arena(self) -> bool {
        matches!(self, Self::Str | Self::Bytes | Self::List)
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Void => "void",
            Self::Int => "int",
            Self::Str => "str",
            Self::Bytes => "bytes",
            Self::List => "list",
            Self::Pending => "pending",
        }
    }
}

/// Reference to an arena-resident value: byte offset from the arena base plus
/// the length of the payload.
///
/// Carrying the length here rather than in an arena header costs four bytes per
/// handle and buys two things: `len` is O(1) without touching the arena at all,
/// and restore validation can check a handle's extent without trusting any
/// bytes inside the arena.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Handle {
    pub off: u32,
    pub len: u32,
}

impl Handle {
    /// Sentinel for zero-length arena values. Not a null pointer — offset 0 with
    /// length 0 is a legitimate empty string, and the arena reserves offset 0 to
    /// make that representable.
    pub const EMPTY: Self = Self { off: 0, len: 0 };

    #[must_use]
    pub const fn new(off: u32, len: u32) -> Self {
        Self { off, len }
    }

    /// Exclusive end offset, saturating. Saturation rather than wrapping means a
    /// forged handle fails the `end <= arena.len()` check instead of aliasing
    /// low memory.
    #[must_use]
    pub const fn end(self) -> u32 {
        self.off.saturating_add(self.len)
    }

    /// Pack into the eight-byte payload of a [`Value`].
    #[must_use]
    pub const fn to_bits(self) -> u64 {
        ((self.off as u64) << 32) | self.len as u64
    }

    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self {
            off: (bits >> 32) as u32,
            len: bits as u32,
        }
    }
}

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{:#x}+{}", self.off, self.len)
    }
}

/// Host-assigned identifier for an in-flight effect.
///
/// Monotonic per VM and recorded in the journal, so a replayed run assigns the
/// same ids in the same order — which is why `select`'s winner index is
/// reproducible.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EffectId(pub u64);

impl fmt::Debug for EffectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A VM operand.
///
/// Deliberately not an `enum` with a niche-optimized payload: the explicit tag
/// byte is the serialized form, and matching the in-memory layout to the wire
/// layout is what keeps `cont::snapshot` a memcpy.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Value {
    tag: Tag,
    payload: u64,
}

/// Serialized width of one operand slot. The three unused bytes are padding to
/// keep the operand array 8-byte aligned in both the arena and the snapshot, so
/// the memcpy needs no fixups.
pub const SLOT_LEN: usize = 16;

impl Value {
    pub const VOID: Self = Self {
        tag: Tag::Void,
        payload: 0,
    };

    #[must_use]
    pub const fn int(v: i64) -> Self {
        Self {
            tag: Tag::Int,
            payload: v as u64,
        }
    }

    #[must_use]
    pub const fn str(h: Handle) -> Self {
        Self {
            tag: Tag::Str,
            payload: h.to_bits(),
        }
    }

    #[must_use]
    pub const fn bytes(h: Handle) -> Self {
        Self {
            tag: Tag::Bytes,
            payload: h.to_bits(),
        }
    }

    #[must_use]
    pub const fn list(h: Handle) -> Self {
        Self {
            tag: Tag::List,
            payload: h.to_bits(),
        }
    }

    #[must_use]
    pub const fn pending(id: EffectId) -> Self {
        Self {
            tag: Tag::Pending,
            payload: id.0,
        }
    }

    /// Booleans are integers; the ISA has no bool type because `brz`/`brnz`
    /// only need zero/nonzero and a separate tag would add a lattice element
    /// for nothing.
    #[must_use]
    pub const fn bool(b: bool) -> Self {
        Self::int(b as i64)
    }

    #[must_use]
    pub const fn tag(self) -> Tag {
        self.tag
    }

    #[must_use]
    pub const fn ty(self) -> Ty {
        self.tag.ty()
    }

    #[must_use]
    pub const fn is_void(self) -> bool {
        matches!(self.tag, Tag::Void)
    }

    /// Truthiness for `brz`/`brnz`. Arena values are truthy when non-empty,
    /// which keeps `len`-then-`brz` idioms short in hand-written `.cdx`.
    #[must_use]
    pub fn is_truthy(self) -> bool {
        match self.tag {
            Tag::Void => false,
            Tag::Int => self.payload != 0,
            Tag::Str | Tag::Bytes | Tag::List => self.handle_unchecked().len != 0,
            Tag::Pending => true,
        }
    }

    /// Integer payload, or a type error naming what was found.
    pub fn as_int(self) -> Result<i64> {
        if matches!(self.tag, Tag::Int) {
            Ok(self.payload as i64)
        } else {
            Err(self.tag_error(Ty::Int))
        }
    }

    /// Arena handle for any arena-resident tag.
    pub fn as_handle(self) -> Result<Handle> {
        if self.tag.is_arena() {
            Ok(Handle::from_bits(self.payload))
        } else {
            Err(self.tag_error(Ty::Handle))
        }
    }

    /// Arena handle, requiring a specific tag. `cat` uses this to refuse mixing
    /// a `str` with a `bytes` even though both are arena-resident.
    pub fn as_handle_of(self, want: Tag) -> Result<Handle> {
        if self.tag == want {
            Ok(Handle::from_bits(self.payload))
        } else {
            Err(self.tag_error(want.ty()))
        }
    }

    pub fn as_pending(self) -> Result<EffectId> {
        if matches!(self.tag, Tag::Pending) {
            Ok(EffectId(self.payload))
        } else {
            Err(self.tag_error(Ty::Pending))
        }
    }

    /// Handle without a tag check.
    ///
    /// Only for paths where the tag was already matched — [`Value::is_truthy`]
    /// and the snapshot writer. Returns [`Handle::EMPTY`]-shaped garbage rather
    /// than misbehaving if misused, because the payload is just bits.
    #[must_use]
    fn handle_unchecked(self) -> Handle {
        Handle::from_bits(self.payload)
    }

    fn tag_error(self, want: Ty) -> Diag {
        Diag::new(
            Code::TagMismatch,
            format!("expected {}, found {}", want.name(), self.tag.name()),
        )
    }

    /// Structural equality for the `eq` instruction, at the level that does not
    /// need the arena: same tag and same payload bits.
    ///
    /// Arena values compare by handle here, so two equal strings at different
    /// offsets are *not* equal by this function alone. `interp` resolves that by
    /// interning every constant and by comparing arena contents for the
    /// `Str`/`Bytes` case; see `interp::exec_eq`. Keeping the shallow case here
    /// lets the interpreter skip the arena entirely for integers.
    #[must_use]
    pub fn shallow_eq(self, other: Self) -> bool {
        self.tag == other.tag && self.payload == other.payload
    }

    /// Write the serialized form into `out`. Little-endian, matching the rest of
    /// the container formats.
    pub fn write(self, out: &mut Vec<u8>) {
        out.push(self.tag as u8);
        out.extend_from_slice(&[0u8; 7]);
        out.extend_from_slice(&self.payload.to_le_bytes());
    }

    /// Read a serialized operand slot.
    ///
    /// Validates the tag but not the payload: an arena handle's extent is
    /// checked by `cont::restore` against the arena it ships with, because that
    /// is the only place both are in hand.
    pub fn read(buf: &[u8]) -> Result<Self> {
        let raw = buf
            .get(..SLOT_LEN)
            .ok_or_else(|| Diag::new(Code::SnapshotCorrupt, "operand slot truncated"))?;
        let tag = Tag::from_byte(raw[0]).ok_or_else(|| {
            Diag::new(
                Code::SnapshotCorrupt,
                format!("unassigned value tag {}", raw[0]),
            )
        })?;
        let payload = u64::from_le_bytes(raw[8..16].try_into().expect("slice is 8 bytes"));
        Ok(Self { tag, payload })
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.tag {
            Tag::Void => f.write_str("void"),
            Tag::Int => write!(f, "{}", self.payload as i64),
            Tag::Pending => write!(f, "pending{:?}", EffectId(self.payload)),
            t => write!(f, "{}{:?}", t.name(), self.handle_unchecked()),
        }
    }
}

impl Default for Value {
    fn default() -> Self {
        Self::VOID
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_layout_is_the_wire_format() {
        assert_eq!(core::mem::size_of::<Value>(), SLOT_LEN);
        let mut buf = Vec::new();
        Value::int(-42).write(&mut buf);
        assert_eq!(buf.len(), SLOT_LEN);
        assert_eq!(Value::read(&buf).unwrap().as_int().unwrap(), -42);
    }

    #[test]
    fn handles_roundtrip_through_payload_bits() {
        for h in [Handle::EMPTY, Handle::new(0, 12), Handle::new(u32::MAX, 1)] {
            assert_eq!(Handle::from_bits(h.to_bits()), h);
            let v = Value::str(h);
            assert_eq!(v.as_handle().unwrap(), h);
        }
    }

    #[test]
    fn handle_end_saturates_rather_than_wrapping() {
        assert_eq!(Handle::new(u32::MAX, 8).end(), u32::MAX);
        assert_eq!(Handle::new(4, 8).end(), 12);
    }

    #[test]
    fn tag_mismatch_names_both_types() {
        let e = Value::int(1).as_pending().unwrap_err();
        assert_eq!(e.code, Code::TagMismatch);
        assert!(e.message.contains("pending"), "{}", e.message);
        assert!(e.message.contains("int"), "{}", e.message);
    }

    #[test]
    fn as_handle_of_refuses_a_different_arena_tag() {
        let v = Value::str(Handle::new(8, 4));
        assert!(v.as_handle_of(Tag::Str).is_ok());
        assert!(v.as_handle_of(Tag::Bytes).is_err());
        assert!(v.as_handle().is_ok(), "as_handle is tag-agnostic by design");
    }

    #[test]
    fn truthiness_matches_the_brz_contract() {
        assert!(!Value::VOID.is_truthy());
        assert!(!Value::int(0).is_truthy());
        assert!(Value::int(-1).is_truthy());
        assert!(!Value::str(Handle::EMPTY).is_truthy());
        assert!(Value::str(Handle::new(0, 3)).is_truthy());
        assert!(Value::pending(EffectId(0)).is_truthy());
    }

    #[test]
    fn every_tag_maps_to_a_lattice_element() {
        for b in 0..=5u8 {
            let t = Tag::from_byte(b).expect("assigned tag");
            assert_eq!(t as u8, b, "tag numbering is the wire format");
            assert_ne!(t.ty(), Ty::Top, "no tag may map to the error element");
        }
        assert!(Tag::from_byte(6).is_none());
    }

    #[test]
    fn reading_an_unassigned_tag_is_corruption_not_a_panic() {
        let mut buf = vec![0u8; SLOT_LEN];
        buf[0] = 200;
        assert_eq!(Value::read(&buf).unwrap_err().code, Code::SnapshotCorrupt);
        assert_eq!(
            Value::read(&buf[..4]).unwrap_err().code,
            Code::SnapshotCorrupt
        );
    }
}
