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
