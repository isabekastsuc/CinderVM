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
answers) is an instruction that reads from the journal.

**Interpreting a live object graph.** Serialization becomes a graph walk with
identity preservation. A flat operand stack over a flat heap arena of tagged
values serializes as a copy of two arrays plus a relocation pass.

The VM approach costs an assembler and a verifier. It buys a machine state that
is already a byte string.

## The three invariants

Most of the codebase falls out of three rules, stated verbatim at the top of
`src/lib.rs`:

1. **The interpreter performs no I/O.** `interp::Vm::step` is a pure function of
   `(image, state, answer)`. When it needs an effect it returns a `trap::Trap`
   and stops; the host answers with a `trap::Answer`. Replay is exact because
   `replay::Host` serves answers from a journal and the interpreter cannot tell
   the difference: there was never a code path that touched the network.
2. **Every value is `Copy`, tagged, and pointer-free.** `value::Value` is 16
   bytes with an explicit tag byte and no `Drop`. Larger payloads live in the
   arena behind a `value::Handle`, an arena-relative offset. Snapshot is two
   copies and a hash; restore relocates by arithmetic.
3. **Verification is a precondition, not a mode.** An `image::Image` cannot be
   constructed except through `verify::admit`. By the time the interpreter sees
   an image, stack depth, operand types, pending-value liveness, branch targets,
   metering on back-edges, and fork balance have been proven. So `interp`
   carries no `pc` bounds check and no stack-depth check.

## Toolchain walkthrough

`.cdx` is a macro assembler for the bytecode, meant for tests and examples; the
production path is `asm::Builder` driven by a higher-level frontend. A minimal
program and the toolchain applied to it:

```asm
        .isa      cdx/4
        .image    "triage"
        .fn       main() -> i32
        .maxstack 1
main:
        ldi       0
        ret
```

```sh
$ cinderc build triage.cdx -o triage.cdxb
wrote triage.cdxb (N bytes, checksum 0x...)

$ cinderc dis triage.cdxb          # disassemble the sealed image
$ cinderc run triage.cdxb --trace  # run, tracing traps and the halt code
$ cinder explain E_PENDING_ESCAPE  # look up a diagnostic code
```

`cinderc build` assembles, then calls `verify::admit`, then writes the sealed
`.cdxb`. `cinderc run` reads an image, constructs a `Vm` with default limits,
and drives the `step` loop, answering each trap. `cinder run` delegates to
`cinderc run`; `cinder explain <CODE>` prints the blurb and phase for a
diagnostic code, or lists every known code when the argument is unrecognised.

<div align="center">
<img src="docs/assets/demo.svg" alt="Toolchain flow: cinderc build assembles and verifies a .cdx file into a sealed .cdxb image, which cinderc run then executes step by step" width="860">
</div>

