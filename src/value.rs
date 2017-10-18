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
