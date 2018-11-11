<div align="center">

<img src="docs/assets/cinder-mark.svg" alt="cindervm mark: a bottom-anchored operand stack feeding a program-counter ring, crossed by a dashed snapshot boundary" width="132" height="132">

# cindervm

A deterministic bytecode virtual machine for agent execution.
Bytecode is verified before it runs, the whole machine is a value you can
snapshot, and a run replays exactly from its journal.

</div>

---

## Contents

- [What this is](#what-this-is)
- [Why bytecode instead of a coroutine](#why-bytecode-instead-of-a-coroutine)
- [The three invariants](#the-three-invariants)
- [Toolchain walkthrough](#toolchain-walkthrough)
- [The instruction set](#the-instruction-set)
- [The verifier](#the-verifier)
- [Continuations and replay](#continuations-and-replay)
- [Building from source](#building-from-source)
- [Repository layout](#repository-layout)
- [Stability](#stability)
- [License](#license)

---

## What this is

An agent run is long-lived, effectful, and resumable. Modelled as a host
coroutine, its suspended state is a compiler-generated struct with no
serializable form: you cannot write it to disk, inspect it, or resume it in a
different process.

`cindervm` models the run as bytecode on a machine built for it. The library
crate (`cindervm`) holds the machine and its toolchain; three binaries drive it:

| Binary       | Role                                                             |
| ------------ | --------------------------------------------------------------- |
| `cinderc`    | Assemble `.cdx` to a verified `.cdxb` image, disassemble, run.   |
| `cinder`     | Print the version, explain a diagnostic code, run an image.      |
| `cinder-fuzz`| Mutation-fuzz the verifier and interpreter against each other.   |

The crate has no dependencies outside `core`, `alloc`, and `std`. The hash
(`hash.rs`), the heap arena (`heap.rs`), and the argument parsing in the
binaries are all written in-crate, because this is a trust boundary and every
dependency is code you would also be trusting. `unsafe` is denied crate-wide
(`#![deny(unsafe_code)]`).

## Why bytecode instead of a coroutine

Three designs were considered and set aside:

**Host coroutines** (`async fn`, `async def`). Suspension points are implicit
and the suspended state has no stable representation. Good for I/O concurrency,
wrong for a run you want to move or persist.

**Replay-based durable execution.** Recovery re-runs the whole program and
short-circuits completed calls from a log, which makes one unlogged source of
non-determinism enough to diverge. `cindervm` restores state directly rather
than re-deriving it, and every source of non-determinism (`NOW`, `RAND`, tool
