# Security Policy

CinderVM is a trust-boundary component: the interpreter is the last code
between untrusted bytecode and a host process, and the supervisor is the last
code between a tenant's tool call and the outside world. Bugs here are graded
accordingly.

## Supported Versions

| Version | Supported |
|---|---|
| 0.9.x | Active development; security fixes released with the next patch |
| 0.8.x | Security fixes only |
| 0.7.x and earlier | End of life |

## Reporting a Vulnerability

Please do **not** open a public issue for a security problem.

Report it through GitHub's private vulnerability reporting (the "Security"
tab on the repository), or by email to `security@cindervm.dev`. If you report
by email, mention `[security]` in the subject and, if you have one, attach a
PGP key fingerprint in the first message so we can agree on an encrypted
channel.

What to include:

- Version (`cindervm --version`, and the ISA version the image declares).
- The image or journal that reproduces the issue, or a minimized `.cdx`
  source. Prefer a minimized input; the corpus rules apply to you too.
- Expected vs. actual behavior, and your assessment of impact (interpreter
  panic, verifier admitting invalid bytecode, snapshot corruption, budget
  bypass, journal divergence).

What happens next:

- We acknowledge within 2 business days.
- We triage within 5 business days: reproduce, classify, and either confirm
  or explain why the behavior is by design.
- Fixes land in the oldest supported line. Disclosure is coordinated with you
  and typically happens 90 days after the fix ships; nothing is disclosed
  before a fix is available.
- You are credited in the changelog and release notes unless you ask not to
  be.

## Scope

In scope: the Rust crate (`src/`), the supervisor (`cmd/cinderd` and
`internal/`), the frame protocol between them, and the deployment guidance in
`docs/deployment.md`.

Out of scope: third-party tools invoked through the syscall broker, the Go and
Rust toolchains themselves, and host infrastructure.

## Notes for reviewers and maintainers

- A panic in the interpreter on verified bytecode is a security bug, not a
  robustness nit. The fuzzer's invariant exists to make this class rare.
- The crate denies `unsafe` at the lint level; `heap::` and `cont::` are
  additionally Miri'd in CI, because aliasing mistakes would hide there.
- A verifier that admits an image the interpreter cannot run is a security
  bug. When in doubt, prefer the verifier rejecting too much over the
  interpreter checking too little: a false rejection is a compiler error, a
  false admission is a memory-safety incident waiting to happen.
- `cinderd` binds `127.0.0.1:7749` with no authentication by default. That is
  intentional for a trusted boundary; exposing it beyond one without setting
  `CINDERD_AUTH` (and keeping `/debug/vm` unreachable) is misconfiguration,
  and PRs should not silently change this default.