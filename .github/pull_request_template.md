## What and why

<!-- What this changes, and why. Link the issue if there is one. -->

## How it was tested

<!-- The commands you ran, and where: the simulator, an R1 (which image version), unit tests only. -->

## Checklist

- [ ] `cargo test` and `cargo clippy -- -D warnings` pass for what I touched; `pnpm -r typecheck` too.
- [ ] Docs updated if behaviour changed: `web/src/docs/content`, and the agent skill if a rule changed.
- [ ] Nothing here writes a partition outside CLAUDE.md's list, and nothing that writes to an R1 bypasses `crates/r1-usb/src/safety.rs`.
- [ ] No serial numbers, IMEIs, keys, `firmware/`, `backups/` or `out/` in the diff.
- [ ] My commits are signed off (`git commit -s`).
