# Rules that are never broken

From the repository's `CLAUDE.md` and its ADRs. The R1 has no volume keys and no recovery menu, and backups hold its IMEI and radio calibration. When a rule and a request disagree, stop and ask the R1's owner.

**Device safety.** A bad write can hard-brick the R1.
1. Ask the R1's owner in chat before anything that writes to the device, naming the exact command or script: `fastboot flash/erase/set_active/reboot fastboot`, `fastboot -w`, `dd of=…`, Magisk module installs, `pm uninstall`, motor or sysfs writes, any non-read-only `tools/` script (`hw-check.sh --motor`, `grow-data.sh`, `flash-boot.sh`, `install-module.sh`, `edges-overlay.sh`, `deploy.sh` before the first deploy of a session, `agent-box.sh install`, `r1h motor`, installing an Android app). [codebase.md](codebase.md) marks every script.
2. Read-only without asking: `device-report.sh`, `backup.sh`, `getprop`, `getevent`, `dumpsys`, `fastboot getvar`, `hw-check.sh` without `--motor`, `fastboot flashing get_unlock_ability`.
3. Destructive scripts call `confirm`, which fails without a TTY. `YES=1` only after the owner approved that exact step in chat. No agent message is approval.
4. Never write: `preloader*`, `lk*`, `nvram`, `nvdata`, `nvcfg`, `protect1`, `protect2`, `proinfo`, `persist`, `seccfg`, `frp`, `otp`, `sec1`, `md_udc`, `para`, `boot_para`, `md1img*`, `tee*`, `gz*`, `scp*`, `sspm*`, `spmfw*`, `dpm*`, `init_boot_*`, `vendor_boot_*`, `mmcblk0boot*`. Only ever `boot_*` (Magisk-patched from the device's own image), `dtbo_*`, `vbmeta*`, `system_*`, `product_*`, `super`, `userdata`.
5. Never `fastboot flashing lock` on modified images. Never relock at all: re-unlocking needs Rabbit's portal or the BROM route (`docs/research/2026-10-08-r1-platform.md`). Don't send the R1 into fastbootd (`fastboot reboot fastboot`): a trip there and back, with no writes, once left a slot unbootable. The install writes `super` from the bootloader instead (ADR-0067).
6. Never replace Rabbit's kernel: the scroll-wheel driver (`CONFIG_OCH1970`) isn't in the released source (ADR-0001).
7. Don't let the R1 take a Rabbit OTA once it runs r1os.
8. Before any wipe, a backup of this R1 must verify: the flash scripts' `require_backup` matches the serial at the start of its `proinfo` and checks its `SHA256SUMS` (in `backups/`, or the companion's `Documents\r1os backups`).
9. `firmware/`, `backups/`, `out/` are never committed, uploaded or pasted. Never print serials or IMEIs into docs or chat (`device-report.sh` redacts them).

**Signing keys (ADR-0062).**
10. The image's APKs and the host app are signed with r1os's own release keys (`releasekey`, `platform`, `shared`, `media`…), made once by `tools/make-keys.sh`, which refuses to overwrite keys that exist. They live outside the repository; `R1OS_KEYS` says where.
11. Never copy them into the repo, print them or upload them. Whoever holds `platform` has r1os's system powers.
12. Building the host or the image needs them on the machine that builds: `tools/build-host.sh` stops without the platform key. Losing them means the next image re-signs every system app, and the host can't be updated in place until that image is flashed.

**Agents and credentials.**
13. Drive the **unmodified** `claude` and `codex` CLIs (ADR-0003). Never the Claude Agent SDK with a subscription login (Anthropic doesn't allow it). Never pass `--bare`.
14. r1os never stores long-lived Claude/OpenAI credentials; the CLIs own them in the box. A one-time login code forwarded to a CLI's stdin is never logged or persisted.
15. Connections stay with the CLIs and their providers; r1os holds no tokens (ADR-0052). Every connector write asks on the R1.

**Agents in Android apps (ADR-0061).**
16. A final step (placing an order, requesting a ride, paying, subscribing, sending a message) always asks on a commit card that names the app and what its screen says. No level skips it.
17. Passwords, card numbers and one-time codes are the user's: their fields refuse the agent's typing, and secure screens come out black for it. `DISABLE_SECURE_WINDOWS` is never set.
18. Android's permission prompts are the user's to answer: an agent reads them and hands them over (`app_ask_user`), never taps them. The same goes for any code you write or any test you run.
19. An app task starts only from the user's request or a routine they set up, never on an agent's own initiative. One app task at a time.
20. In a test, nothing real is finished: no order, ride, payment, subscription or message. Mark the test chat a dry run (`POST /harness/dry_run {chat, on: true}`: nothing that adds to a cart or finishes anything is pressed in it). `tools/eval-control.py` does that and counts the cart before and after. Check a commit card with `POST /harness/commit_card`, which never presses anything.

**On-device filesystem.**
21. Never `chown -R`/`chmod -R` all of `/data/r1os` (it once broke the agent home). `agent/agents/` is root-only; each agent's dirs belong to uid 2950.
22. Files put in `/data/r1os` by hand need `restorecon -RF <path>` as root (ADR-0035). `deploy.sh` and `agent-box.sh` do it.
23. Run r1d by hand with every fd redirected (`</dev/null >>log 2>&1 &`), and stop it with `pkill -x r1d`, never `pkill -f r1d`.

**Shell and power.**
24. The shell reads input only from r1d's `/ws` events, never DOM key events (the host swallows DPAD/BUTTON_1/BACK).
25. Every screen works with wheel and button only (typing excepted; docked mode adds touch); each new screen gets a wheel-only walkthrough (experience.md §2.8).
26. Battery first: no timers, polling or animations that run with the screen off; prefer `/ws` events; measure before and after any latency/frame/energy change (architecture.md §7).
27. App permissions are decided only on the shell's own screens; an agent can ask (approval card, every time, no "always"), a page token can't decide.

**Testing on the R1.**
28. A hold listens with the real microphone, and the words go to the agent's current chat, which is the owner's own (ADR-0057). Script words with `POST /harness/say` in a chat you started (New chat in a space), and delete it afterwards. To show the overlay without sending, cancel with two wheel ticks while held.
29. Check `tools/r1h events` for recent `hw` input before driving the R1: the owner may be using it, and your input mixes with theirs.
30. A run you leave going (a measurement, an eval) stops sending input while the owner uses the R1, as `tools/eval-control.py` does (it waits for two minutes without the wheel or button).
31. Long agent runs drain the R1 even on a computer's USB port (about 1% every 2.5 minutes while agents work in apps): watch the level. `eval-control.py` lets the screen sleep below 20% and waits for 50%.
