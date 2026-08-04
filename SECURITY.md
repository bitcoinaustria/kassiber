# Security Policy

## Supported versions

Kassiber is pre-release. Only the latest published release receives fixes;
there are no maintenance branches for older versions.

| Version | Supported |
| --- | --- |
| latest release (`0.22.x`) | yes |
| anything older | no — upgrade first |

## Reporting a vulnerability

**Do not open a public issue, pull request, or discussion for a
security-impacting bug.**

Use any of these:

- Email `security@bitcoin-austria.at`.
- Signal: `BitcoinAT.21`.
- GitHub → the repository's **Security** tab → **Report a vulnerability**
  (private advisory — keeps the report, the fix, and the credit in one place).

Please include:

- affected version (`kassiber --version`) and platform,
- what an attacker gains, and
- a reproduction — steps, or a minimal command sequence.

Use `kassiber diagnostics collect` for the environment part of the report: its
output is designed to be safe to share. Never send descriptors, xpubs, seed
words, passphrases, API keys, or backend tokens; describe them instead.

## What to expect

- Acknowledgement within 7 days.
- An assessment (accepted / not-a-vulnerability / duplicate) and, if accepted,
  a rough fix timeline in the same thread.
- Coordinated disclosure: we publish an advisory and credit you, unless you
  prefer otherwise. Please hold public details until the fix ships.
- There is no bug bounty. This is a volunteer project.

## Scope

In scope: the Kassiber CLI, Python daemon, and desktop app in this
repository — anything that leaks wallet, book, or credential material,
weakens the at-rest encryption boundary, or lets untrusted input reach code
execution.

Out of scope (documented behavior, not bugs — see
[Privacy & security](docs/reference/privacy-and-security.md) for the reasoning):

- a compromised OS, or any process already running as your user reading
  Kassiber state,
- the privacy cost of syncing against a third-party backend you configured,
- missing SPV/header verification: chain backends are trusted,
- accounting or tax-math errors — those are ordinary bugs, file them publicly,
- anything in a third-party dependency: report it upstream.

## Security model

The threat model, the complete outbound-request inventory, the SQLCipher
at-rest boundary, credential storage, and the known caveats live in
[docs/reference/privacy-and-security.md](docs/reference/privacy-and-security.md).
Read that before pointing Kassiber at real wallets.
