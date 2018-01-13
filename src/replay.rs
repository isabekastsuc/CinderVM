//! Replay: a host that serves answers from a journal instead of the network.

use crate::diag::{Code, Diag, Result};
use crate::journal::{Journal, KIND_EFFECT, KIND_ORACLE};
use crate::trap::{Answer, Trap};
use crate::value::Value;

/// A replay host over a journal.
#[derive(Clone, Debug)]
pub struct Host {
    journal: Journal,
    cursor: u64,
}

impl Host {
    #[must_use]
    pub fn from_journal(journal: Journal) -> Self {
        Self { journal, cursor: 0 }
    }

    /// Answer a trap from the journal. `effect` matches the recorded kind.
    pub fn answer(&mut self, trap: &Trap) -> Result<Answer> {
        let record = self
            .journal
            .answer(self.cursor)
            .ok_or_else(|| Diag::new(Code::JournalExhausted, "journal ended mid-run"))?;
        self.cursor += 1;
        let expected = match trap {
            Trap::Effect { .. } | Trap::Yield => KIND_EFFECT,
            Trap::Oracle(_) => KIND_ORACLE,
        };
        if record.kind != expected {
            return Err(Diag::new(
                Code::Diverged,
                format!("journal kind {}, expected {}", record.kind, expected),
            ));
        }
        let mut buf = [0u8; 16];
        let v = if record.payload.len() >= 16 {
            Value::read(&record.payload)
        } else {
            buf[..record.payload.len()].copy_from_slice(&record.payload);
            Value::read(&buf)
        }?;
        Ok(Answer::Value(v))
    }

    #[must_use]
    pub fn remaining(&self) -> usize {
        self.journal.len().saturating_sub(self.cursor as usize)
    }
}

/// A live host that captures every answer into a journal for later replay.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    journal: Journal,
}

impl Recorder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            journal: Journal::new(),
        }
    }

    pub fn record(&mut self, trap: &Trap, answer: &Answer) -> Result<()> {
        let payload = match (trap, answer) {
            (Trap::Oracle(_), Answer::Value(v)) => {
                let mut buf = Vec::with_capacity(16);
                v.write(&mut buf);
                buf
            }
            (Trap::Effect { .. }, Answer::Value(v)) => {
                let mut buf = Vec::new();
                v.write(&mut buf);
                buf
            }
            _ => vec![0u8; 16],
        };
        let kind = if matches!(trap, Trap::Oracle(_)) {
            KIND_ORACLE
        } else {
            KIND_EFFECT
        };
        self.journal.append(kind, &payload)?;
        Ok(())
    }

