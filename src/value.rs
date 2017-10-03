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
