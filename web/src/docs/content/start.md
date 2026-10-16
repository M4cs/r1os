---
title: Install
description: What you need, and how to install r1os.
group: start
order: 2
---

# Install

r1os runs on the Rabbit R1. There's no prebuilt image or over-the-air update yet: you build the image from [r1os's source](https://github.com/M4cs/r1os) and flash it yourself.

> [!CAUTION]
> **Read this first.** Installing r1os means unlocking the bootloader and flashing a system image. The R1 has no volume keys and no recovery menu, so a write to the wrong partition can brick it. Back up first, and follow each step exactly.

## Before you start

- **You'll need** an R1 whose bootloader you can unlock, Linux (or WSL) with room for the Android source (~60–90 GB), and a Claude or ChatGPT subscription. The full list is [below](#what-you-need).
- **What it costs:** Rabbit says unlocking voids the warranty, and flashing erases everything on the R1. You can [go back to stock rabbitOS](#if-it-gets-stuck) later.
- **The steps:** [unlock and back up](#unlock-and-back-up), [build](#build), [flash](#flash), [deploy](#deploy), then [first boot](#first-boot).
- **No R1, or not ready to flash?** [The simulator](/docs/develop#the-simulator) runs r1os's daemon and shell on your computer, with nothing to flash.

> [!WARNING]
> This guide hasn't been run end to end from a stock R1 yet. If a step doesn't match what you see, stop and [open an issue](https://github.com/M4cs/r1os/issues).

The [companion](/docs/companion#installing-r1os-over-usb) is getting a wizard that does these steps for you. It hasn't run on an R1 yet, so for now this guide is the way.

## What you need

- **A Rabbit R1** whose bootloader you can unlock.
- **Linux for the image** (WSL works), with plenty of RAM, swap and disk: the Android source sync alone is ~60–90 GB.
- **The tools:** Android's platform tools and NDK, Rust with the `aarch64-linux-android` target, Bun 1.3+, JDK 17+, `uv` and `openssl`. Set `ANDROID_HOME` to your Android SDK, and `ANDROID_NDK_HOME` if the NDK isn't inside it. The scripts in `tools/` are bash; on Windows, use Git Bash or WSL. Python scripts use `python3`; on Windows, `py -3`.
- **A Claude or ChatGPT subscription** for your agents.

## Unlock and back up

1. **On rabbitOS,** take Rabbit's updates: r1os keeps the device's kernel and vendor, and is tried on Rabbit's September 2026 build. Turn on developer mode and request the unlock in Rabbit's OS3 portal, under settings › r1 › device modification. Rabbit says this voids the warranty.
2. **Back up** the device's unique partitions, the IMEI and radio calibration among them. `tools/backup.sh` copies them into `backups/` with checksums; it needs root, so on stock rabbitOS read them with [mtkclient](https://github.com/bkerler/mtkclient). Save that dump to a folder of its own, `backups/<date>-stock/`, with the partitions listed at the top of `tools/backup.sh`, then run `sha256sum * > SHA256SUMS` in it: `tools/flash-gsi.sh` won't run until it finds that file. Don't set `NO_BACKUP_OK`. Keep the backup private.
3. **Unlock:** `adb reboot bootloader`, then `fastboot flashing unlock`, and confirm with the side button.

## Build

Make r1os's signing keys once, with `tools/make-keys.sh`. They live outside the repository (`R1OS_KEYS` sets where). Back them up offline and never share them: whoever holds them can sign apps with r1os's system powers.

Then fetch and build what the image needs:

```bash
tools/fetch-speech.sh    # speech-to-text, needed to build r1d
tools/fetch-models.sh    # System One and the translator (~2.2 GB; TRANSLATOR=0 skips it)
tools/fetch-aurora.sh    # Aurora Store, preinstalled in the image
tools/build-host.sh      # the host app, signed with your platform key
tools/build-bootlog.sh   # verbose boot
```

A first build needs no R1. Its boot animation is a still of r1os's characters, and your R1 shows your own agents from the boot after r1os has drawn them. Once an R1 runs r1os, `tools/r1h bootanim default` puts the animated one in `out/bootanim` for the next build. Then, on Linux, from the repository:

```bash
sudo R1OS_KEYS=/path/to/.r1os/keys AOSP_TREE=/big/disk/aosp15 bash tools/build-image.sh
```

`R1OS_KEYS` is the folder `tools/make-keys.sh` made: `~/.r1os/keys` for the user who ran it, or `/mnt/c/Users/<you>/.r1os/keys` in WSL if you made them on Windows. `AOSP_TREE` is where the Android source goes (~60–90 GB; the default is `/aosp15`).

It syncs AOSP's Android 15, adds TrebleDroid's patches and r1os's layer, and writes `out/image/r1os-system-a15-<date>.img` with its checksum.

## Flash

Before you flash, check that:

- [ ] your backup is in `backups/<date>-stock/`, with its `SHA256SUMS`;
- [ ] the image is built, as `out/image/r1os-system-a15-<date>.img`;
- [ ] the latest release of [Rabbit's published stock firmware](https://github.com/rabbit-hmi-oss/firmware/releases) is unzipped as `<rabbit-firmware>` (only its vbmeta images and empty userdata are used);
- [ ] the R1 is in fastboot, and `fastboot devices` lists it.

> [!CAUTION]
> **This erases everything on the R1.** `tools/flash-gsi.sh` refuses to run without a backup. It turns verified boot off, erases all data, removes the stock product partition, and flashes r1os to the active slot's system partition.

```bash
tools/flash-gsi.sh out/image/r1os-system-a15-<date>.img <rabbit-firmware>
```

## Deploy

Once it has booted:

```bash
adb root
tools/grow-data.sh           # /data to the full partition
tools/deploy.sh              # r1d, the shell, speech and the models
tools/agent-box.sh install   # Alpine, with Claude Code and Codex
```

## Keep it safe

- r1os only ever writes `boot_*`, `dtbo_*`, `vbmeta*`, `system_*`, `product_*`, `super` and `userdata`. Rabbit's kernel, vendor and bootloader stay.
- Never relock the bootloader while it runs a modified image.
- Never let it take a Rabbit OTA: it would replace r1os.
- USB adb isn't authenticated yet: a computer with a cable gets root, and with it r1d's token and the CLIs' logins. An approval prompt is planned.
- Don't send the R1 into fastbootd (`fastboot reboot fastboot`). One trip there and back, with nothing written, left an R1's slot unbootable until it was flashed again.

## If it gets stuck

- **It starts Android's recovery** ("Can't load Android system", "Try again or factory data reset"). Don't choose factory data reset: it erases your data and doesn't fix a system that won't start. Choose try again once. If it comes back to recovery, stop there and [open an issue](https://github.com/M4cs/r1os/issues) with what the screen says. Getting it to start again depends on what broke.
- **Back to stock rabbitOS:** both of these erase everything on the R1.
  - [Rabbit's web flashing tool](https://rabbit-hmi-oss.github.io/flashing/) restores rabbitOS v0.8.293, which then takes Rabbit's updates. It's reported to relock the bootloader; that isn't confirmed.
  - From the command line, with the R1 in fastboot and Rabbit's `rabbit_OS_v0.8.293.zip` extracted: `tools/flash-stock-base.sh <that folder>` puts v0.8.293's kernel, `dtbo`, `vbmeta` and `super` on slot a, with verified boot off and the bootloader left unlocked. It never touches the boot chain or the R1's calibration, and it refuses to run without a backup of this R1.
- **Your backup** is what restores the IMEI and radio calibration, if anything ever overwrites them. r1os never writes those partitions. Putting them back is a job for mtkclient, done by hand.

## First boot

Onboarding runs on the wheel and button: Wi-Fi, your first agent, then Claude or Codex, signed in by scanning a QR code with your phone. Then hold the button and say something.

Next: [Using r1os](/docs/using), [Android apps](/docs/android) and the [Companion](/docs/companion). No R1? [The simulator](/docs/develop#the-simulator) runs r1os's daemon and shell on your computer.
