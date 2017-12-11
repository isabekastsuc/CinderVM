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
