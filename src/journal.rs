//! The journal: a hash-chained, append-only record of every host interaction.

use crate::diag::{Code, Diag, Result};
use crate::hash::{chain, hash_bytes};
use crate::value::Value;

const JOURNAL_MAGIC: &[u8; 4] = b"CJRN";

/// One journalled interaction.
#[derive(Clone, Debug)]
pub struct Record {
    pub seq: u64,
    pub kind: u8,
    pub payload: Vec<u8>,
    pub prev: u64,
}

/// Append-only chain of records.
#[derive(Clone, Debug, Default)]
pub struct Journal {
    records: Vec<Record>,
    head: u64,
}

impl Journal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            head: 0,
        }
    }

    /// Append a record; the previous chain hash is extended.
    pub fn append(&mut self, kind: u8, payload: &[u8]) -> Result<u64> {
        let seq = self.records.len() as u64;
        let prev = self.head;
        let head = chain(prev, payload);
        self.records.push(Record {
            seq,
            kind,
            payload: payload.to_vec(),
            prev,
        });
        self.head = head;
        Ok(head)
    }

    /// Replay-walk with integrity checking.
    pub fn verify_chain(&self) -> Result<()> {
        let mut prev = 0u64;
        for r in &self.records {
            if r.prev != prev {
                return Err(Diag::new(Code::ChainBroken, "journal chain broken"));
            }
            prev = chain(prev, &r.payload);
        }
        if self.head != prev {
            return Err(Diag::new(
                Code::Diverged,
                "journal head disagrees with chain",
            ));
        }
        Ok(())
    }

    /// Look up the answer recorded for a kind at a sequence number.
    pub fn answer(&self, seq: u64) -> Option<&Record> {
        self.records.get(seq as usize)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Serialize.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(JOURNAL_MAGIC);
        out.extend_from_slice(&self.head.to_le_bytes());
        out.extend_from_slice(&(self.records.len() as u32).to_le_bytes());
        for r in &self.records {
            out.push(r.kind);
            out.extend_from_slice(&r.seq.to_le_bytes());
            out.extend_from_slice(&r.prev.to_le_bytes());
            out.extend_from_slice(&(r.payload.len() as u32).to_le_bytes());
            out.extend_from_slice(&r.payload);
        }
        out
    }

    /// Deserialize and chain-verify.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(bytes);
        if c.take(4)? != JOURNAL_MAGIC {
            return Err(Diag::new(Code::RecordMalformed, "bad journal magic"));
        }
        let head = u64::from_le_bytes(c.take(8)?.try_into().expect("8 bytes"));
        let n = u32::from_le_bytes(c.take(4)?.try_into().expect("4 bytes")) as usize;
        let mut records = Vec::with_capacity(n);
        for _ in 0..n {
            let kind = c.take(1)?[0];
            let seq = u64::from_le_bytes(c.take(8)?.try_into().expect("8 bytes"));
            let prev = u64::from_le_bytes(c.take(8)?.try_into().expect("8 bytes"));
            let len = u32::from_le_bytes(c.take(4)?.try_into().expect("4 bytes")) as usize;
            let payload = c.take(len)?.to_vec();
            records.push(Record {
                seq,
                kind,
                payload,
                prev,
            });
        }
        let journal = Self { records, head };
        journal.verify_chain()?;
        Ok(journal)
    }

    /// Hash of the payload bytes of a record, for `replay` divergence checks.
    #[must_use]
    pub fn payload_hash(r: &Record) -> u64 {
        hash_bytes(&r.payload)
    }
}

/// A journalled value payload.
pub fn value_payload(v: &Value) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    v.write(&mut out);
    out
}

/// Kinds shared by the journal writer and the replay host.
pub const KIND_EFFECT: u8 = 1;
pub const KIND_ORACLE: u8 = 2;
pub const KIND_LOG: u8 = 3;

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos + n;
        if end > self.data.len() {
            return Err(Diag::new(Code::RecordMalformed, "journal truncated"));
        }
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_verify() {
        let mut j = Journal::new();
        j.append(KIND_ORACLE, &[1, 2, 3]).unwrap();
        j.append(KIND_EFFECT, &[4, 5]).unwrap();
        j.verify_chain().unwrap();
        let bytes = j.to_bytes();
        let back = Journal::from_bytes(&bytes).unwrap();
        assert_eq!(back.len(), 2);
    }
}
