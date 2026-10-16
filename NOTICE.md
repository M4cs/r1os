# Third-party notices

The packages each binary and bundle ships, with their licence texts, are listed by `tools/notices.py` (`out/notices/`, made for each release).

r1os's own code is under the MIT licence ([LICENSE](LICENSE), [ADR-0064](docs/decisions/0064-r1os-is-open-source-under-mit.md)). It builds on work by others, each under its own licence, below.

## r1ptt (MIT)

Several scripts in `tools/` are adapted from [r1ptt](https://github.com/windoze95/r1ptt):
`lib.sh` (helpers, the vbmeta flag patch), `fastboot-entry.py`, `flash-gsi.sh`, `flash-stock-base.sh`,
`root.sh` and `flash-boot.sh`. The Magisk "additional setup" step in `install-module.sh` comes from
r1ptt's `provision.sh`. So is the R1 knowledge they encode:
- the side button is `mtk-kpd` scancode 116;
- the wheel is `och1970_holl_key` with KEY_UP/KEY_DOWN;
- Rabbit's USB "CUA" mode must be turned off on non-stock systems;
- the data wipe sequence.

```
MIT License

Copyright (c) 2026 Julian Dice

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

r1ptt in turn credits:
- ClawPTT and rabbit-r1-openclaw (MIT, mthelpme) for the side-button facts;
- r1_escape for the MediaTek preloader FASTBOOT handshake;
- TurboTheTurtle/rabbit-r1-firmware for the partition layout and stock-baseline sequence.

## ponces/treble_aosp (Apache-2.0)

r1os's system image build (`tools/build-image.sh`, `device/aosp/manifest/`) follows the recipe in
[ponces/treble_aosp](https://github.com/ponces/treble_aosp) (android-14.0 branch, archived 2026-04),
Copyright ponces, under the Apache License 2.0. `device/aosp/manifest/remove.xml` is copied from it
unchanged, and `r1os.xml` is adapted from its `build/default.xml`. At build time, the script clones
that repository to apply its `trebledroid` patch set (TrebleDroid's patches; ponces' own aren't
used).

## TrebleDroid (not vendored)

r1os's image is built on TrebleDroid's GSI work by phhusson and contributors: `device_phh_treble`,
`vendor_hardware_overlay`, `vendor_interfaces` and the TrebleDroid patches, fetched at build time
under their own licenses. AOSP itself is under the Apache License 2.0 and other open-source licenses.

## Aurora Store (GPL-3.0-or-later; not vendored)

r1os's image ships Aurora Store, an open-source client for Google Play's catalog by Aurora OSS
(https://gitlab.com/AuroraOSS/AuroraStore), unmodified: its release APK (the `preload` build, 4.8.4), signed by
Aurora OSS, fetched at build time by `tools/fetch-aurora.sh` and installed as a privileged app (ADR-0013). Its
source is at that address, under the GNU General Public License v3.0 or later. r1os's own code only starts it
(`android.intent.action.SHOW_APP_INFO`) and doesn't link it.

## Others (not vendored)

- **Rabbit Inc.:** official developer mode, the WebUSB flash tool, stock firmware images, and the GPL kernel source ([rabbit-hmi-oss](https://github.com/rabbit-hmi-oss)).
- **David Buchanan's [rabbit_r1_boot_notes](https://github.com/DavidBuchanan314/rabbit_r1_boot_notes):** reverse-engineering notes on the R1 boot chain.
- **Andy Yan's LineageOS GSI builds; Magisk by topjohnwu; mtkclient by bkerler.**

## Rubik (SIL Open Font License 1.1)

The shell's typeface (ADR-0021): `host/app/src/main/res/font/rubik_*.ttf` and `shell/public/fonts/Rubik-*.ttf`, copied unmodified from AOSP's `external/google-fonts/rubik`. Copyright 2015 The Rubik Project Authors (https://github.com/googlefonts/rubik). The license is in [`third_party/rubik/OFL.txt`](third_party/rubik/OFL.txt).

## The R1's fonts in the companion and the site (SIL Open Font License 1.1, Apache License 2.0)

The companion draws the R1's screen with the R1's own typefaces: `companion/src/fonts/r1/*.woff2`, converted unmodified from AOSP 15's copies (the site uses them through the companion). Each family's licence, with its copyright, is beside them in [`companion/src/fonts/r1/licences/`](companion/src/fonts/r1/licences/):

| Font | Licence | From AOSP |
|---|---|---|
| Carrois Gothic SC | OFL-1.1 | `external/google-fonts/carrois-gothic-sc` |
| Coming Soon | Apache-2.0 | `external/google-fonts/coming-soon` |
| Cutive Mono | OFL-1.1 | `external/google-fonts/cutive-mono` |
| Dancing Script | OFL-1.1 | `external/google-fonts/dancing-script` |
| Droid Sans Mono | Apache-2.0 | `frameworks/base/data/fonts` |
| Noto Serif | OFL-1.1 | `external/noto-fonts/notoserif` |
| Roboto | Apache-2.0 | `external/roboto-fonts` |
| Roboto Flex | OFL-1.1 | `external/roboto-flex-fonts` |
| Source Sans Pro | OFL-1.1 | `external/google-fonts/source-sans-pro` |

The companion's Rubik (`companion/src/fonts/rubik_*.ttf`) is the same as the shell's, above. The website's
(`companion/src/fonts/rubik_*.woff2`) is those files repackaged as WOFF2 by fontTools, every glyph and table kept.

## Phosphor Icons (MIT)

`packages/icons` redistributes [Phosphor Icons](https://phosphoricons.com) (`@phosphor-icons/core` 2.1.1); its notice is in [`packages/icons/THIRD_PARTY.md`](packages/icons/THIRD_PARTY.md).

## On-device models and runtimes (fetched, not in this repository)

`tools/fetch-speech.sh` and `tools/fetch-models.sh` download these, pinned by hash, and `tools/deploy.sh` puts them on the R1. Each stays under its own licence.

- **Speech to text:** Banafo's Kroko streaming English model (`sherpa-onnx-streaming-zipformer-en-kroko-2025-08-06`). Its archive's README says only "See license at https://huggingface.co/Banafo/Kroko-ASR". That page puts Banafo's community models under CC-BY-SA, and r1os uses this one on those terms. The page's `LICENSE` file is empty and doesn't name this build, and an earlier Banafo demo said its models were under the Coqui Public License, for non-commercial use, so check the terms yourself before any commercial use. r1os converts it to ONNX Runtime's `.ort` format.
- **System One:** sentence-transformers' all-MiniLM-L6-v2 (Apache-2.0).
- **The Translator:**
  - **Listening:** OpenAI's Whisper base (MIT), as exported by sherpa-onnx.
  - **Translating:** Helsinki-NLP's OPUS-MT models (CC-BY-4.0), in Xenova's ONNX conversions.
  - **Speaking:** Piper voices, as packaged by sherpa-onnx, with espeak-ng's data (GPL-3.0-or-later). Each voice is under its dataset's terms, in its `MODEL_CARD`:

    | Voice | Dataset terms |
    |---|---|
    | German, Spanish, Portuguese | CC0 |
    | French | CC-BY 4.0 |
    | English | the Blizzard 2013 Lessac licence |
    | Arabic, Italian | at their dataset's URL |
    | Russian | unknown |
    | Hindi | CC BY-NC-SA 4.0 (non-commercial) |
    | Chinese | non-commercial use only |

    r1os isn't a commercial product, so the non-commercial voices are fetched with the rest. Anyone using r1os commercially should leave them out of `tools/translator-models.txt`.
- **Runtimes:** sherpa-onnx (Apache-2.0), with ONNX Runtime (MIT).
