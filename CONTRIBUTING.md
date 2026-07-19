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

The diagnostic named only the first predecessor, which made merges
reachable from three or more blocks confusing to debug. Collect all
predecessors during the fixpoint and list them in order of visit.

Closes #214
```

- Allowed types: `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `chore`, `ci`.
- Common scopes: `isa`, `asm`, `verify`, `interp`, `cont`, `journal`,
  `replay`, `heap`, `cinderd`, `sched`, `broker`.
- A breaking change (new ISA version, changed journal semantics) must be
  flagged in the footer: `BREAKING CHANGE: ...` — this is how `CHANGELOG.md`
  gets its `Changed`/`Removed` entries.
- Add a `CHANGELOG.md` entry in the same commit for anything a user could
  observe.

## Checklist before a PR

- [ ] `cargo fmt --all --check` is clean.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` is clean.
- [ ] `cargo test --all-features` passes.
- [ ] `make test` passes (this includes the corpus conformance run and the
      cross-language e2e tests, which need `cinderc` to be built first).
- [ ] If you touched an opcode: `docs/isa.md` regenerated
      (`cargo run --bin cinderc -- --emit-isa-md > docs/isa.md`) and corpus
      images added or updated per the [corpus rules](#corpus-rules).
- [ ] If you changed verifier or continuation behavior: `make fuzz T=60` ran
      clean, and `make miri` passed for `heap::` / `cont::` if you touched
      either.
- [ ] No new dependencies in the core crate. Zero-dependency is a policy,
      not an accident: dev-dependencies (criterion, proptest) are fine.
- [ ] No `unsafe`. The crate denies it at the lint level
      (`#![deny(unsafe_code)]`); a change that needs `unsafe` is a design
      discussion, not a PR.
- [ ] `CHANGELOG.md` entry added under `[Unreleased]`.

## Review process

- PRs are reviewed by at least one maintainer; changes to `src/verify.rs`,
  `src/cont.rs`, and `src/journal.rs` need two.
- Keep PRs focused. A PR that mixes a corpus addition with a verifier change
  is harder to review and is usually asked to split.
- Respond to review comments; it is fine to push back, but say why.
- Merges are squash-commits onto `main`; the PR title becomes the commit
  subject, so make it a good Conventional Commit line.

## Changing the instruction set

Adding an opcode touches, in one commit:

1. `src/isa.rs` — the `const` table entry: mnemonic, encoding shape, stack
   effect, type rule.
2. `src/asm.rs` and `src/disas.rs` — they are derived from the table, so this
   is usually just a compile error telling you where to look.
3. `src/verify.rs` — the transfer function, and the lattice if you introduce a
   new type.
4. `src/interp.rs` — the instruction semantics, and the `Trap` surface if the
   opcode can suspend.
5. `docs/isa.md` — regenerated, never hand-edited.
6. The corpus — see below.

Adding opcodes bumps the crate minor version. Changing the meaning of an
opcode bumps the ISA version (`cdx/N`), and existing images keep declaring
their target version, so old images keep verifying.

## Corpus rules

`corpus/` holds the conformance suite: 214 images that must be rejected and
96 that must be accepted, each with an expected outcome in
`corpus/MANIFEST.tsv`.

- Every new opcode needs at least one accepted image exercising it and at
  least one rejected image for each error path it can hit.
- Rejected images must be accompanied by the exact expected error code
  (`E_PENDING_ESCAPE`, `E_DEPTH_MERGE`, `E_FORK_IMBALANCE`, ...). If the code
  does not exist yet, name the new code in the PR description.
- Do not delete or relabel existing images without a reason in the PR; the
  corpus is a regression contract.
- Fuzzer findings that expose a verifier gap arrive as new rejected images,
  not as interpreter changes.

## License

By contributing you agree that your contribution is licensed under the
project's terms as stated in the README (Apache-2.0 OR MIT, at your option).