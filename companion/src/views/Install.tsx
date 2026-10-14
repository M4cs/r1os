// Installing r1os on an R1 over USB (ADR-0065, ADR-0067, docs/specs/companion.md §11; T-171): the wizard, reachable
// with no linked R1. Five steps, one at a time: drivers, a backup, the unlock, the flash and the first setup. Each says
// what's happening, what to do with your hands and what to do if it goes wrong, and finds its state out (the R1 on
// USB, what's installed, fetched and read) rather than assuming it. Nothing is erased by a single click: the flash
// shows its plan and needs ERASE typed, and only after a backup or a typed "I understand". The first setup erases
// nothing: it shows its plan, read from the R1, and starts on a click. A log of it all, without serials or IMEIs, can
// be saved.

import { Icon } from "@r1os/icons";
import { AnimatePresence, LayoutGroup, motion } from "motion/react";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  type BackupMade,
  type BackupStatus,
  type Checked,
  type Drivers,
  type FlashPlan,
  type FlashWrite,
  type Merge,
  type ReadStatus,
  type SetupInput,
  type SetupInputs,
  type SetupPlan,
  type SetupR1,
  type SetupStatus,
  type SetupStep as SetupStepT,
  type StockStatus,
  type UsbR1,
  type UsbScan,
  type UsbTools,
  inTauri,
  install,
  openUrl,
  pickCheckout,
  pickFolder,
  pickSystemImage,
  reveal,
  saveDownload,
} from "../link";
import { page, soft, spring } from "../motion";
import { PlugInOff, PressSideButton } from "./Hands";
import { Twice, ago, size } from "./parts";
import { Item, Section } from "./Settings";

type StepId = "drivers" | "backup" | "unlock" | "flash" | "setup";
const STEPS: { id: StepId; title: string }[] = [
  { id: "drivers", title: "drivers" },
  { id: "backup", title: "backup" },
  { id: "unlock", title: "unlock" },
  { id: "flash", title: "flash" },
  { id: "setup", title: "first setup" },
];

/** Rough speeds for an estimate before anything's been timed: fastboot over USB 2 (a guess until it's timed on an
 *  R1), and the preloader's serial port (about 1 MB a second, T-169). Once something's going, its own pace counts. */
const USB_RATE = 20 << 20;
const PORT_RATE = 1 << 20;

const RABBIT_UNLOCK = "https://www.rabbit.tech/support/article/unlock-bootloader-rabbit-r1";
const RABBIT_RESTORE = "https://rabbit-hmi-oss.github.io/flashing/";

const MODE: Record<UsbR1["mode"], string> = {
  preloader: "starting up (its preloader)",
  adb: "booted",
  unauthorized: "booted, asking to allow usb debugging",
  fastboot: "in its bootloader",
  fastbootd: "in fastbootd",
  no_driver: "plugged in, with no driver for it yet",
};

const MERGE: Record<Merge, string> = {
  none: "no rabbit update pending",
  unknown: "a rabbit update in an unknown state",
  snapshotted: "a rabbit update waiting for its first start",
  merging: "a rabbit update finishing",
  cancelled: "a rabbit update cancelled",
};

/** What an R1 on USB is doing, in words. */
function r1Words(r: UsbR1): string {
  if (r.mode === "adb") return r.r1os ? `booted into r1os ${r.r1os}` : "booted";
  if (r.mode === "fastboot") return `in its bootloader${r.unlocked === true ? " · unlocked" : r.unlocked === false ? " · locked" : ""}${r.slot ? ` · slot ${r.slot}` : ""}`;
  return MODE[r.mode];
}

/** "about 3 min", "about 40 s". */
function about(s: number): string {
  if (!Number.isFinite(s) || s <= 0) return "";
  if (s < 10) return "a few seconds";
  if (s < 90) return `about ${Math.max(10, Math.round(s / 5) * 5)} s`;
  if (s < 3600) return `about ${Math.round(s / 60)} min`;
  return `about ${(s / 3600).toFixed(1)} h`;
}

function words(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

// ---- what's kept while the companion runs --------------------------------------------------------------------------

/** What the wizard keeps while the companion runs, so leaving the page and coming back finds it as it was: the log, the
 *  step, the R1 last seen, a backup made or skipped (for which R1), the system image picked, a flash done, an R1 set
 *  up. Never on disk. */
const kept = {
  lines: [] as string[],
  said: "",
  at: "drivers" as StepId,
  known: null as string | null,
  made: null as BackupMade | null,
  skipped: null as string | null,
  system: null as string | null,
  flashed: false,
  setUp: null as SetupR1 | null,
};
type Kept = typeof kept;

/** One of [kept], as state. */
function useKept<K extends keyof Kept>(key: K): [Kept[K], (v: Kept[K]) => void] {
  const [v, setV] = useState(kept[key]);
  const set = useCallback(
    (x: Kept[K]) => {
      kept[key] = x;
      setV(x);
    },
    [key],
  );
  return [v, set];
}

// ---- the log -------------------------------------------------------------------------------------------------------

interface Log {
  lines: string[];
  add: (line: string) => void;
}

/** The core names an R1 by a hash of its serial and takes the serial out of fastboot's lines, so it never gets here.
 *  Anything that could be an IMEI (15 digits in a row) is taken out too, whatever said it. */
function scrub(s: string): string {
  return s.replace(/\b\d{15}\b/g, "<number taken out>");
}

const logWatchers = new Set<() => void>();
function addLine(line: string) {
  const t = new Date().toTimeString().slice(0, 8);
  kept.lines = [...kept.lines.slice(-1999), `${t}  ${scrub(line)}`];
  logWatchers.forEach((f) => f());
}
function watchLog(f: () => void) {
  logWatchers.add(f);
  return () => void logWatchers.delete(f);
}

function useLog(): Log {
  return { lines: useSyncExternalStore(watchLog, () => kept.lines), add: addLine };
}

// ---- what's plugged in ---------------------------------------------------------------------------------------------

/** What's on USB, looked at every few seconds while this page is open and the window shows. While the core is talking
 *  to the R1 it says it's busy, and the last answer stands. */
function useUsb(): { scan: UsbScan | null; again: () => void } {
  const [scan, setScan] = useState<UsbScan | null>(null);
  const [tick, setTick] = useState(0);
  useEffect(() => {
    let gone = false;
    let t: ReturnType<typeof setTimeout> | undefined;
    const look = async () => {
      if (document.visibilityState === "visible") {
        try {
          const s = await install.scan();
          if (!gone && !s.busy) setScan(s);
        } catch {
          // The next look.
        }
      }
      if (!gone) t = setTimeout(look, 2500);
    };
    void look();
    return () => {
      gone = true;
      clearTimeout(t);
    };
  }, [tick]);
  return { scan, again: useCallback(() => setTick((n) => n + 1), []) };
}

// ---- the wizard ----------------------------------------------------------------------------------------------------

export function Install() {
  const { scan, again } = useUsb();
  const log = useLog();
  const [at, setAt] = useKept("at");
  const r1s = scan?.r1s ?? [];
  const r1 = r1s.length === 1 ? r1s[0] : null;
  // The R1's identity, kept while it's off or starting up (its preloader has a serial of its own).
  const [known, setKnown] = useKept("known");
  useEffect(() => {
    if (r1 && r1.mode !== "preloader") setKnown(r1.id);
  }, [r1?.id, r1?.mode]); // eslint-disable-line react-hooks/exhaustive-deps

  // This computer's side, found out once and again after each change.
  const [tools, setTools] = useState<UsbTools | null>(null);
  const [drivers, setDrivers] = useState<Drivers | null>(null);
  const reloadPc = useCallback(() => {
    install.tools().then(setTools).catch(() => undefined);
    install.drivers().then(setDrivers).catch((e) => addLine(`drivers: ${words(e)}`));
  }, []);
  useEffect(reloadPc, [reloadPc]);

  // This R1's backup: one here that verifies, one made now, or skipped by typing (for this R1, or before one was seen).
  const [backup, setBackup] = useState<BackupStatus | null>(null);
  const [made, setMade] = useKept("made");
  const [skippedFor, setSkippedFor] = useKept("skipped");
  const reloadBackup = useCallback(() => {
    install.backupStatus(known).then(setBackup).catch(() => undefined);
  }, [known]);
  useEffect(reloadBackup, [reloadBackup]);
  const backedUp = !!backup?.latest || (!!made && made.identity === known);
  const skipped = skippedFor !== null && (skippedFor === "" || skippedFor === known);

  // The system image for the flash (picked here; r1os's releases come later), whether it was flashed just now, and the
  // R1 as the first setup left it.
  const [system, setSystem] = useKept("system");
  const [flashed, setFlashed] = useKept("flashed");
  const [setUp, setSetUp] = useKept("setUp");

  // The R1 as it changes, in the log.
  useEffect(() => {
    if (!scan) return;
    const now = r1 ? `usb: r1 ${r1.id} is ${r1Words(r1)}` : r1s.length > 1 ? "usb: more than one r1 is plugged in" : "usb: no r1 plugged in";
    if (now !== kept.said) {
      kept.said = now;
      addLine(now);
    }
  });

  const done: Record<StepId, boolean> = {
    drivers: !!tools?.version && !!drivers?.r1 && !!drivers?.mtk,
    backup: backedUp || skipped,
    unlock: (r1?.mode === "fastboot" && r1.unlocked === true) || (r1?.mode === "adb" && !!r1.r1os),
    flash: flashed || (r1?.mode === "adb" && !!r1.r1os),
    setup: !!setUp && setUp.r1 === known,
  };
  const index = STEPS.findIndex((s) => s.id === at);
  // Another step: its top in view.
  const strip = useRef<HTMLElement>(null);
  const go = (id: StepId) => {
    setAt(id);
    strip.current?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  };

  return (
    <motion.section className="install" {...page}>
      <header className="install-head">
        <h1>install r1os</h1>
        <p className="lede muted">over usb, from this computer: drivers, a backup, the unlock, the flash, then the first setup. it works with a stock r1 or one running r1os, linked or not.</p>
        <div className="chips">
          <span className="chip">
            <span className={`dot ${r1 ? "awake" : ""}`} />
            {r1 ? <span>r1 <span className="mono">{r1.id}</span> · {r1Words(r1)}</span> : r1s.length > 1 ? "more than one r1 is plugged in: leave just one" : scan ? "no r1 plugged in" : "looking…"}
          </span>
          {scan && !scan.tools && <span className="chip muted">fetch the usb tools to see more</span>}
          {!inTauri && <span className="chip">demo: a pretend r1</span>}
        </div>
      </header>

      <LayoutGroup id="install-steps">
        <nav className="install-steps" ref={strip}>
          {STEPS.map((s, i) => (
            <button key={s.id} className={`install-step ${at === s.id ? "on" : ""} ${done[s.id] ? "done" : ""}`} onClick={() => go(s.id)}>
              {at === s.id && <motion.span className="install-step-fill" layoutId="install-step-fill" transition={spring} />}
              <span className="install-step-n">{done[s.id] ? <Icon name="check" weight="bold" size={13} /> : i + 1}</span>
              <span>{s.title}</span>
            </button>
          ))}
        </nav>
      </LayoutGroup>

      <AnimatePresence mode="wait">
        <motion.div key={at} {...page}>
          {at === "drivers" && <DriversStep tools={tools} drivers={drivers} reload={reloadPc} log={log} />}
          {at === "backup" && (
            <BackupStep
              r1={r1}
              known={known}
              status={backup}
              made={made}
              skipped={skipped}
              log={log}
              reload={reloadBackup}
              onMade={(b) => {
                setMade(b);
                // A backup names its R1 by the serial in its proinfo: the R1's own identity, which its preloader's
                // serial isn't.
                if (!known) setKnown(b.identity);
                reloadBackup();
              }}
              onSkip={() => setSkippedFor(known ?? "")}
            />
          )}
          {at === "unlock" && <UnlockStep r1={r1} log={log} again={again} />}
          {at === "flash" && (
            <FlashStep
              r1={r1}
              known={known}
              backupOk={backedUp || skipped}
              system={system}
              onSystem={setSystem}
              log={log}
              again={again}
              onFlashed={() => {
                setFlashed(true);
                // Its /data is erased: a setup before the flash is gone with it.
                setSetUp(null);
              }}
              onBackup={() => go("backup")}
              onSetup={() => go("setup")}
            />
          )}
          {at === "setup" && <SetupStep r1={r1} known={known} flashed={flashed} setUp={setUp} log={log} onSetUp={setSetUp} />}
        </motion.div>
      </AnimatePresence>

      <div className="install-nav">
        {index > 0 ? (
          <button className="btn small" onClick={() => go(STEPS[index - 1].id)}>
            <Icon name="arrow-left" weight="bold" size={14} /> {STEPS[index - 1].title}
          </button>
        ) : (
          <span />
        )}
        {index < STEPS.length - 1 && (
          <button className={`btn small ${done[at] ? "primary" : ""}`} onClick={() => go(STEPS[index + 1].id)}>
            {done[at] ? "next" : "skip ahead"}: {STEPS[index + 1].title} <Icon name="arrow-right" weight="bold" size={14} />
          </button>
        )}
      </div>

      <LogCard log={log} />
    </motion.section>
  );
}

// ---- the steps' shared parts ---------------------------------------------------------------------------------------

/** A step: what's happening, what to do with your hands (with a picture for what's done by hand), its own rows, and
 *  what to do if it goes wrong. */
function StepCard({ what, hands, picture, wrong, children }: { what: React.ReactNode; hands: React.ReactNode; picture?: React.ReactNode; wrong: React.ReactNode[]; children: React.ReactNode }) {
  return (
    <motion.div className="list-card install-card" initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0, transition: soft }}>
      <Section title="what's happening" />
      <div className="install-what">{what}</div>
      <Section title="with your hands" />
      <div className={`hands ${picture ? "" : "words"}`}>
        {picture}
        <div className="hands-words">{hands}</div>
      </div>
      {children}
      <Section title="if it goes wrong" />
      <ul className="wrong">
        {wrong.map((w, i) => (
          <li key={i}>{w}</li>
        ))}
      </ul>
    </motion.div>
  );
}

/** A state found out: yes, no, or not known yet. */
function Found({ ok, yes, no, unknown = "…" }: { ok: boolean | null | undefined; yes: string; no: string; unknown?: string }) {
  return (
    <span className={`state ${ok ? "ok" : ok === false ? "no" : ""}`}>
      {ok ? <Icon name="check-circle" weight="fill" size={15} /> : ok === false ? <Icon name="circle-dashed" weight="bold" size={15} /> : null}
      {ok ? yes : ok === false ? no : unknown}
    </span>
  );
}

/** Seconds since [since], ticking while it's shown. */
function useSince(since: number): number {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  return Math.max(0, (now - since) / 1000);
}

/** Progress: a track, what's being done, how far (in bytes, or as [count] says it), and how long it's likely to take
 *  still (its own pace once it has one, else [rate]). With no total, how long it's been going. */
function Bar({ what, done = 0, total = 0, since, rate, count }: { what: string; done?: number; total?: number; since: number; rate?: number; count?: string }) {
  const elapsed = useSince(since);
  const frac = total > 0 ? Math.min(1, done / total) : 0;
  const pace = elapsed > 3 && done > 0 ? done / elapsed : rate;
  const left = total > 0 && pace ? (total - done) / pace : NaN;
  return (
    <div className="install-bar">
      <div className="track">
        {total > 0 ? (
          <motion.div className="track-fill" animate={{ width: `${Math.max(2, frac * 100)}%` }} transition={soft} />
        ) : (
          <div className="track-fill install-wait" />
        )}
      </div>
      <div className="install-bar-line">
        <span>{what}</span>
        <span>
          {total > 0 ? (count ?? `${done ? size(done) : "0 mb"} of ${size(total)}`) : `for ${elapsed < 60 ? `${Math.round(elapsed)} s` : `${Math.floor(elapsed / 60)} min ${Math.round(elapsed % 60)} s`}`}
          {total > 0 && left > 0 ? ` · ${about(left)} left` : ""}
        </span>
      </div>
    </div>
  );
}

// ---- 1. drivers ----------------------------------------------------------------------------------------------------

function DriversStep({ tools, drivers, reload, log }: { tools: UsbTools | null; drivers: Drivers | null; reload: () => void; log: Log }) {
  const [fetching, setFetching] = useState<{ got: number; total: number; since: number } | null>(null);
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState<{ what: string; since: number } | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const fetchTools = async () => {
    setErr(null);
    const since = Date.now();
    setFetching({ got: 0, total: tools?.size ?? 0, since });
    const off = await install.on<{ got: number; total: number }>("usb-tools-progress", (p) => setFetching({ ...p, since }));
    log.add(`tools: fetching google's platform-tools ${tools?.pinned ?? ""}`);
    try {
      const r = await install.fetchTools();
      log.add(`tools: platform-tools ${r.version} fetched and checked`);
    } catch (e) {
      setErr(words(e));
      log.add(`tools: ${words(e)}`);
    } finally {
      off();
      setFetching(null);
      reload();
    }
  };

  const change = async (what: "install" | "undo") => {
    setAsking(false);
    setErr(null);
    setBusy({ what: what === "install" ? "installing: answer windows' prompt, then it carries on by itself" : "taking it out: answer windows' prompt", since: Date.now() });
    log.add(what === "install" ? "drivers: installing (windows asks for an administrator's ok)" : "drivers: taking the r1's driver out");
    try {
      const r = await (what === "install" ? install.installDrivers() : install.undoDrivers());
      r.log.split(/\r?\n/).filter((l) => l.trim()).forEach((l) => log.add(`drivers: ${l.trim()}`));
    } catch (e) {
      setErr(words(e));
      log.add(`drivers: ${words(e)}`);
    } finally {
      setBusy(null);
      reload();
    }
  };

  const missing = drivers ? !drivers.r1 || !drivers.mtk : false;
  return (
    <StepCard
      what={
        <p>
          windows needs two drivers to reach the r1: one for its bootloader, and rabbit's own for the second it's starting up (the backup of a stock r1 and the way into the bootloader from power-off use it). the usb tools are google's adb and fastboot, which this computer drives itself. nothing here touches the r1.
        </p>
      }
      hands={<p>nothing yet: the r1 can stay unplugged. installing the drivers asks for an administrator's ok once, in windows' own prompt.</p>}
      wrong={[
        "windows' prompt was closed or said no: nothing changed. install again when you're ready.",
        "device manager still shows the r1 as “android” with a warning after installing: unplug it and plug it in again.",
        "to take the r1's driver back out (and the certificate made for it on this computer): undo, above. rabbit's mediatek driver stays: it's rabbit's own, and windows lists it with your apps.",
        "fetching fails: check this computer is online. what's fetched is checked against its pinned sha-256, and anything else is thrown away.",
      ]}
    >
      <Section title="on this computer" />
      <Item label="google's usb tools" sub={`adb and fastboot, platform-tools ${tools?.pinned ?? ""}: fetched once and checked. fetching them accepts google's terms.`}>
        {tools?.version ? (
          <Found ok yes={`platform-tools ${tools.version}`} no="" />
        ) : (
          <div className="row-buttons">
            <button className="link" onClick={() => tools && void openUrl(tools.terms)}>
              google's terms <Icon name="arrow-up-right" size={13} />
            </button>
            <motion.button className="btn small" whileTap={{ scale: 0.95 }} disabled={!!fetching || !tools?.pinned} onClick={() => void fetchTools()}>
              <Icon name="download-simple" weight="bold" size={14} /> {fetching ? "fetching…" : `fetch · ${tools?.size ? size(tools.size) : "…"}`}
            </motion.button>
          </div>
        )}
      </Item>
      {fetching && (
        <div className="install-row">
          <Bar what="fetching platform-tools from google" done={fetching.got} total={fetching.total} since={fetching.since} />
        </div>
      )}
      <Item label="the r1's driver" sub="so windows can reach the r1 in its bootloader. made on this computer for the r1 alone, and signed by a certificate made for it">
        <Found ok={drivers?.r1} yes="installed" no="not yet" />
      </Item>
      <Item label="rabbit's mediatek driver" sub="the r1's port for the second it's starting up: rabbit's signed driver, fetched from its release and checked">
        <Found ok={drivers?.mtk} yes="installed" no="not yet" />
      </Item>
      <AnimatePresence initial={false}>
        {asking && (
          <motion.div className="install-plan" initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: "auto", transition: soft }} exit={{ opacity: 0, height: 0 }}>
            <div className="install-plan-title">
              <Icon name="shield-check" weight="bold" size={16} /> what installing does, on this computer
            </div>
            <ul>
              {!drivers?.r1 && <li>makes a driver for the r1's bootloader (its usb ids only), signs it with a certificate made here, trusts that certificate and deletes its private key, then installs it</li>}
              {!drivers?.mtk && <li>fetches rabbit's mediatek preloader driver (9 mb, checked) and runs its installer</li>}
              <li>windows asks for an administrator's ok once. nothing is written to the r1.</li>
            </ul>
            <div className="row-buttons">
              <motion.button className="btn small primary" whileTap={{ scale: 0.95 }} onClick={() => void change("install")}>
                install
              </motion.button>
              <button className="btn small" onClick={() => setAsking(false)}>
                not now
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>
      <Item label="the drivers" sub={missing ? "about a minute, most of it windows' own" : drivers ? "both installed" : "…"}>
        <div className="row-buttons">
          {missing && !asking && (
            <motion.button className="btn small" whileTap={{ scale: 0.95 }} disabled={!!busy} onClick={() => setAsking(true)}>
              <Icon name="plugs" weight="bold" size={14} /> install…
            </motion.button>
          )}
          {drivers?.r1 && <Twice label="undo" armed="click again: take it out" disabled={!!busy} onConfirm={() => void change("undo")} />}
        </div>
      </Item>
      {busy && (
        <div className="install-row">
          <Bar what={busy.what} since={busy.since} />
        </div>
      )}
      {err && <p className="err install-err">{err}</p>}
    </StepCard>
  );
}

// ---- 2. backup -----------------------------------------------------------------------------------------------------

function BackupStep({ r1, known, status, made, skipped, log, reload, onMade, onSkip }: {
  r1: UsbR1 | null;
  known: string | null;
  status: BackupStatus | null;
  made: BackupMade | null;
  skipped: boolean;
  log: Log;
  reload: () => void;
  onMade: (b: BackupMade) => void;
  onSkip: () => void;
}) {
  const [tools, setTools] = useState<{ what: string; got: number; total: number; since: number } | null>(null);
  const [run, setRun] = useState<{ since: number; part?: string; done: number; total: number; line?: string } | null>(null);
  const [typed, setTyped] = useState("");
  const [err, setErr] = useState<string | null>(null);
  // Over adb at USB speed (an R1 booted into r1os has root), or through the preloader at power-on.
  const fast = r1?.mode === "adb" && !!r1.r1os;
  const bootedWithoutRoot = r1?.mode === "adb" && !r1.r1os;
  const part = useRef("");

  const fetchTools = async () => {
    setErr(null);
    const since = Date.now();
    setTools({ what: "uv", got: 0, total: 0, since });
    const off = await install.on<{ what: string; got: number; total: number }>("backup-tools-progress", (p) => setTools({ ...p, since }));
    log.add("backup: fetching uv and mtkclient");
    try {
      await install.fetchBackupTools();
      log.add("backup: uv, mtkclient and its python ready");
    } catch (e) {
      setErr(words(e));
      log.add(`backup: ${words(e)}`);
    } finally {
      off();
      setTools(null);
      reload();
    }
  };

  const go = async () => {
    setErr(null);
    part.current = "";
    setRun({ since: Date.now(), done: 0, total: 0 });
    const offs = await Promise.all([
      install.on<{ part: string; done: number; total: number }>("backup-progress", (p) => {
        if (p.part !== part.current) {
          part.current = p.part;
          log.add(`backup: reading ${p.part}`);
        }
        setRun((r) => r && { ...r, ...p });
      }),
      install.on<string>("backup-line", (l) => {
        log.add(`backup: ${l}`);
        setRun((r) => r && { ...r, line: l });
      }),
    ]);
    log.add(fast ? "backup: reading the r1's own partitions over usb" : "backup: waiting for the r1 at power-on");
    try {
      const b = await install.backup();
      log.add(`backup: done: ${b.parts.length} partitions, checked, in ${b.dir}`);
      onMade(b);
    } catch (e) {
      setErr(words(e));
      log.add(`backup: ${words(e)}`);
    } finally {
      offs.forEach((f) => f());
      setRun(null);
    }
  };

  const skip = async () => {
    if (await install.skipBackup(typed, known ?? null)) {
      log.add("backup: skipped, with “i understand” typed");
      onSkip();
    } else setErr(`type “${status?.skip_phrase ?? "I understand"}” to go on without a backup`);
  };

  const have = made && made.identity === known ? made.dir : status?.latest;
  const ready = fast || (!!status?.tools && !!status?.mtk_driver && !bootedWithoutRoot);
  return (
    <StepCard
      what={
        <p>
          before anything is written, the r1's own partitions are copied to this computer: its imei and radio calibration, its unlock state, and rabbit's boot chain on both slots. no image can give those back. r1os's flash never writes them, so a backup is what gets them back if something else ever does. it only reads, into documents\r1os backups, with sums to check it by.
        </p>
      }
      picture={fast ? undefined : <PlugInOff />}
      hands={
        fast ? (
          <p>keep the r1 plugged in and on. it reads at usb speed, a minute or two.</p>
        ) : (
          <ol>
            <li>turn the r1 off completely, and leave it unplugged.</li>
            <li>click back up below.</li>
            <li>plug the r1 into this computer while it waits. it's caught in the second it starts up, then read for several minutes (about 1 mb a second). keep it plugged in until it's done.</li>
          </ol>
        )
      }
      wrong={[
        "nothing happens when it's plugged in: check rabbit's mediatek driver (the drivers step), try another cable or a port on the computer itself, then stop and start again.",
        "the r1 starts up as usual instead: it wasn't caught. stop, turn it off, and try again.",
        "it stops part way: nothing it read is kept (it holds the r1's imei). run it again.",
        "an r1 booted without root (rabbit's own os) is read at power-on: turn it off first.",
      ]}
    >
      <Section title="this r1's backup" />
      <Item label="a backup here" sub={have ? `documents\\r1os backups\\${have.split(/[\\/]/).slice(-2).join("\\")}: its sums verify` : known ? "none for this r1 yet" : "plug the r1 in to find out whether this computer has its backup"}>
        <div className="row-buttons">
          <Found ok={known ? !!have : null} yes="checked" no="not yet" unknown="not known yet" />
          {have && inTauri && (
            <button className="btn small" onClick={() => void reveal(have)}>
              <Icon name="folder-open" weight="bold" size={14} /> show
            </button>
          )}
        </div>
      </Item>
      <Item label="how it's read" sub={fast ? "over usb from r1os, which has root" : bootedWithoutRoot ? "this r1 is booted without root: turn it off, and it's read at power-on" : "at power-on, through the r1's preloader, with mtkclient (gpl, fetched for this)"}>
        <span className="value">{fast ? "usb" : "power-on"}</span>
      </Item>
      {!fast && (
        <>
          <Item label="the backup's tools" sub="uv, mtkclient and a python for it, fetched once (about 55 mb) and checked">
            {status?.tools ? (
              <Found ok yes="ready" no="" />
            ) : (
              <motion.button className="btn small" whileTap={{ scale: 0.95 }} disabled={!!tools || !!run} onClick={() => void fetchTools()}>
                <Icon name="download-simple" weight="bold" size={14} /> {tools ? "fetching…" : "fetch"}
              </motion.button>
            )}
          </Item>
          {tools && (
            <div className="install-row">
              <Bar what={tools.what === "python" ? "getting python and mtkclient's packages ready" : `fetching ${tools.what}`} done={tools.got} total={tools.total} since={tools.since} />
            </div>
          )}
          <Item label="rabbit's mediatek driver" sub="the preloader's port (the drivers step)">
            <Found ok={status?.mtk_driver} yes="installed" no="not yet" />
          </Item>
        </>
      )}
      <Item label="back up" sub={run ? undefined : fast ? "reads only · a minute or two" : "reads only · several minutes once it's caught"} wide={!!run}>
        {run ? (
          <div className="install-running">
            {run.total > 0 ? (
              <Bar what={`reading ${run.part ?? "…"}`} done={run.done} total={run.total} since={run.since} rate={USB_RATE} />
            ) : (
              <Bar what={run.line ?? (fast ? "starting…" : "waiting for the r1: plug it in now")} since={run.since} rate={PORT_RATE} />
            )}
            <div className="row-buttons">
              <button className="btn small" onClick={() => void install.stopBackup()}>
                <Icon name="stop" weight="fill" size={14} /> stop
              </button>
            </div>
          </div>
        ) : (
          <motion.button className="btn small primary" whileTap={{ scale: 0.95 }} disabled={!ready} onClick={() => void go()}>
            <Icon name="floppy-disk" weight="bold" size={14} /> back up
          </motion.button>
        )}
      </Item>
      {!have && (
        <Item label="go on without one" sub={skipped ? "skipped: you typed it" : `type “${status?.skip_phrase ?? "I understand"}”: r1os's flash never writes these partitions, so it's insurance not taken, not a step left out`}>
          {skipped ? (
            <Found ok yes="skipped" no="" />
          ) : (
            <div className="row-buttons">
              <input className="field" value={typed} placeholder={status?.skip_phrase ?? "I understand"} spellCheck={false} onChange={(e) => setTyped(e.target.value)} onKeyDown={(e) => e.key === "Enter" && void skip()} />
              <button className="btn small" disabled={!typed.trim()} onClick={() => void skip()}>
                skip
              </button>
            </div>
          )}
        </Item>
      )}
      {err && <p className="err install-err">{err}</p>}
    </StepCard>
  );
}

// ---- getting into the bootloader (the unlock and the flash) -------------------------------------------------------

/** Into the R1's bootloader: from a booted R1, a restart; from power-off, its preloader asked as it starts. And out
 *  again without flashing. Boot modes only: nothing is written. */
function IntoBootloader({ r1, log, again }: { r1: UsbR1 | null; log: Log; again: () => void }) {
  const [waiting, setWaiting] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const restart = async () => {
    setErr(null);
    setBusy(true);
    log.add("bootloader: restarting the r1 into its bootloader");
    try {
      await install.toBootloader();
    } catch (e) {
      setErr(words(e));
      log.add(`bootloader: ${words(e)}`);
    } finally {
      setBusy(false);
      again();
    }
  };
  const fromOff = async () => {
    setErr(null);
    setWaiting(Date.now());
    log.add("bootloader: waiting for the r1 at power-on");
    try {
      await install.enterFastboot();
      log.add("bootloader: its preloader said ready and was asked for the bootloader");
    } catch (e) {
      if (words(e) !== "stopped") setErr(words(e));
      log.add(`bootloader: ${words(e)}`);
    } finally {
      setWaiting(null);
      again();
    }
  };
  const leave = async () => {
    setErr(null);
    setBusy(true);
    log.add("bootloader: leaving it; the r1 starts its own system");
    try {
      await install.leaveBootloader();
    } catch (e) {
      setErr(words(e));
      log.add(`bootloader: ${words(e)}`);
    } finally {
      setBusy(false);
      again();
    }
  };

  if (r1?.mode === "fastboot")
    return (
      <Item label="in the bootloader" sub={`${r1.unlocked ? "unlocked" : r1.unlocked === false ? "locked" : "…"}${r1.slot ? ` · slot ${r1.slot} active` : ""}. leaving it starts the r1's own system again`}>
        <div className="row-buttons">
          <Found ok yes="yes" no="" />
          <button className="btn small" disabled={busy} onClick={() => void leave()}>
            leave it
          </button>
        </div>
      </Item>
    );
  return (
    <>
      <Item
        label="into the bootloader"
        sub={r1?.mode === "adb" ? "a restart, about 10 s: nothing written" : r1?.mode === "fastbootd" ? "it's in fastbootd, which r1os's install never uses: restart it into the bootloader with fastboot reboot bootloader" : "from power-off: the r1 off, then plugged in while this waits"}
      >
        <div className="row-buttons">
          {r1?.mode === "adb" && (
            <motion.button className="btn small" whileTap={{ scale: 0.95 }} disabled={busy || waiting !== null} onClick={() => void restart()}>
              <Icon name="arrow-clockwise" weight="bold" size={14} /> restart into it
            </motion.button>
          )}
          {waiting === null ? (
            <motion.button className="btn small" whileTap={{ scale: 0.95 }} disabled={busy} onClick={() => void fromOff()}>
              <Icon name="power" weight="bold" size={14} /> from power-off…
            </motion.button>
          ) : (
            <button className="btn small" onClick={() => void install.stopEnterFastboot()}>
              stop waiting
            </button>
          )}
        </div>
      </Item>
      {waiting !== null && (
        <div className="hands install-row">
          <PlugInOff />
          <div className="hands-words">
            <ol>
              <li>turn the r1 off completely.</li>
              <li>plug it into this computer. its preloader is asked for the bootloader in the second it starts; this waits up to ten minutes.</li>
            </ol>
            <Bar what="waiting for the r1 to start up" since={waiting} />
          </div>
        </div>
      )}
      {err && <p className="err install-err">{err}</p>}
    </>
  );
}

// ---- 3. unlock -----------------------------------------------------------------------------------------------------

function UnlockStep({ r1, log, again }: { r1: UsbR1 | null; log: Log; again: () => void }) {
  const [ability, setAbility] = useState<{ unlocked: boolean | null; ability: number | null } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const inBootloader = r1?.mode === "fastboot";
  const unlocked = inBootloader ? r1.unlocked : r1?.mode === "adb" && r1.r1os ? true : null;

  const read = async () => {
    setErr(null);
    try {
      const a = await install.unlockAbility();
      setAbility(a);
      log.add(`unlock: unlocked ${a.unlocked === null ? "unknown" : a.unlocked ? "yes" : "no"}; get_unlock_ability says ${a.ability ?? "nothing"}`);
    } catch (e) {
      setErr(words(e));
      log.add(`unlock: ${words(e)}`);
    }
  };
  const copy = () => {
    void navigator.clipboard?.writeText("fastboot flashing unlock").then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <StepCard
      what={
        <p>
          the r1's bootloader only starts a system that isn't rabbit's once it's unlocked. rabbit allows it through its os3 portal, and the r1 itself asks before it unlocks. unlocking erases everything on the r1, and voids its warranty for good. this wizard reads whether it's unlocked; the unlock itself is yours to send.
        </p>
      }
      picture={<PressSideButton />}
      hands={
        <ol>
          <li>
            on rabbit's os3 portal, turn on developer mode: you type the warranty acknowledgment and the r1's imei there, never here.{" "}
            <button className="link inline" onClick={() => void openUrl(RABBIT_UNLOCK)}>
              rabbit's steps <Icon name="arrow-up-right" size={12} />
            </button>
          </li>
          <li>with the r1 on and online, choose device modification › unlock there.</li>
          <li>bring the r1 into its bootloader (below).</li>
          <li>
            in a terminal, in the usb tools' folder (<span className="mono">%LOCALAPPDATA%\r1os\companion\platform-tools</span>), run <span className="mono cmd">fastboot flashing unlock</span>{" "}
            <button className="link inline" onClick={copy}>
              <Icon name={copied ? "check" : "copy"} size={12} /> {copied ? "copied" : "copy"}
            </button>
          </li>
          <li>the r1 asks on its screen: press its side button to say yes.</li>
        </ol>
      }
      wrong={[
        "the portal says the imei doesn't match: type the one rabbit's own os shows, or the one on the r1's box.",
        "fastboot refuses the unlock: ask for it again in the os3 portal, then try again.",
        "never relock it: fastboot flashing lock on an r1 running r1os can leave it unable to start.",
      ]}
    >
      <Section title="this r1" />
      <Item label="the bootloader" sub={inBootloader ? "as the bootloader says it" : unlocked ? "r1os runs on it, so it's unlocked" : "found out in the bootloader"}>
        <Found ok={unlocked} yes="unlocked" no="locked" unknown="not known yet" />
      </Item>
      <IntoBootloader r1={r1} log={log} again={again} />
      <Item label="unlock ability" sub={ability ? "as the bootloader gives it: what the r1's number means isn't settled, so it's shown, not judged" : "fastboot flashing get_unlock_ability: read only"}>
        <div className="row-buttons">
          {ability && <span className="value mono">{ability.ability ?? "no answer"}</span>}
          <button className="btn small" disabled={!inBootloader} onClick={() => void read()}>
            read
          </button>
        </div>
      </Item>
      {err && <p className="err install-err">{err}</p>}
    </StepCard>
  );
}

// ---- 4. flash ------------------------------------------------------------------------------------------------------

interface Running {
  since: number;
  step: number;
  steps: number;
  write: FlashWrite | null;
  /** fastboot's chunks of a sparse image: the one being sent, of how many. */
  sparse: [number, number] | null;
}

function FlashStep({ r1, known, backupOk, system, onSystem, log, again, onFlashed, onBackup, onSetup }: {
  r1: UsbR1 | null;
  known: string | null;
  backupOk: boolean;
  system: string | null;
  onSystem: (path: string) => void;
  log: Log;
  again: () => void;
  onFlashed: () => void;
  onBackup: () => void;
  onSetup: () => void;
}) {
  const [stock, setStock] = useState<StockStatus | null>(null);
  const [stockGoing, setStockGoing] = useState<{ file: string; index: number; count: number } | null>(null);
  const [read, setRead] = useState<ReadStatus | null>(null);
  const [checked, setChecked] = useState<Checked | null>(null);
  const [plan, setPlan] = useState<FlashPlan | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [building, setBuilding] = useState<{ done: number; total: number; since: number } | null>(null);
  const [typed, setTyped] = useState("");
  const [running, setRunning] = useState<Running | null>(null);
  const [finished, setFinished] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  // What the core holds (the check and plan go when the R1 leaves its bootloader), and what's been read from this R1.
  const refresh = useCallback(() => {
    install
      .prepared()
      .then((p) => {
        setChecked(p.checked);
        setPlan(p.plan);
      })
      .catch(() => undefined);
    install.stock().then(setStock).catch(() => undefined);
  }, []);
  useEffect(refresh, [refresh, r1?.mode]);
  useEffect(() => {
    if (known) install.readStatus(known).then(setRead).catch(() => setRead(null));
  }, [known]);

  const fail = (where: string, e: unknown) => {
    setErr(words(e));
    log.add(`${where}: ${words(e)}`);
  };

  const fetchStock = async (from: string | null) => {
    setErr(null);
    setStockGoing({ file: "", index: 0, count: stock?.files.length ?? 4 });
    const off = await install.on<{ file: string; index: number; count: number }>("flash-stock-progress", (p) => {
      setStockGoing(p);
      log.add(`rabbit's files: reading ${p.file}`);
    });
    log.add(from ? "rabbit's files: copying them from the folder picked" : "rabbit's files: reading them out of rabbit's v0.8.293 release");
    try {
      await install.fetchStock(from);
      log.add("rabbit's files: here and checked");
    } catch (e) {
      fail("rabbit's files", e);
    } finally {
      off();
      setStockGoing(null);
      refresh();
    }
  };
  const fromFolder = async () => {
    const dir = await pickFolder("rabbit's v0.8.293 release, unpacked");
    if (dir) void fetchStock(dir);
  };
  const pick = async () => {
    const path = await pickSystemImage();
    if (path) {
      onSystem(path);
      setPlan(null);
      log.add(`system: ${path.split(/[\\/]/).pop()} picked`);
    }
  };

  const readR1 = async () => {
    setErr(null);
    setBusy("reading the r1…");
    log.add("read: super's first mib and para, over usb, as root");
    try {
      const r = await install.readR1();
      setRead(r);
      log.add(`read: r1 ${r.r1}: ${r.merge ? MERGE[r.merge] : "merge status unknown"}; vendor_a ${size(r.vendor_a ?? 0)}, vendor_b ${size(r.vendor_b ?? 0)}`);
    } catch (e) {
      fail("read", e);
    } finally {
      setBusy(null);
      again();
    }
  };

  const check = async () => {
    setErr(null);
    setBusy("checking…");
    log.add("check: reading the bootloader (nothing written)");
    try {
      const c = await install.check();
      log.add(`check: r1 ${c.r1}: slot ${c.slot}, vendor_${c.slot} ${size(c.vendor_bytes)}, the other slot's vendor ${c.other_vendor_bytes ? size(c.other_vendor_bytes) : "empty"}, ${MERGE[c.merge]}`);
    } catch (e) {
      fail("check", e);
    } finally {
      setBusy(null);
      refresh();
    }
  };

  const build = async () => {
    if (!system) return;
    setErr(null);
    setTyped("");
    const since = Date.now();
    setBuilding({ done: 0, total: 0, since });
    const off = await install.on<{ done: number; total: number }>("flash-prepare-progress", (p) => setBuilding({ ...p, since }));
    log.add(`build: super for this r1 with ${system.split(/[\\/]/).pop()}, on this computer`);
    try {
      const p = await install.prepare(system);
      log.add(`build: done and verified: ${size(p.bytes)} to write`);
      p.lines.forEach((l) => log.add(`plan: ${l}`));
    } catch (e) {
      fail("build", e);
    } finally {
      off();
      setBuilding(null);
      refresh();
    }
  };

  const flash = async () => {
    if (!plan) return;
    setErr(null);
    setRunning({ since: Date.now(), step: 0, steps: plan.writes.length + 1, write: null, sparse: null });
    const offs = await Promise.all([
      install.on<FlashWrite & { step: number; steps: number }>("flash-progress", (p) => setRunning((r) => r && { ...r, step: p.step, steps: p.steps, write: p, sparse: null })),
      install.on<string>("flash-line", (l) => {
        log.add(`flash: ${l}`);
        const m = /Sending sparse '[^']+' (\d+)\/(\d+)/.exec(l);
        if (m) setRunning((r) => r && { ...r, sparse: [Number(m[1]), Number(m[2])] });
      }),
    ]);
    log.add(`flash: ${plan.confirm} typed; flashing r1 ${plan.r1}, slot ${plan.slot}`);
    try {
      await install.run(typed);
      setFinished(true);
      onFlashed();
      log.add("flash: done; r1os is starting");
    } catch (e) {
      fail("flash", e);
    } finally {
      offs.forEach((f) => f());
      setRunning(null);
      setTyped("");
      refresh();
      again();
    }
  };

  const inBootloader = r1?.mode === "fastboot";
  const other = !!checked && !!known && checked.r1 !== known;
  const name = system?.split(/[\\/]/).pop();
  // How far the flash has got, in bytes: the writes before this one, and this one's chunks sent.
  const sent = running && plan
    ? plan.writes.slice(0, Math.max(0, running.step - 1)).reduce((n, w) => n + w.bytes, 0) +
      (running.write && running.sparse ? (running.write.bytes * (running.sparse[0] - 1)) / running.sparse[1] : 0)
    : 0;
  const doing = running?.write
    ? running.write.what === "boot"
      ? "starting r1os"
      : `${running.write.what === "erase" ? "erasing" : "writing"} ${running.write.part}${running.sparse ? ` (part ${running.sparse[0]} of ${running.sparse[1]})` : ""}`
    : "checking the bootloader again";

  return (
    <StepCard
      what={
        <>
          <p>
            r1os goes on from the bootloader, never fastbootd: verification off in vbmeta on both slots, then super rewritten in place (r1os's system on both slots, this r1's own vendor kept exactly where it is), then everything on the r1 erased. the kernel, the vendor, the bootloader and the r1's own partitions are never written. it's all checked first, built on this computer, and shown before anything is sent.
          </p>
          <p className="muted small">this flash is new: it's been checked on a computer against a real r1's layout, and not yet run end to end on an r1.</p>
        </>
      }
      hands={
        <ol>
          <li>plug the r1 into a usb port on the computer itself, not a hub, and charge it first: a low battery stops the check.</li>
          <li>don't unplug it, or close this window, while it flashes.</li>
          <li>afterwards it starts r1os by itself: the first start takes a few minutes.</li>
        </ol>
      }
      wrong={[
        "the check stops: it says why (locked, in fastbootd, a rabbit update pending, a low battery, an empty vendor). nothing has been written.",
        "fastboot refuses super: nothing of yours is gone yet, since super is written before the erase. save the log and ask for help.",
        <>
          stuck, or it won't start afterwards: rabbit's own restore puts its os back (it relocks the bootloader).{" "}
          <button className="link inline" onClick={() => void openUrl(RABBIT_RESTORE)}>
            rabbit's restore <Icon name="arrow-up-right" size={12} />
          </button>
        </>,
        "never take rabbit's update on an r1 running r1os.",
      ]}
    >
      <Section title="what it takes" />
      <Item label="rabbit's files" sub={stock ? `its vbmeta images and empty userdata: ${size(stock.bytes)} read out of its 922 mb v0.8.293 release, each checked` : "…"}>
        {stock?.ready ? (
          <Found ok yes="here, checked" no="" />
        ) : (
          <div className="row-buttons">
            <button className="link" disabled={!!stockGoing} onClick={() => void fromFolder()}>
              from a folder…
            </button>
            <motion.button className="btn small" whileTap={{ scale: 0.95 }} disabled={!!stockGoing} onClick={() => void fetchStock(null)}>
              <Icon name="download-simple" weight="bold" size={14} /> {stockGoing ? `${stockGoing.index + 1} of ${stockGoing.count}…` : "fetch"}
            </motion.button>
          </div>
        )}
      </Item>
      <Item label="r1os's system image" sub={name ? `${name}: a system image picked on this computer; every check still applies` : "r1os's releases come later: for now, a system image you built or were given"}>
        <button className="btn small" disabled={!!building || !!running} onClick={() => void pick()}>
          <Icon name="file-arrow-down" weight="bold" size={14} /> {name ? "another…" : "choose…"}
        </button>
      </Item>
      <Item
        label="read from the r1"
        sub={
          read?.read
            ? `${read.at ? ago(read.at) : ""} · ${read.merge ? MERGE[read.merge] : "merge status unknown"} · vendor_a ${read.vendor_a ? size(read.vendor_a) : "empty"}, vendor_b ${read.vendor_b ? size(read.vendor_b) : "empty"}`
            : "super's layout and whether a rabbit update is pending: read from the r1 booted with root (r1os, or magisk), reading only. a stock r1's will come with its backup: not yet"
        }
      >
        <div className="row-buttons">
          {read?.read && <Found ok yes="read" no="" />}
          <button className="btn small" disabled={r1?.mode !== "adb" || !!busy} onClick={() => void readR1()}>
            {read?.read ? "read again" : "read it"}
          </button>
        </div>
      </Item>

      <Section title="in the bootloader" />
      <IntoBootloader r1={r1} log={log} again={again} />
      <Item
        label="checked"
        sub={
          checked
            ? `slot ${checked.slot} · vendor ${size(checked.vendor_bytes)} · ${checked.other_vendor_bytes ? "the other slot gets r1os too" : "the other slot's vendor is empty: no fallback slot"} · ${MERGE[checked.merge]}`
            : "unlocked, not fastbootd, the battery, no rabbit update pending, the active slot's vendor whole: read only"
        }
      >
        <div className="row-buttons">
          {checked && <Found ok yes="passed" no="" />}
          <button className="btn small" disabled={!inBootloader || !read?.read || !!busy || !!running} onClick={() => void check()}>
            {busy === "checking…" ? "checking…" : checked ? "check again" : "check"}
          </button>
        </div>
      </Item>
      {other && <p className="err install-err">the r1 checked isn't the one plugged in: check this one.</p>}
      <Item
        label="built on this computer"
        sub={plan ? `${size(plan.bytes)} to write, verified as it was built` : "the vbmeta copies, and super with r1os on both slots and this r1's vendor where it is: nothing goes to the r1"}
        wide={!!building}
      >
        {building ? (
          <Bar what="building super: r1os's system on both slots" done={building.done} total={building.total} since={building.since} />
        ) : (
          <div className="row-buttons">
            {plan && <Found ok yes="ready" no="" />}
            <button className="btn small" disabled={!checked || !system || !stock?.ready || !!running} onClick={() => void build()}>
              {plan ? "build again" : "build"}
            </button>
          </div>
        )}
      </Item>

      {plan && !running && !finished && (
        <>
          <Section title="the plan" />
          <div className="install-plan danger">
            <div className="install-plan-title">
              <Icon name="warning" weight="fill" size={16} /> this erases everything on the r1
            </div>
            <ul className="keep-case">
              {plan.lines.map((l) => (
                <li key={l}>{l}</li>
              ))}
            </ul>
            <div className="install-writes">
              {plan.writes.map((w, i) => (
                <div key={i} className="install-write">
                  <span className="mono">{w.what}</span>
                  <span className="mono">{w.part}</span>
                  <span className="muted">{w.bytes ? size(w.bytes) : ""}</span>
                </div>
              ))}
            </div>
            <p className="muted small">
              {size(plan.bytes)} to write: {about(plan.bytes / USB_RATE + 20)} roughly, then r1os's first start takes a few minutes. keep the r1 plugged in until it's done.
            </p>
            {backupOk ? (
              <div className="row-buttons">
                <input
                  className="field install-confirm"
                  value={typed}
                  placeholder={`type ${plan.confirm} to flash`}
                  spellCheck={false}
                  onChange={(e) => setTyped(e.target.value)}
                />
                <motion.button
                  className="btn small danger"
                  whileTap={{ scale: 0.95 }}
                  disabled={typed.trim() !== plan.confirm || !inBootloader || !!other}
                  onClick={() => void flash()}
                >
                  <Icon name="lightning" weight="fill" size={14} /> flash
                </motion.button>
              </div>
            ) : (
              <div className="row-buttons">
                <span className="small">back this r1 up first, or skip the backup by typing on its step.</span>
                <button className="btn small" onClick={onBackup}>
                  the backup <Icon name="arrow-right" weight="bold" size={14} />
                </button>
              </div>
            )}
          </div>
        </>
      )}
      {running && plan && (
        <>
          <Section title="flashing" />
          <div className="install-row">
            <Bar what={`${doing} · step ${Math.max(1, running.step)} of ${running.steps}`} done={sent} total={plan.bytes} since={running.since} rate={USB_RATE} />
            <p className="muted small">don't unplug the r1 or close this window.</p>
          </div>
        </>
      )}
      {finished && (
        <div className="install-row">
          <p className="ok-note">flashed. r1os is starting: its first start takes a few minutes. never take rabbit's update on it.</p>
          <div className="row-buttons">
            <span className="small">keep it plugged in: the first setup puts r1os's core on it once it's started.</span>
            <button className="btn small primary" onClick={onSetup}>
              the first setup <Icon name="arrow-right" weight="bold" size={14} />
            </button>
          </div>
        </div>
      )}
      {err && <p className="err install-err">{err}</p>}
    </StepCard>
  );
}

// ---- 5. first setup ------------------------------------------------------------------------------------------------

/** A checkout's inputs for the setup, in words. */
const INPUT: Record<SetupInput["key"], string> = {
  r1d: "r1d, built for the r1",
  shell: "the shell",
  speech_libs: "speech-to-text's libraries",
  speech_model: "the speech-to-text model",
  system1: "system one's model",
  translator: "the translator's models",
  alpine: "alpine, for the agent box",
  codex: "codex",
  codex_host: "codex's code-mode host",
  busybox: "a static busybox",
};

/** Rough seconds each step takes on the R1 besides what it sends over USB: /data grown, r1d back, the agent box's
 *  downloads and build. Guesses until a setup's been timed on an R1. */
const ON_R1: Record<SetupStepT["what"], number> = {
  grow: 15,
  speech: 3,
  system1: 2,
  translator: 20,
  r1d: 2,
  shell: 2,
  restart: 15,
  agent_exec: 2,
  busybox: 2,
  agent_box: 240,
};

/** How long a step is likely to take, in seconds. */
function estimate(s: SetupStepT): number {
  return s.bytes / USB_RATE + ON_R1[s.what];
}

/** "115 gb", "2.8 gb": decimal, as the setup says a partition's size. */
function gb(bytes: number): string {
  const g = bytes / 1e9;
  return `${g >= 10 ? g.toFixed(0) : g.toFixed(1)} gb`;
}

/** What the setup read from an R1, in a line. */
function statusWords(s: SetupStatus): string {
  const box = s.agent_box;
  return [
    s.image ? `r1os ${s.image}` : s.restarter === "Magisk" ? "r1os-base, under magisk" : "not r1os's image",
    s.data.grown ? `/data ${gb(s.data.fs_bytes)}` : `/data ${gb(s.data.fs_bytes)} of ${gb(s.data.partition_bytes)}`,
    s.r1d_running ? `r1d running${s.r1d?.version ? ` (${s.r1d.version})` : ""}` : s.r1d ? "r1d there, not running" : "no r1d yet",
    box.rootfs && box.claude ? `the agent box: alpine ${box.alpine ?? "?"}, claude code ${box.claude}` : "no agent box yet",
    s.online ? "online" : s.online === false ? "offline" : "online or not: not known",
  ].join(" · ");
}

/** Whether the agent box is built, as the setup read it. */
function boxBuilt(s: SetupStatus): boolean {
  return s.agent_box.rootfs && s.agent_box.packages && !!s.agent_box.claude;
}

interface SetupRunning {
  since: number;
  /** The step going (from 1, 0 before the first), of how many, its words and when it started. */
  index: number;
  of: number;
  words: string;
  stepSince: number;
  /** The last lines the R1 printed in this step. */
  lines: string[];
}

/** The setup's progress, each step's share its estimate: the steps done, and the one going as its time passes (up to
 *  its estimate, so a slow one waits at the end of its share rather than running past it). */
function SetupBar({ steps, running }: { steps: SetupStepT[]; running: SetupRunning }) {
  const inStep = useSince(running.stepSince);
  const est = steps.map(estimate);
  const i = Math.max(1, running.index) - 1;
  const done = running.index ? est.slice(0, i).reduce((n, s) => n + s, 0) + Math.min((est[i] ?? 0) * 0.95, inStep) : 0;
  const total = est.reduce((n, s) => n + s, 0);
  return (
    <Bar
      what={running.index ? running.words : "reading the r1 again: is the plan still the same?"}
      done={done}
      total={total}
      since={running.since}
      rate={1}
      count={`step ${Math.max(1, running.index)} of ${running.of}`}
    />
  );
}

function SetupStep({ r1, known, flashed, setUp, log, onSetUp }: {
  r1: UsbR1 | null;
  known: string | null;
  flashed: boolean;
  setUp: SetupR1 | null;
  log: Log;
  onSetUp: (done: SetupR1) => void;
}) {
  const [inputs, setInputs] = useState<SetupInputs | null>(null);
  const [checking, setChecking] = useState(false);
  const [plan, setPlan] = useState<SetupPlan | null>(null);
  // What was last read from an R1: by a plan, a read alone, or the run.
  const [seen, setSeen] = useState<SetupR1 | null>(setUp);
  const [busy, setBusy] = useState(false);
  const [running, setRunning] = useState<SetupRunning | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [opened] = useState(Date.now());
  // The R1 a plan was last asked for (null: none booted), so a plan that fails isn't asked for again by itself.
  const tried = useRef<string | null | undefined>(undefined);
  const booted = r1?.mode === "adb" ? r1 : null;

  const fail = (e: unknown) => {
    setErr(words(e));
    log.add(`setup: ${words(e)}`);
  };

  /** The checkout's files checked: [from] one just picked, else the one picked before. [say]: in the log. */
  const checkInputs = async (from: string | null, say: boolean) => {
    setErr(null);
    setChecking(true);
    if (from) log.add(`setup: r1os's files from ${from}`);
    try {
      const i = await install.setupInputs(from);
      setInputs(i);
      if (say && i.from) log.add(i.ready ? "setup: every file it takes is here, read and checked" : `setup: ${i.error}`);
    } catch (e) {
      fail(e);
    } finally {
      setChecking(false);
    }
  };

  // The plan the core holds, then the checkout picked before.
  useEffect(() => {
    install
      .setupPlanned()
      .then((p) => {
        setPlan(p);
        if (p) tried.current = p.r1;
      })
      .catch(() => undefined)
      .finally(() => void checkInputs(null, false));
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const pick = async () => {
    const dir = await pickCheckout();
    if (dir) {
      setPlan(null);
      tried.current = undefined;
      void checkInputs(dir, true);
    }
  };

  /** The plan, read from the booted R1 (or for an assumed just-flashed one with none). Reads only. */
  const makePlan = async () => {
    setErr(null);
    setBusy(true);
    tried.current = booted?.id ?? null;
    log.add(booted ? `setup: reading r1 ${booted.id} for its plan (reading only)` : "setup: no r1 booted: the plan for a just-flashed one, assumed");
    try {
      const p = await install.setupPlan();
      setPlan(p);
      if (!p.assumed && p.r1) {
        setSeen({ r1: p.r1, status: p.status });
        log.add(`setup: r1 ${p.r1}: ${statusWords(p.status)}`);
        p.steps.forEach((s, n) => log.add(`plan: ${n + 1}. ${s.words}`));
        if (p.box_later) log.add("plan: the r1 isn't online: the agent box waits for wi-fi");
        if (!p.steps.length) {
          log.add("plan: nothing to do: it's set up");
          onSetUp({ r1: p.r1, status: p.status });
        }
      }
    } catch (e) {
      setPlan(null);
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  /** What's on the R1 alone, before the checkout is ready. Reads only. */
  const readR1 = async () => {
    setErr(null);
    setBusy(true);
    log.add("setup: reading the r1 (reading only)");
    try {
      const s = await install.setupStatus();
      setSeen(s);
      log.add(`setup: r1 ${s.r1}: ${statusWords(s.status)}`);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  // A plan once the files are ready, and again whenever another R1 is booted. Reading only, so it needn't wait for a
  // click: after the flash, the R1's own plan is here as soon as r1os has started.
  useEffect(() => {
    if (!inputs?.ready || busy || running) return;
    const target = booted?.id ?? null;
    const stale = !plan || (target !== null && plan.r1 !== target);
    if (stale && tried.current !== target) void makePlan();
  }, [inputs?.ready, booted?.id, plan, busy, running]); // eslint-disable-line react-hooks/exhaustive-deps

  const go = async () => {
    if (!plan || plan.assumed) return;
    setErr(null);
    const since = Date.now();
    setRunning({ since, index: 0, of: plan.steps.length, words: "", stepSince: since, lines: [] });
    const offs = await Promise.all([
      install.on<{ index: number; of: number; words: string }>("setup-progress", (p) => {
        log.add(`setup: step ${p.index} of ${p.of}: ${p.words}`);
        setRunning((r) => r && { ...r, ...p, stepSince: Date.now(), lines: [] });
      }),
      install.on<string>("setup-line", (l) => {
        log.add(`setup: ${l}`);
        setRunning((r) => r && { ...r, lines: [...r.lines.slice(-5), l] });
      }),
    ]);
    log.add(`setup: set up clicked; setting up r1 ${plan.r1}`);
    try {
      const done = await install.setupRun();
      setSeen(done);
      onSetUp(done);
      log.add(`setup: done: ${statusWords(done.status)}`);
    } catch (e) {
      fail(e);
    } finally {
      offs.forEach((f) => f());
      setRunning(null);
      // Once it's started, the core keeps no plan: the next is read anew.
      install.setupPlanned().then(setPlan).catch(() => setPlan(null));
    }
  };

  const other = !!plan && !plan.assumed && !!booted && plan.r1 !== booted.id;
  const waiting = flashed && !booted;
  const shown = seen && seen.r1 === (booted?.id ?? known) ? seen : null;
  const done = setUp && setUp.r1 === (booted?.id ?? known) ? setUp : null;
  const total = plan ? plan.steps.reduce((n, s) => n + estimate(s), 0) : 0;
  const r1Sub = !r1
    ? flashed
      ? "r1os is starting after the flash: its first start takes a few minutes, then it shows up here"
      : "plug it in, started into r1os"
    : r1.mode === "adb"
      ? r1.r1os
        ? `booted into r1os ${r1.r1os}: the setup reads and writes it over adb, as root`
        : "booted, but not into r1os's image: the setup needs root over adb (r1os's image, or magisk)"
      : r1.mode === "unauthorized"
        ? "it's asking whether to allow usb debugging: allow it on its screen"
        : r1.mode === "fastboot"
          ? "in its bootloader: leave it (the flash step) and r1os starts"
          : MODE[r1.mode];

  return (
    <StepCard
      what={
        <>
          <p>
            r1os's image has started, but /data is still nearly empty: r1os's core (r1d), its screens, speech-to-text and its models live there, beside the agent box where claude and codex run. this computer puts them on over usb: /data grown to its full size first (online; nothing is erased), each file checked by its sha-256, r1d started, then the agent box built on the r1. only what isn't there yet is done, so it's safe to run again.
          </p>
          <p className="muted small">
            it writes to the r1 but erases nothing, so there's no word to type: the plan is read from the r1 and shown first, and it starts when you click set up. r1os's releases come later: for now, its files come from your r1os checkout.
          </p>
        </>
      }
      hands={
        <ol>
          <li>{flashed ? "leave the r1 plugged in: it starts r1os by itself, a few minutes the first time." : "plug the r1 in, started into r1os."}</li>
          <li>if it asks whether to allow usb debugging, allow it on its screen.</li>
          <li>look over the plan, then click set up. keep the r1 plugged in until it's done.</li>
          <li>afterwards, follow the r1's own screen: wi-fi, a first agent, then signing in with your phone. if the agent box waited for wi-fi, come back here and set up once more.</li>
        </ol>
      }
      wrong={[
        "a file is missing on this computer: build it in your checkout as it says (tools/build-r1d.sh, pnpm --dir shell build, tools/fetch-speech.sh), then check again.",
        "the r1 doesn't show up after the flash: give its first start a few minutes, and allow usb debugging if it asks. if it never starts, rabbit's restore (the flash step) puts its os back.",
        "it says the r1 changed since the plan: something on it changed in between. read it again for a new plan.",
        "r1d doesn't come back: its log is /data/r1os/log/r1d.log on the r1. save this log and ask for help.",
        "the agent box's build fails: usually the r1's wi-fi. check it's online, then set up again.",
        "it stops part way: nothing is erased or lost. set up again, and it carries on with what isn't there yet.",
      ]}
    >
      <Section title="what it puts on" />
      <Item
        label="r1os's files"
        sub={inputs?.from ? `from ${inputs.from}${inputs.ready ? ": each one read and checked" : ""}` : "r1os's releases come later: for now, your r1os checkout, with r1d, the shell and the speech files built in it"}
      >
        <div className="row-buttons">
          {inputs?.from && <Found ok={checking ? null : inputs.ready} yes="here, checked" no="not all there" unknown="checking…" />}
          {inputs?.from && (
            <button className="link" disabled={checking || !!running} onClick={() => void checkInputs(null, true)}>
              check again
            </button>
          )}
          <button className="btn small" disabled={checking || !!running} onClick={() => void pick()}>
            <Icon name="folder-open" weight="bold" size={14} /> {inputs?.from ? "another…" : "choose…"}
          </button>
        </div>
      </Item>
      {inputs?.from && inputs.found.length > 0 && (
        <div className="install-inputs">
          {inputs.found.map((f) => (
            <div key={f.key} className="install-input">
              <span>{INPUT[f.key]}</span>
              <span className="mono muted" title={f.path ?? undefined}>
                {f.path ?? ""}
              </span>
              <span className={`state ${f.here ? "ok" : ""}`}>{f.here ? "here" : f.optional ? "left out" : "missing"}</span>
            </div>
          ))}
        </div>
      )}
      {inputs?.error && !checking && <p className="err install-err">{inputs.error}</p>}

      <Section title="this r1" />
      <Item label="the r1" sub={r1Sub}>
        <Found ok={booted ? true : r1 ? false : null} yes={booted?.r1os ? `r1os ${booted.r1os}` : "booted"} no="not yet" unknown={waiting ? "starting…" : "not plugged in"} />
      </Item>
      {waiting && (
        <div className="install-row">
          <Bar what="waiting for r1os to start, with usb debugging allowed" since={opened} />
        </div>
      )}
      <Item label="what's on it" sub={shown ? statusWords(shown.status) : "/data, r1d, the agent box, and whether it's online: reading only"}>
        <button className="btn small" disabled={!booted || busy || !!running || checking} onClick={() => void (inputs?.ready ? makePlan() : readR1())}>
          {busy ? "reading…" : shown ? "read it again" : "read it"}
        </button>
      </Item>

      {plan && !running && (
        <>
          <Section title="the plan" />
          <div className="install-plan">
            <div className="install-plan-title">
              <Icon name="list-checks" weight="bold" size={16} /> {plan.assumed ? "what it does on a just-flashed r1" : `what it does on r1 ${plan.r1}`}
            </div>
            {plan.assumed && <p className="muted small">assumed, not read: {plan.why ?? "no r1 read yet"}. once the r1 has started r1os, it's read for its own plan.</p>}
            {plan.steps.length ? (
              <ol className="keep-case">
                {plan.steps.map((s) => (
                  <li key={s.what}>{s.words}</li>
                ))}
              </ol>
            ) : (
              <p>nothing to do: everything's on it already.</p>
            )}
            {plan.already.length > 0 && (
              <p className="muted small">
                already there, left alone: <span className="keep-case">{plan.already.join(" · ")}</span>
              </p>
            )}
            {plan.box_later && (
              <p className="small install-note">
                <Icon name="wifi-slash" weight="bold" size={15} />
                <span>the r1 isn't online, so the agent box waits: its build downloads alpine's packages and claude code on the r1. after this, connect it to wi-fi on its own screen, then read it again here for the box.</span>
              </p>
            )}
            {plan.steps.length > 0 && (
              <>
                <p className="muted small">
                  {about(total)} roughly. it writes to the r1 but erases nothing, so there's no word to type: it starts when you click set up. keep the r1 plugged in until it's done.
                </p>
                <div className="row-buttons">
                  <motion.button className="btn small primary" whileTap={{ scale: 0.95 }} disabled={plan.assumed || !booted || other || busy} onClick={() => void go()}>
                    <Icon name="rocket-launch" weight="bold" size={14} /> set up
                  </motion.button>
                  {plan.assumed && <span className="small muted">the r1 is read for its own plan first</span>}
                </div>
              </>
            )}
          </div>
          {other && <p className="err install-err">the r1 plugged in isn't the one this plan was read from: read it again.</p>}
        </>
      )}
      {running && plan && (
        <>
          <Section title="setting up" />
          <div className="install-row install-running">
            <SetupBar steps={plan.steps} running={running} />
            {running.lines.length > 0 && <pre className="install-log-lines install-live">{running.lines.join("\n")}</pre>}
            <p className="muted small">don't unplug the r1 or close this window. its screen may go blank for a moment while r1d starts.</p>
          </div>
        </>
      )}
      {done && !running && (
        <div className="install-row">
          <p className="ok-note">set up: {statusWords(done.status)}.</p>
          <p className="small">
            {boxBuilt(done.status)
              ? "r1os's core is running, and the agent box is built. follow the r1's own screen now: a first agent, then signing in with your phone."
              : "r1os's core is running. the agent box is still to come: connect the r1 to wi-fi on its own screen, then read it again here and set up once more."}
          </p>
        </div>
      )}
      {err && <p className="err install-err">{err}</p>}
    </StepCard>
  );
}

// ---- the log -------------------------------------------------------------------------------------------------------

function LogCard({ log }: { log: Log }) {
  const [open, setOpen] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  const box = useRef<HTMLPreElement>(null);
  useEffect(() => {
    if (open && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [open, log.lines.length]);
  const save = async () => {
    const d = new Date();
    const p = (n: number) => String(n).padStart(2, "0");
    const name = `r1os install log ${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}.${p(d.getMinutes())}.txt`;
    try {
      setSaved(await saveDownload(name, { text: `${log.lines.join("\n")}\n`, mime: "text/plain" }));
    } catch (e) {
      setSaved(words(e));
    }
  };
  return (
    <div className="list-card install-log">
      <div className="install-log-head">
        <button className="link" onClick={() => setOpen(!open)}>
          <Icon name={open ? "caret-down" : "caret-right"} weight="bold" size={14} /> log · {log.lines.length} {log.lines.length === 1 ? "line" : "lines"}
        </button>
        <button className="btn small" disabled={!log.lines.length} onClick={() => void save()}>
          <Icon name="file-text" weight="bold" size={14} /> save
        </button>
      </div>
      {open && (
        <pre ref={box} className="install-log-lines">
          {log.lines.join("\n")}
        </pre>
      )}
      <p className="muted small list-foot">
        what this wizard did and saw, for you or for whoever helps you. r1s are named by a hash of their serial, never the serial or the imei.
        {saved && <> saved: {saved}</>}
      </p>
    </div>
  );
}
