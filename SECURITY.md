# Security policy

r1os runs with root on a device people carry everywhere, and it holds their agents' sign-ins, so a hole matters. Thank
you for looking.

## Reporting a vulnerability

Report it privately, through GitHub: the repository's **Security** tab → **Report a vulnerability**. Don't open a
public issue, pull request or discussion for it.

Say what's affected (the r1os image version from Settings › About, the companion's version, or a commit), how to
reproduce it, and what someone could do with it. Never include a device's serial number or IMEI, or anyone's
sign-in codes; describe them instead.

You'll get an answer within a week. Once it's fixed, the fix is published with credit to you, unless you'd rather not
be named.

## In scope

- **r1d** (`crates/r1d`), the daemon that runs as root on the R1: its HTTP and WebSocket API on `127.0.0.1:7171` and
  its per-boot token; the agents' jail (their uid, seccomp, SELinux domain); what agents and apps can reach through
  it.
- **The servers r1d opens on the local network:** the sign-in page for an agent's account (port 7180, for up to 10
  minutes) and file sharing.
- **The host app** (`host/`) and its WebViews.
- **The companion** (`companion/`) and the link between it and an R1 (`crates/r1-link`, over iroh and n0's relays).
- **The installer** (`crates/r1-usb`): drivers, the backup, the flash. Especially anything that could write a partition
  it shouldn't.
- **The r1os system image** built from `device/aosp`.

## Known and being worked on

These are known. A report is still welcome if you find a way to make them worse.

- **USB means root:** anyone with the R1's USB cable and adb gets root, because the image is a userdebug build.
  Requiring adb authorization, or a user build, is planned before a general release.
- **One user for all agents:** every agent runs as the same user and shares one login home.
- **The sign-in page:** it's served over plain HTTP on the local network, behind a one-time token.
- **No mic or camera indicator** for Android apps: r1os turns Android's off, since r1d opens the mic on every button
  press.

Not a bug: agents reach the local network as well as the internet, by design, so they can use your home's devices.

The whole list, with what leaves the R1 and who can reach it, is the site's privacy page
(`web/src/docs/content/privacy.md`).

## Out of scope

- Rabbit's own firmware, bootloader and kernel, which r1os doesn't change. Report those to Rabbit.
- Anthropic's and OpenAI's CLIs and services. Report those to them.
- Attacks that need the R1 unlocked in hand with r1os's own permission already granted.
