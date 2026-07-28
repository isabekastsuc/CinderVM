//! The host boundary: what the interpreter asks for and what the host hands
//! back.

use crate::isa::Op;
use crate::value::{EffectId, Value};

/// One step of the machine.
#[derive(Clone, Debug)]
pub enum Step {
    /// The program stopped with an exit status.
    Halted(i64),
    /// The interpreter needs the host: an effect to perform or an oracle to
    /// answer.
    Trap(Trap),
}

/// What the interpreter wants from the host.
#[derive(Clone, Debug)]
pub enum Trap {
    /// Run tool `tool` with the packed argument list. The host answers with
    /// [`Answer::Effect`].
    Effect {
        id: EffectId,
        tool: u32,
        args: Vec<Value>,
    },
    /// A non-deterministic source (`now`, `rand`, `env`, `log`, `queryq`).
    /// The host's reply is journalled so replay reproduces it exactly.
    Oracle(Op),
    /// Cooperative preemption point (`yieldctx`).
    Yield,
}

/// The host's reply to a [`Trap`].
#[derive(Clone, Debug)]
pub enum Answer {
    /// A value resolving the pending effect or oracle.
    Value(Value),
    /// The effect failed; `value` is the structured error payload.
    Fail(Value),
    /// The host declined to continue (shutdown path).
    Shutdown,
}
