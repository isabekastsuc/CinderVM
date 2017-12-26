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
