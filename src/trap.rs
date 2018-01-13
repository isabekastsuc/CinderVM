//! The host boundary: what the interpreter asks for and what the host hands
//! back.

use crate::isa::Op;
use crate::value::{EffectId, Value};

/// One step of the machine.
#[derive(Clone, Debug)]
pub enum Step {
    /// The program stopped with an exit status.
