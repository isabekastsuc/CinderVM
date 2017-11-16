# Contributing to CinderVM

Thanks for considering a contribution. This document describes the toolchain,
the conventions, and the review bar. Read it before opening a PR; the corpus
and the ISA tables have rules that CI will not remind you about.

## Table of contents

- [Prerequisites](#prerequisites)
- [Development setup](#development-setup)
- [Making changes](#making-changes)
  - [Branch naming](#branch-naming)
  - [Commit messages](#commit-messages)
- [Checklist before a PR](#checklist-before-a-pr)
- [Review process](#review-process)
- [Changing the instruction set](#changing-the-instruction-set)
- [Corpus rules](#corpus-rules)
- [License](#license)

## Prerequisites

- Rust >= 1.78. The pinned toolchain lives in `rust-toolchain.toml`; `rustup`
  picks it up automatically on `cd` into the repo.
- Go >= 1.22, only if you touch `cmd/cinderd` or `internal/`.

There is no C toolchain, no build script, and no system library dependency.

## Development setup

```sh
git clone https://github.com/ashish/cindervm
cd cindervm
make            # debug build of both halves
make test       # unit + corpus conformance + Go tests + cross-language e2e
make verify     # clippy -D warnings, rustfmt, go vet, staticcheck, isa drift
```

`make verify` is the gate: a green local `make verify && make test` is what CI
runs, so a green CI is a formality. Individual cargo commands:

```sh
cargo build --workspace
cargo test  --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --all --check
```

Fuzzing and benchmarks:

```sh
make fuzz T=60   # 60s of bytecode fuzzing against the verifier/interpreter pair
make bench       # criterion, baseline-compared
make miri        # nightly Miri over heap:: and cont:: only
```

## Making changes

The crate is deliberately flat and the boundaries follow the machine's
structure. If you are unsure where a change belongs, ask in the issue before
writing code. `src/isa.rs` is the single definition of the instruction set —
the opcode table, operand shapes, stack effects, and type rules live in one
`const` array, and the assembler, disassembler, verifier, and `docs/isa.md`
are all derived from it.

### Branch naming

Branch off `main`. Never commit to `main` directly.

| Prefix | Use |
|---|---|
| `feat/` | new opcodes, instructions, or CLI surface |
| `fix/` | bug fixes, including fuzzer findings |
| `perf/` | performance work with a benchmark to prove it |
| `refactor/` | no behavior change, structure only |
| `test/` | corpus, integration, or property tests |
| `docs/` | documentation only |
| `ci/` | workflows, tooling, dependency bumps |

### Commit messages

Conventional Commits, lowercase imperative subject, optional scope, with a
body when the why is not obvious:

```
feat(verifier): report both predecessor blocks on E_DEPTH_MERGE
