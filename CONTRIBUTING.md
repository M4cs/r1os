# Contributing to r1os

Thanks for helping. r1os is open source under the [MIT licence](LICENSE): by contributing, you agree your work is
released under it too.

## Before you start

- **Bugs and ideas:** open an issue (there are templates). For a security hole, don't: see [SECURITY.md](SECURITY.md).
- **A bigger change:** open an issue first, so we can agree on the shape before you build it.
- **Working with a coding agent?** [CLAUDE.md](CLAUDE.md) holds the rules it should follow. They're the rules for people
  too.

## The R1 can be bricked

The Rabbit R1 has no volume keys and no recovery menu, and a bad write to the wrong partition can leave it dead. So:

- **Partitions:** never write one outside the short list in CLAUDE.md, and never the R1's own (`nvram`, `nvdata`,
  `proinfo`, `seccfg` and the rest). Never relock the bootloader.
- **Backups:** back up your R1 before you flash anything.
- **Never commit or paste** `firmware/`, `backups/`, `out/`, signing keys, a serial number or an IMEI. Logs shared in
  issues should have them removed.
- **Writing code that can write to an R1:** it goes through `crates/r1-usb/src/safety.rs`, and comes with tests
  against a fake device.

## Building and testing

The full table is in CLAUDE.md. The usual ones:

| What | Command |
|---|---|
| r1d tests and lint | `cargo test -p r1d` · `cargo clippy -p r1d --all-targets -- -D warnings` |
| Everything in Rust | `cargo test --workspace` · `cargo clippy --workspace --all-targets -- -D warnings` |
| The shell, companion and site | `bun install`, then `bun run typecheck` |
| The desktop simulator (no R1 needed) | `tools/sim.sh`, then open http://127.0.0.1:7171 |
| The companion with a pretend R1 | `bun run --cwd companion dev` |

CI runs the Rust tests and lint, the typechecks and a secret scan on every pull request.
To scan for secrets before each commit too, turn on the hook: `git config core.hooksPath tools/hooks` (it needs
[gitleaks](https://github.com/gitleaks/gitleaks)).

## Pull requests

- **Keep it focused:** one change per pull request.
- **Say what changed and why,** and how you tested it: on the simulator, on an R1, or only unit tests.
- **Behaviour changes update the docs** in the same pull request: the pages on the website (`web/src/docs/content`), and
  the agent skill (`web/public/skills/r1os`) when a rule changes.
- **Match the style around your change:** naming, comments, and plain wording in anything a person reads.
- **Sign off your commits** (`git commit -s`): it certifies the [Developer Certificate of
  Origin](https://developercertificate.org/), that you wrote the change or have the right to submit it.
