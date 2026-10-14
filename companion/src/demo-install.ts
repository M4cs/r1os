// The demo's pretend R1 on USB, for the install wizard outside Tauri: src-tauri/src/install.rs's commands answered here,
// with their progress events. By default an R1 running r1os (unlocked, with root), so every step can be walked through;
// `?r1=stock` makes it a stock R1, off and locked, whose backup goes through its preloader. It changes mode as a real
// one would (restarts take a few seconds). Made up: nothing here talks to a real R1.

import type { Checked, FlashPlan, FlashWrite } from "./link";

type Emit = (event: string, payload: unknown) => void;
type Mode = "off" | "adb" | "preloader" | "fastboot";

const stockR1 = typeof location !== "undefined" && new URLSearchParams(location.search).get("r1") === "stock";
/** Its identity (a hash of its serial), and its preloader's: that has a serial of its own. */
const ID = "5e0c41b7a9d2";
const AT_POWER_ON = "c07fa1e3b512";
const VENDOR = 354_168_832;
const SYSTEM = 1_972_977_664;
const TOOLS = 8_044_989;
const BACKUPS = String.raw`C:\Users\you\Documents\r1os backups`;

const r1 = { mode: (stockR1 ? "off" : "adb") as Mode, unlocked: !stockR1, slot: "b", r1os: stockR1 ? null : ("0.2.17" as string | null) };
const pc = { tools: null as string | null, drivers: { r1: false, mtk: false }, backupTools: false, backups: {} as Record<string, string>, stock: false };
const flash = { read: null as number | null, checked: null as Checked | null, plan: null as FlashPlan | null };
/** What a running wait or backup stops with. */
let stop: (() => void) | null = null;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
function fail(why: string): never {
  throw new Error(why);
}
function forget() {
  flash.checked = null;
  flash.plan = null;
}
/** Into [mode] after [ms], as a restart would. */
function later(mode: Mode, ms: number, then?: () => void) {
  r1.mode = "off";
  setTimeout(() => {
    r1.mode = mode;
    then?.();
  }, ms);
}
/** Sleeps that a stop cuts short. */
function stoppable(): { wait: (ms: number) => Promise<void>; done: () => void } {
  let stopped = false;
  stop = () => (stopped = true);
  return {
    wait: async (ms) => {
      await sleep(ms);
      if (stopped) fail("stopped");
    },
    done: () => (stop = null),
  };
}

function readStatus(identity: unknown) {
  return identity === ID && flash.read ? { read: true, at: flash.read, merge: "none", vendor_a: VENDOR, vendor_b: VENDOR } : { read: false };
}

function planFor(system: string): FlashPlan {
  const name = system.split(/[\\/]/).pop() ?? "system.img";
  const writes: FlashWrite[] = [
    ...["vbmeta", "vbmeta_system", "vbmeta_vendor"].flatMap((v) => ["a", "b"].map((s): FlashWrite => ({ what: "flash", part: `${v}_${s}`, bytes: 4096 }))),
    { what: "flash", part: "super", bytes: 2 * SYSTEM + 1_052_672 },
    { what: "erase", part: "userdata", bytes: 0 },
    { what: "flash", part: "userdata", bytes: 7_356_684 },
  ];
  return {
    r1: ID,
    slot: r1.slot,
    system: name,
    system_bytes: SYSTEM,
    lines: [
      "Verification off in vbmeta, vbmeta_system and vbmeta_vendor, on slots a and b",
      `super: ${name} as the system on slots a and b, the R1's own vendor kept where it is, Rabbit's product removed`,
      "ERASE userdata (everything on the R1) and write Rabbit's empty userdata",
      `Untouched: the kernel (boot), vendor, dtbo, the bootloader, and everything that's this R1's own. Boots slot ${r1.slot}.`,
    ],
    writes,
    bytes: writes.reduce((n, w) => n + w.bytes, 0),
    confirm: "ERASE",
  };
}

/** The install's commands, or null for any other. */
export function installDemo(cmd: string, args: Record<string, unknown>, emit: Emit): Promise<unknown> | null {
  const run = COMMANDS[cmd];
  return run ? run(args, emit) : null;
}

const COMMANDS: Record<string, (args: Record<string, unknown>, emit: Emit) => Promise<unknown>> = {
  async usb_tools() {
    return { version: pc.tools, pinned: "37.0.1", size: TOOLS, terms: "https://developer.android.com/studio/terms" };
  },
  async usb_fetch_tools(_, emit) {
    for (let i = 1; i <= 20; i++) {
      await sleep(90);
      emit("usb-tools-progress", { got: Math.round((TOOLS * i) / 20), total: TOOLS });
    }
    pc.tools = "37.0.1";
    return { version: pc.tools };
  },
  async usb_scan() {
    // The core forgets a check once its R1 is seen out of the bootloader.
    if (flash.checked && r1.mode !== "fastboot" && r1.mode !== "off") forget();
    if (r1.mode === "off") return { tools: !!pc.tools, r1s: [] };
    // Without platform-tools, or in the bootloader without its driver, it's only seen as plugged in.
    const mode = r1.mode === "preloader" ? "preloader" : !pc.tools || (r1.mode === "fastboot" && !pc.drivers.r1) ? "no_driver" : r1.mode;
    return {
      tools: !!pc.tools,
      r1s: [{ id: mode === "preloader" ? AT_POWER_ON : ID, mode, unlocked: mode === "fastboot" ? r1.unlocked : null, slot: mode === "fastboot" ? r1.slot : null, r1os: mode === "adb" ? r1.r1os : null }],
    };
  },
  async drivers_status() {
    return { ...pc.drivers };
  },
  async drivers_install(args) {
    await sleep(2200);
    pc.drivers.r1 = true;
    if (args.mtk) pc.drivers.mtk = true;
    return {
      status: { ...pc.drivers },
      log: "r1: the driver catalogued and signed\nr1: its certificate trusted, its private key deleted\nr1: added and installed\nmtk: Rabbit's installer exited 0",
    };
  },
  async drivers_undo() {
    await sleep(1500);
    pc.drivers.r1 = false;
    return { status: { ...pc.drivers }, log: "r1: the driver removed\nr1: its certificate removed" };
  },
  async backup_status(args) {
    const id = args.identity as string | null;
    return { tools: pc.backupTools, mtk_driver: pc.drivers.mtk, root: BACKUPS, latest: id ? (pc.backups[id] ?? null) : null, skip_phrase: "I understand" };
  },
  async backup_fetch_tools(_, emit) {
    for (const [what, total] of [["uv", 22_310_473], ["mtkclient", 32_229_485]] as const) {
      for (let i = 1; i <= 10; i++) {
        await sleep(110);
        emit("backup-tools-progress", { what, got: Math.round((total * i) / 10), total });
      }
    }
    emit("backup-tools-progress", { what: "python", got: 0, total: 0 });
    await sleep(1500);
    pc.backupTools = true;
  },
  async backup_run(_, emit) {
    const parts: [string, number][] = [["nvram", 5 << 20], ["nvdata", 64 << 20], ["proinfo", 3 << 20], ["md_udc", 22 << 20], ["para", 512 << 10], ["lk_a", 2 << 20], ["boot_a", 32 << 20], ["boot_b", 32 << 20], ["md1img_a", 100 << 20], ["md1img_b", 100 << 20]];
    const total = parts.reduce((n, [, s]) => n + s, 0);
    const date = "20261014-153000";
    const s = stoppable();
    try {
      if (r1.mode === "adb" && r1.r1os && pc.tools) {
        let done = 0;
        for (const [part, bytes] of parts) {
          for (let i = 1; i <= 4; i++) {
            await s.wait(120);
            emit("backup-progress", { part, done: done + (bytes * i) / 4, total });
          }
          done += bytes;
        }
      } else {
        if (r1.mode === "adb") fail("this R1 has no root over adb: it needs r1os's image (adb root) or Magisk's su");
        if (!pc.backupTools) fail("the backup's tools aren't fetched yet");
        if (!pc.drivers.mtk) fail("Rabbit's MediaTek driver isn't installed yet");
        emit("backup-line", "Waiting for the R1 at power-on…");
        await s.wait(3500);
        r1.mode = "preloader";
        for (const line of ["Port - Device detected :)", "Preloader - CPU: MT6765/MT8768t(Helio P35/G35)", "DA - Uploading stage 1 …", "DA - Stage 2 uploaded"]) {
          await s.wait(600);
          emit("backup-line", line);
        }
        for (const [part] of parts) {
          await s.wait(500);
          emit("backup-line", `Dumping partition "${part}"`);
        }
        r1.mode = "off";
      }
      pc.backups[ID] = `${BACKUPS}\\${ID}\\${date}`;
      return { dir: pc.backups[ID], identity: ID, date, parts };
    } finally {
      s.done();
    }
  },
  async backup_stop() {
    stop?.();
  },
  async backup_skip(args) {
    return String(args.typed ?? "").trim().toLowerCase() === "i understand";
  },
  async flash_stock_status() {
    return { ready: pc.stock, bytes: 7_368_972, files: ["vbmeta.img", "vbmeta_system.img", "vbmeta_vendor.img", "userdata.img"], release: "https://github.com/rabbit-hmi-oss/firmware/releases/download/v0.8.293/rabbit_OS_v0.8.293.zip" };
  },
  async flash_fetch_stock(_, emit) {
    const files = ["vbmeta.img", "vbmeta_system.img", "vbmeta_vendor.img", "userdata.img"];
    for (const [index, file] of files.entries()) {
      emit("flash-stock-progress", { file, index, count: files.length });
      await sleep(index === 3 ? 1400 : 400);
    }
    pc.stock = true;
  },
  async flash_read_status(args) {
    return readStatus(args.identity);
  },
  async flash_read_r1() {
    if (r1.mode !== "adb" || !pc.tools) fail("no R1 booted on USB with USB debugging allowed");
    if (!r1.r1os) fail("this R1 has no root over adb: it needs r1os's image (adb root) or Magisk's su");
    await sleep(1800);
    flash.read = Date.now();
    return { ...readStatus(ID), r1: ID };
  },
  async flash_to_bootloader() {
    forget();
    if (r1.mode !== "adb" || !pc.tools) fail("no R1 booted on USB with USB debugging allowed");
    await sleep(300);
    later("fastboot", 5000);
  },
  async flash_leave_bootloader() {
    forget();
    if (r1.mode !== "fastboot" || !pc.drivers.r1) fail("no R1 in its bootloader");
    await sleep(300);
    later(r1.r1os ? "adb" : "off", 7000);
  },
  async flash_enter_fastboot() {
    forget();
    if (!pc.drivers.mtk) fail("Rabbit's MediaTek driver isn't installed yet");
    // As if it were turned off, then plugged in a few seconds later.
    r1.mode = "off";
    const s = stoppable();
    try {
      await s.wait(4000);
      r1.mode = "preloader";
      await s.wait(900);
      r1.mode = "fastboot";
    } finally {
      s.done();
    }
  },
  async flash_enter_fastboot_stop() {
    stop?.();
  },
  async flash_unlock_ability() {
    if (r1.mode !== "fastboot" || !pc.drivers.r1) fail("no R1 in its bootloader");
    await sleep(400);
    return { r1: ID, unlocked: r1.unlocked, ability: 16_777_216 };
  },
  async flash_check() {
    forget();
    if (r1.mode !== "fastboot" || !pc.drivers.r1) fail("no R1 in its bootloader");
    if (!flash.read) fail("nothing's been read from this R1 yet: start it, read it, then bring it back to the bootloader");
    await sleep(1200);
    if (!r1.unlocked) fail("the bootloader is locked: unlock it first (Rabbit's portal, then `fastboot flashing unlock`)");
    flash.checked = { r1: ID, slot: r1.slot, vendor_bytes: VENDOR, other_vendor_bytes: VENDOR, merge: "none" };
    return flash.checked;
  },
  async flash_prepare(args, emit) {
    if (!flash.checked) fail("check the R1 first");
    flash.plan = null;
    if (!pc.stock) fail("Rabbit's files aren't fetched yet");
    const total = 2 * SYSTEM;
    for (let i = 1; i <= 24; i++) {
      await sleep(200);
      emit("flash-prepare-progress", { done: Math.round((total * i) / 24), total });
    }
    flash.plan = planFor(String(args.system));
    return flash.plan;
  },
  async flash_prepared() {
    return { checked: flash.checked, plan: flash.checked ? flash.plan : null };
  },
  async flash_run(args, emit) {
    if (String(args.typed ?? "").trim() !== "ERASE") fail("not flashed: type ERASE to go ahead");
    const plan = flash.plan;
    if (!flash.checked || !plan) fail("nothing to flash yet: check the R1 and build the flash first");
    if (r1.mode !== "fastboot") fail("no R1 in its bootloader");
    const steps = plan.writes.length + 1;
    const line = (l: string) => emit("flash-line", l);
    try {
      line("Checking the bootloader");
      await sleep(500);
      for (const [i, w] of plan.writes.entries()) {
        emit("flash-progress", { ...w, step: i + 1, steps });
        if (w.part === "super") {
          line("Writing super: r1os's system, the R1's vendor kept");
          for (let n = 1; n <= 15; n++) {
            line(`Sending sparse 'super' ${n}/15 (262140 KB)                OKAY [  8.12s]`);
            await sleep(450);
            line(`Writing 'super'                                    OKAY [  1.96s]`);
          }
        } else if (w.what === "erase") {
          line(`Erasing ${w.part}`);
          await sleep(900);
          line(`Erasing '${w.part}'                                 OKAY [  1.20s]`);
        } else {
          line(`Flashing ${w.part}`);
          line(`Sending '${w.part}' (${Math.max(4, Math.round(w.bytes / 1024))} KB)                 OKAY [  0.01s]`);
          await sleep(w.part === "userdata" ? 900 : 250);
          line(`Writing '${w.part}'                                 OKAY [  0.01s]`);
        }
      }
      emit("flash-progress", { what: "boot", part: null, bytes: 0, step: steps, steps });
      line("Booting r1os: the first boot takes a few minutes");
      line("Rebooting                                          OKAY [  0.00s]");
      later("adb", 8000, () => (r1.r1os = "0.2.18"));
    } finally {
      forget();
    }
  },
};
