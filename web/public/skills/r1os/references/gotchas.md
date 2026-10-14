# Gotchas and lessons

Things that have already gone wrong, each as **symptom → cause → what to do**, with where it was learned. Search for the symptom before debugging from scratch. When you learn a new one, add it here so the next agent finds it (and, if you keep the maintainers' vault, in its research notes too). `docs/runbooks/` are the maintainer's own notes for one machine: take facts from the `tools/` scripts and specs, not their steps or paths.

## Windows, Git Bash and tooling
1. adb sees `C:/Program Files/Git/proc/version` → Git Bash rewrites POSIX-looking args → `MSYS_NO_PATHCONV=1` (`tools/lib.sh` exports it; prefix by hand: `MSYS_NO_PATHCONV=1 adb shell cat /proc/version`). Harmless on Linux and macOS. (`tools/lib.sh`)
2. adb.exe can't find a local file in `/tmp` → native adb doesn't see Git Bash's /tmp → keep local paths under the repo (`$ROOT`, from `pwd -W`). (`tools/lib.sh`)
3. Android build of r1d links wrong / loses `--target` → a global `RUSTFLAGS` (e.g. `-C target-feature=+crt-static`) beats target-specific flags → always use `tools/build-r1d.sh` (sets `CARGO_ENCODED_RUSTFLAGS`, unsets `RUSTFLAGS`). Also the NDK's `.cmd` clang wrappers break on a space in the user's home path, so it calls `clang.exe --target=…` directly. (`tools/build-r1d.sh`)
4. A script needs to pick JSON apart → scripts use Python (`python3`; `py -3` on Windows), not `jq`, so they run wherever the tools do. (convention)
5. `fastboot devices` empty, Device Manager "Android" ⚠ code 28 → the R1's bootloader enumerates as `0E8D:201C`, the same ID as adb, and Windows has nothing to match it with → r1os's own driver isn't installed: the owner runs `cargo run -p r1-usb --example drivers -- install` (or the companion's wizard), which shows Windows' admin prompt and binds the R1 in both modes from then on; agents never change drivers. Fastboot is USB-only. (`crates/r1-usb/src/drivers.rs`)
6. `flash-boot.sh`'s dd route fails on the September 2026 bootloader → it write-protects `boot_*` from Android → boot writes go through fastboot there. (`tools/flash-boot.sh`)

## adb, wireless, root
7. Tools 403 / empty token after a reboot → adbd runs as the shell user → `adb root` first (tools read the token as root). (CLAUDE.md)
8. `curl` 401 after a deploy or restart → r1d made a new per-boot token → re-run `T=$(source tools/lib.sh; r1d_token)`. (ADR-0026)
9. Host can't get its token, everything 401s → uid check or SELinux → `/data/r1os/auth-off` is the way back in. (ADR-0026/0036)
10. adb lists the R1 twice → USB + wireless debugging → tools prefer USB; `ANDROID_SERIAL` picks one. The wireless port changes whenever wireless debugging restarts; `adb_ready` re-finds it by mDNS. (CLAUDE.md)
11. `adb root` over USB drops USB adb (Sept 2026 vendor) → Rabbit's CUA USB mode → `ro.vendor.cua.enable=0`, set as a product property in `r1os.mk` (the vendor sets it before any .rc runs). Workaround: TCP adb, then `adb root`. (research 2026-10-09)
12. `cmd notification post` fails as root → root has no package → `adb unroot`, post, `adb root`.
13. Any computer with a cable gets root adb → TrebleDroid's `ro.adb.secure=0` → open question Q37 / T-026; don't assume adb is authenticated. (open-questions)

## Driving the R1 (UI)
14. Commands seem ignored after a minute → the display sleeps after 60 s and harness input / `r1h screen on` don't count as activity → before each step `MSYS_NO_PATHCONV=1 adb shell input keyevent 59` (Shift). (CLAUDE.md)
15. `r1h launch` does nothing → a sleeping R1 wakes to the glance screen, which swallows launch until a double click goes home; on home a double click opens the agent's space instead → take a screenshot before blind sequences. (CLAUDE.md)
16. A test utterance lands in the owner's real conversation → holds use the real mic and current chat (ADR-0057) → `/harness/say` in a fresh test chat, delete after. (CLAUDE.md)
17. The first press after waking also opened something → wake guard: gestures are ignored while the shell is hidden and for 700 ms after; a double tap whose first press woke the screen is marked `woke: true`. (r1d.md §5)
18. A measurement held the R1 in a state while the owner was using it, or a test's taps mixed with theirs → scripts kept sending input → stop at the owner's input: `tools/eval-control.py` waits for two minutes without `hw` wheel or button events. (T-148, T-157)
19. The R1 shut itself down at 0% mid-run while plugged into a computer → a computer's USB port (500 mA) can't keep up while agents work in apps (about 1% every 2.5 minutes) → watch the level on long runs; `eval-control.py` lets the screen sleep below 20% and waits for 50%. (T-144)

## r1d processes
20. `adb shell` hangs after starting r1d by hand → the child holds the shell's fds → redirect all of them. (CLAUDE.md)
21. `pkill -f r1d` kills the wrong thing → `-f` matches the `adb shell` command line itself → `pkill -x r1d`. (CLAUDE.md)
22. r1d won't stay stopped → init restarts it within ~1 s; that's how deploys work → to copy `core.db` cleanly you must stop the supervisor too; for backups prefer `GET /api/backup` (`VACUUM INTO` snapshot). (research 2026-10-08)
23. Requests stall, stuck `cmd` processes pile up → SELinux refused the `system_server → r1d` binder call-back, so `cmd`/`input`/`am` hang after doing their work → `tool.rs` runs them with a deadline and kills them; run `cmd` itself, not the `input`/`am` scripts (killing the script leaves `cmd`). The same pattern once made `screencap` wait out its 8 s deadline. (`crates/r1d/src/tool.rs`)
24. `svc power` fails from r1d → it starts `app_process`, which r1d's domain may not run → `service call power` (Android 15 codes, SDK 35 only). (r1d.md `/api/power`)
25. r1d died on the first link dial → iroh's DNS resolver and `netdev` ask Android's JVM via `ndk-context`; with `panic = "abort"` the caught panic killed r1d → give iroh explicit DNS servers (the host reports them) and call `ndk_context::initialize_android_context(null, null)` at start; keep `panic = "abort"` (unwinding cost 3.2 MB). (research companion-link)
26. A zombie child of r1d for its lifetime → `( … ) &` in the wrapper became r1d's child after `exec` → detach twice so init adopts it. (`device/aosp/vendor_r1os/bin/r1os-r1d`)

## SELinux and labels (ADR-0035)
27. r1d can't use files pushed by hand → files moved from `/data/local/tmp` keep `shell_data_file`, and init can't relabel them (AOSP refuses `relabelfrom`) → `restorecon -RF` (`-F` because a directory's stored hash otherwise skips it).
28. r1d can't read a test WAV → r1d can't read `/data/local/tmp` → `cp` (not `mv`) clips into `/data/r1os/models/stt/`.
29. A feature fails silently after an image update → missing rule → `logcat -b all -d | grep avc: | grep -E "r1d|r1os"` (`permissive=0` lines); `adb root` then `setenforce 0` gets past it until the next image (a device change: ask). Policy-only rebuild: `build-image.sh layer policy` (~7 min).
30. A box program that runs code from `/tmp` is refused → AOSP's tmpfs-execution neverallow → run from the home instead. (ADR-0035)
31. CLI tools misbehave on refused ioctls → refused ioctls return `EACCES`, not `ENOTTY`/`EOPNOTSUPP` → the box's terminal/FICLONE/FIONBIO ioctls are allowed, not `dontaudit`. (ADR-0035 update)
32. Agent locked out of `/work/.sessions` → it was root-owned on r1os's image → r1d hands `.sessions` to uid 2950 (0700) every session; don't change that. (agents.md §4)

## Host app, image and signing
33. Host janky after a deploy, `status=verify` → a debuggable build is never AOT-compiled, and a fresh install restarts on interpreted code → build `release` (default); `deploy.sh --host` runs `cmd package compile -m speed -f dev.r1os.host` and restarts it after; r1d compiles it at start if needed. ART's interpreter was 46% of the main thread opening a space. (T-100, T-106)
34. Aurora's signature broken → preopting a privileged app rewrites its dex (`LOCAL_PRIVILEGED_MODULE`) → leave the APK alone (export checks it); the host isn't preopted either. (roadmap R4, T-100)
35. Image's host copy ignored, privileged perms missing → Android keeps the adb-installed copy when its versionCode is higher → keep the image's `versionCode` ≥ the installed one. (`host/app/build.gradle.kts`, versionCode)
36. `tools/build-host.sh` stops with "No platform key" → r1os's signing keys aren't on this machine → run `tools/make-keys.sh` once, or set `R1OS_KEYS` to where they are. Never copy them into the repo. (ADR-0062)
37. `ro.build.tags` reads `dev-keys`, not `release-keys` → that tag comes only from `sign_target_files_apks`, which a system-only build doesn't run → expected; the APKs are still signed with r1os's keys. (ADR-0062)
38. The R1 boot-loops after flashing an image with changed keys → a shared uid whose packages carry two certificates throws in `system_server` → before flashing, group every APK in the built system (and inside its APEXes) by `sharedUserId` and check each group has one certificate; keep the previous image to flash back. (ADR-0062)
39. Host shows nothing on a fresh /data → no WebView provider selected → the r1d wrapper selects `com.android.webview` once boot completes.
40. Status bar pulled down on every press → Android's mic privacy chip (r1d opens the mic on every press) → `device_config put privacy camera_mic_icons_enabled false`. (`r1os-r1d` wrapper)
41. Home (Launcher3) crash-loops on a plain GSI → Rabbit's vendor doesn't declare `android.software.app_widgets` → `r1os-features.xml`. (research 2026-10-08)
42. /data only 2.8 GB after a wipe → Rabbit's `userdata.img` + a GSI ignoring fstab `resize` → `tools/grow-data.sh` (writes: ask).
43. Slot b never boots after taking Rabbit's update → Virtual A/B left `vendor_b` at 0 bytes once /data was wiped → after any Rabbit update, check `vendor_<slot>` has a real size in fastbootd before wiping. (research 2026-10-09)
44. Magisk "Incomplete Magisk install" → patching from the computer leaves `/data/adb/magisk` empty → `install-module.sh` copies the APK's assets; a boot-looping module: `adb wait-for-device shell magisk --remove-modules` (no volume keys for safe mode). (`install-module.sh`)
45. Boot took 54 s before Zygote → TrebleDroid's `rw-system.sh` grepped all of `nvdata` for an Oppo model (38 s) → restricted to Oppo/Realme/OnePlus vendors. The first boot after any flash waits ~31 s more for `odsign`. (T-099)
46. Verbose boot spammed the screen → each frame it drew made MediaTek's display driver log 2 lines, which it drew → it drops its own process's lines and redraws ≤ every 150 ms. Its flag is a file on /metadata because persistent props load only after /data. (T-101)
47. A doze screen never shows on the R1 → its panel runs in video mode with only suspend and resume hooks, so it can't keep a picture with the CPU asleep → asleep is plain sleep; SystemUI's doze is turned off. A doze-component overlay installed in /data doesn't take either: DreamManagerService reads it at boot, before /data's overlays apply. (T-157, T-159)

## Agents and CLIs
48. A "no-tools" build still had MCP tools, slash commands and skills → `--tools ""` alone loads claude.ai connectors → no-tools builds pass `--tools "" --strict-mcp-config --restricted --disable-slash-commands --no-session-persistence`. (research 2026-10-08)
49. Skills don't load in chats → `--disable-slash-commands` turns skills off too → chats don't pass it; a message starting with "/" is prefixed as the user's words (`skills::not_a_command`; "/cost" was answered as words). (agents.md §3.6)
50. Connector tools invisible in chats → deferred behind ToolSearch, which chats don't have → `ENABLE_TOOL_SEARCH=false`. (r1d.md connections row)
51. Codex lists no tools ("Code Mode is unavailable") → missing `codex-code-mode-host` → `tools/agent-box.sh codex-host`, `-c features.code_mode_host=true`. App builds through Codex deliberately run without it (tools fail closed). (`agent.rs`)
52. Claude Code fails on Alpine → needs `libgcc libstdc++ ripgrep` and `USE_BUILTIN_RIPGREP=0`; util-linux `setpriv` installs as `/bin/setpriv`. (research 2026-10-08)
53. News questions time out → AP refuses Claude Code's fetcher, NPR's CDN drops it after 60 s → agents use `read_feed` (r1d's fetcher). (agents.md §5)
54. Agent treats web-search results as an injection → results end with a "cite sources" reminder → the prompt says to name the source in a few words. (research 2026-10-08)
55. Cold replies ~10 s → the CLI's own start (~5.6 s) dominates; warm turns ~1.6 s to first word → warm on hold start (`POST /api/chats/warm`); keep idle timeouts (15/3 min). The first turn holds ~32k tokens of context, a good part of it r1d's tool list. (research baseline)
56. Signing in an OAuth MCP server from the box fails ("stdin isn't a terminal") → `claude mcp login --no-browser` needs a TTY → add such servers on claude.ai as custom connectors. Google won't sign in on the R1 alone (device flow lacks Gmail scopes, WebViews refused). (ADR-0052)
57. A stopped Claude story taken as finished → the CLI didn't know → r1d prepends a "the user stopped your last answer" line to the next message (in memory only). (agents.md §3.5)
58. Codex context estimates too high → `turn.completed` usage counts every model call → r1d estimates transcript/4 + 8k. (agents.md §3.3)
59. A fact added by one agent isn't known to another → prompts are fixed when a process starts → it reaches warm processes only after they restart. (agents.md §5)

## The app window and the browser (ADR-0061)
60. An agent's steps landed out of order (typing before the tap it followed: "McDonald'sMcDonald's") → the CLI runs tools sent together in parallel, and the host interleaved them → r1d runs app window and browser tools one at a time, in the order they came (`hub.app_window.turn`, `hub.browser.turn`); keep it so. (T-144)
61. Words typed into a web page inside an app showed but weren't heard (Enter searched for nothing) → text set through accessibility never reaches the page's key handlers → the host types real keys into web fields (tap, select all, delete, keys); time, date, colour and range fields are set instead, and typing reads the field back. (T-144)
62. A read's words don't match the picture on a web page inside an app, or a tap hits a stale place → Chrome builds a page's tree only when asked and can leave it stale for seconds → taps go where the element is now (re-found, brought into view), the host nudges Chrome, and the agent is told to go by the picture (`app_click` at x,y). (T-144, T-146)
63. A test's tap on a choice added it to the real cart → the tap went where Chrome's lagging tree put the choice, which was the "Add to cart" bar, and the guard checked the element named rather than what was under the finger → web elements are clicked by accessibility (that element, wherever it is), never tapped by place; a tap by place checks the buttons on the screen, and in a test one on or within 32 px of an add-to-cart or a final step is refused; `eval-control.py` counts the cart before and after each food scenario. (T-144)
64. Spotify says "You're offline" in the window while the R1 is online → Android cut it off the network in the background, and its offline state can outlast the cut → it usually reconnects within a minute: `app_wait`, read again; still offline, `app_open {restart: true}` (it loses its place). (T-145)
65. The window went black after 30 s with nothing tapping into it → its display has its own power group, which slept on the screen timeout → the host holds a screen wake lock tied to the window's display alone while the R1's screen is on. (T-113)
66. With a window open, the R1's button wouldn't wake it → the window's power group kept Android "Awake" with the R1's screen off, and Android ignores a wake key's wake while awake → the window's display goes to sleep with the R1's screen; r1d wakes it through `cmd power wakeup`, falling back to the wake key. (T-149)
67. The R1's own screen froze ~3 s at each agent step → reads of the app ran on the host's UI thread → reads, finds and pictures run on the window's own thread; the host's 99th-percentile frames went from 1,950 to 73 ms. Never read the window on the UI thread. (T-143)
68. Closing apps left in the background closed nothing (and the log said it had) → from Android 14, `killBackgroundProcesses` kills only the caller's own processes → the host holds `KILL_ALL_BACKGROUND_PROCESSES` (a signature permission) and logs only what's actually gone. (T-150)

## Speech, motor, hardware
69. STT takes ~35 s to load → `.onnx` re-optimizes a ~10k-node graph every load → use the `.ort` copy (~13 s; `fetch-speech.sh` + `optimize-stt.py`, ORT minor version must match the R1's 1.28.x). A hold before it's ready gets "Still warming up…". (r1d.md §4.1)
70. Last word cut off → the user lets go on the last word → r1d keeps recording 400 ms after release (60 ms when you'd already stopped talking).
71. Camera motor ends up somewhere other than reported → the driver drops a write made mid-move and believes the latest request → one move at a time, wait for the read-back; re-home against the 0° stop. The camera HAL also writes the node (opening camera 0 → 180, closing → 90). (ADR-0046)
72. Host can't write the motor node → Rabbit's policy labels it `sysfs_motor`, writable by platform/system apps → r1d drives it (`POST /api/motor`). (Q9b)
73. Wheel ticks appear on button presses → mechanical ghost ticks → keep the 120 ms guard (`wheel_suppressed`). (Q12)

## Performance and power
74. ~15% of a core with the screen off → looping animations ran from attach to detach, and a stopped activity keeps its views attached → loop only while visible (`onVisibilityAggregated`) and hold still while `Dim.dimmed` (Android dims 7 s before timeout and tells no app). (architecture §7, T-032, T-036)
75. A character on screen cost ~22% of all cores → 60 fps idle frames, each recomposing the display (~18 ms host CPU a frame at idle clocks) → idle life ticks at 15/s in whole pixels; 60 fps only for what answers you. Home idle 24% → 6.4%. (T-094)
76. Glance unlock jittered → restarting an animation per wheel tick with overshoot → a retargetable `Spring` that keeps velocity. (`host/…/ui/Spring.kt`)
77. Opening a page took ~400 ms of main thread → a full rebuild per r1d answer plus double measuring → batch answers (`ListPage.soon`/`fetch`), show newest 8 chats / 40 messages, slide in after the first frame as one GPU layer. One GPU layer for a page that changes while sliding was worse. (T-041, T-106)
78. An app in the window cost a lot of CPU and memory → it drew at the panel's 60 fps, and apps opened for the agent stayed running → the window's display runs at 15 fps; an app opened only for the agent closes a minute after the window; cached apps close after 10 idle minutes. In web-heavy apps most of what's left is Chrome's renderer running the page's scripts, which doesn't follow the frame rate. (T-148, T-150)
79. Idle link costs ~2% of a core → iroh's QUIC network checks to four relays every 20–26 s → bind with the one relay last settled on (0.8%). (T-035)
80. Drain from 100% reads zero → the charge counter sits at full until the level drops; Android never suspends with adb on USB → unplug for drain figures (`r1h power mark` / `diff`). (research baseline)
81. Standby ~1.6%/hour, 86% of wakeups the Wi-Fi driver → T-037's power policy (no background scans, no location, cell radio off without a SIM, Doze sooner). (research baseline)

## Build environment
82. Android 15 build refuses `WITHOUT_CHECK_API=true` → keep API checks; use `SKIP_ABI_CHECKS=true` and `m --skip-soong-tests`. The AOSP tree's sync alone is ~60–90 GB: give it a large disk of its own, and the build a lot of RAM plus swap. The image builds in Linux (WSL works). (`tools/build-image.sh`)
83. Something an agent fetched into `AppData\Local` isn't there in the owner's terminal, or uv says "Missing expected target directory for Python minor version link" → an agent run by the Claude desktop app (a packaged app) has its writes under `AppData\Local` redirected to `AppData\Local\Packages\Claude_…\LocalCache\Local`, which only that app's processes see, and junctions made there name the real path → have the owner fetch from their terminal what they'll run there; `AppData\Local\Temp` isn't redirected. (2026-10-14, `crates/r1-usb/src/folders.rs`)
84. CI's clippy fails on code that passes `cargo clippy` locally ("for loop over a single element", "using `chunks_exact` with a constant chunk size") → CI runs the newest stable Rust (`dtolnay/rust-toolchain@stable`), and each release adds lints. Install the version it ran beside yours (`rustup toolchain install 1.99.0 --profile minimal -c clippy`) and run `cargo +1.99.0 clippy --workspace --all-targets -- -D warnings` (with `RUSTFLAGS` empty). Keep fixes within the workspace's `rust-version` (`as_chunks` is 1.88+). (2026-10-14)
