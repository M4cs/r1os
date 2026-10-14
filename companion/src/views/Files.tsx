// An R1's files from the computer (docs/specs/companion.md §6): its photos, its recordings (with what was said) and
// its journal, to look at, save to the PC and delete. Photos dropped on the page (or picked) go to the R1's photos.

import { Icon } from "@r1os/icons";
import { AnimatePresence, LayoutGroup, motion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { type R1Info, api, apiBytes, mediaUrl, saveDownload, upload, useGet } from "../link";
import { soft, spring } from "../motion";
import { Empty, Sheet, Twice, ago, size } from "./parts";
import { Item, Section } from "./Settings";

interface Media {
  id: string;
  kind: "photo" | "recording";
  at: number;
  mime: string;
  bytes: number;
  width?: number;
  height?: number;
  duration_ms?: number;
  source: string;
  label: string;
  transcript?: string;
}
interface Entry {
  id: string;
  at: number;
  kind: string;
  text: string;
  media?: string;
  media_item?: Media;
}
const TABS: ["photos" | "recordings" | "journal", string][] = [["photos", "photos"], ["recordings", "recordings"], ["journal", "journal"]];

/** A name for a file saved from the R1: when it was taken. */
function fileName(m: Media): string {
  const d = new Date(m.at);
  const stamp = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}.${String(d.getMinutes()).padStart(2, "0")}`;
  return `r1 ${m.kind} ${stamp}.${m.mime.includes("wav") ? "wav" : m.mime.includes("png") ? "png" : "jpg"}`;
}

export function Files({ r1 }: { r1: R1Info }) {
  const [tab, setTab] = useState<(typeof TABS)[number][0]>("photos");
  const [err, setErr] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const fail = (e: unknown) => setErr(e instanceof Error ? e.message : String(e));
  const save = async (m: Media) => {
    const { mime, base64 } = await apiBytes(r1.id, `/api/media/${m.id}/file`);
    const where = await saveDownload(fileName(m), { base64, mime });
    setNote(`saved ${where.split(/[\\/]/).pop()} in downloads`);
  };
  return (
    <div className="files-page">
      <LayoutGroup id="files-tabs">
        <div className="seg files-tabs">
          {TABS.map(([t, label]) => (
            <button key={t} className={`seg-item ${t === tab ? "on" : ""}`} onClick={() => setTab(t)}>
              {t === tab && <motion.span className="seg-fill" layoutId="fill" transition={spring} />}
              <span>{label}</span>
            </button>
          ))}
        </div>
      </LayoutGroup>
      {err && <p className="err">{err}</p>}
      <AnimatePresence>
        {note && (
          <motion.p className="ok-note" initial={{ opacity: 0, y: -4 }} animate={{ opacity: 1, y: 0 }} exit={{ opacity: 0 }} onAnimationComplete={() => setTimeout(() => setNote(null), 2600)}>
            {note}
          </motion.p>
        )}
      </AnimatePresence>
      <AnimatePresence mode="wait">
        <motion.div key={tab} initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0, transition: soft }} exit={{ opacity: 0, transition: { duration: 0.1 } }}>
          {tab === "photos" && <Photos r1={r1} save={(m) => save(m).catch(fail)} fail={fail} />}
          {tab === "recordings" && <Recordings r1={r1} save={(m) => save(m).catch(fail)} fail={fail} />}
          {tab === "journal" && <Journal r1={r1} fail={fail} />}
        </motion.div>
      </AnimatePresence>
    </div>
  );
}

// ---- photos ------------------------------------------------------------------------------------------------

function Photos({ r1, save, fail }: { r1: R1Info; save: (m: Media) => void; fail: (e: unknown) => void }) {
  const { data, reload } = useGet<{ media: Media[] }>(r1.id, "/api/media?kind=photo&limit=200", ["media_changed"]);
  const [open, setOpen] = useState<Media | null>(null);
  const [over, setOver] = useState(false);
  const [adding, setAdding] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const photos = data?.media ?? [];

  const add = async (files: File[]) => {
    const images = files.filter((f) => f.type.startsWith("image/"));
    setAdding(images.length);
    for (const f of images) {
      try {
        await upload(r1.id, "POST", "/api/media?source=computer", "image/jpeg", await asJpeg(f));
      } catch (e) {
        fail(e);
      }
      setAdding((n) => n - 1);
    }
    reload();
  };

  return (
    <div
      className={`drop ${over ? "over" : ""}`}
      onDragOver={(e) => {
        e.preventDefault();
        setOver(true);
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        e.preventDefault();
        setOver(false);
        void add([...e.dataTransfer.files]);
      }}
    >
      <div className="files-bar">
        <span className="muted small">{data ? `${photos.length} photos · drop photos here to put them on your r1` : "…"}</span>
        <input ref={input} type="file" accept="image/*" multiple hidden onChange={(e) => e.target.files && void add([...e.target.files])} />
        <button className="btn small" onClick={() => input.current?.click()} disabled={adding > 0}>
          <Icon name="upload-simple" weight="bold" size={14} />
          {adding > 0 ? `adding ${adding}…` : "add photos"}
        </button>
      </div>
      {data && photos.length === 0 && <Empty icon="image">no photos yet: take some with the r1's camera, or drop some here</Empty>}
      <div className="photo-wall">
        {photos.map((m, i) => (
          <Thumb key={m.id} r1={r1} m={m} i={i} onOpen={() => setOpen(m)} />
        ))}
      </div>
      <Sheet open={!!open} onClose={() => setOpen(null)} title={open ? `${ago(open.at)} · ${open.source === "computer" ? "from a computer" : open.source}` : ""}>
        {open && (
          <div className="photo-big">
            <Pic r1={r1} m={open} />
            <div className="row-buttons">
              <span className="muted small">
                {open.width}×{open.height} · {size(open.bytes)}
              </span>
              <button className="btn small" onClick={() => save(open)}>
                <Icon name="download-simple" weight="bold" size={14} /> save to this computer
              </button>
              <Twice label="delete" armed="click again: delete it" icon="trash" onConfirm={() => void api(r1.id, "DELETE", `/api/media/${open.id}`).then(() => (setOpen(null), reload())).catch(fail)} />
            </div>
          </div>
        )}
      </Sheet>
    </div>
  );
}

/** A photo in the wall, fetched once it's on screen. */
function Thumb({ r1, m, i, onOpen }: { r1: R1Info; m: Media; i: number; onOpen: () => void }) {
  const ref = useRef<HTMLButtonElement>(null);
  const [seen, setSeen] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const io = new IntersectionObserver(([e]) => e.isIntersecting && setSeen(true), { rootMargin: "200px" });
    io.observe(el);
    return () => io.disconnect();
  }, []);
  return (
    <motion.button ref={ref} className="photo-tile" onClick={onOpen} initial={{ opacity: 0, scale: 0.96 }} animate={{ opacity: 1, scale: 1, transition: { ...soft, delay: Math.min(i, 12) * 0.02 } }} whileHover={{ y: -3 }}>
      {seen ? <Pic r1={r1} m={m} /> : <span className="photo-thumb loading" />}
    </motion.button>
  );
}

function Pic({ r1, m }: { r1: R1Info; m: Media }) {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    let gone = false;
    mediaUrl(r1.id, m.id).then((u) => !gone && setSrc(u)).catch(() => undefined);
    return () => {
      gone = true;
    };
  }, [r1.id, m.id]);
  return src ? <img src={src} alt="" /> : <span className="photo-thumb loading" />;
}

/** Any picture as a JPEG the R1 takes (at most 2048 px on its long side): base64. */
/** A picture as a JPEG of at most 2048 px, base64 (the R1 keeps photos so). */
export async function asJpeg(f: File): Promise<string> {
  const img = await createImageBitmap(f);
  const k = Math.min(1, 2048 / Math.max(img.width, img.height));
  const c = document.createElement("canvas");
  c.width = Math.round(img.width * k);
  c.height = Math.round(img.height * k);
  c.getContext("2d")!.drawImage(img, 0, 0, c.width, c.height);
  const blob = await new Promise<Blob>((ok, no) => c.toBlob((b) => (b ? ok(b) : no(new Error("couldn't make a jpeg"))), "image/jpeg", 0.9));
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

// ---- recordings --------------------------------------------------------------------------------------------

function Recordings({ r1, save, fail }: { r1: R1Info; save: (m: Media) => void; fail: (e: unknown) => void }) {
  const { data, reload } = useGet<{ media: Media[] }>(r1.id, "/api/media?kind=recording&limit=200", ["media_changed", "journal_changed"]);
  const list = data?.media ?? [];
  return (
    <motion.div className="list-card">
      <Section title={`recordings${list.length ? ` · ${list.length}` : ""}`} />
      {data && list.length === 0 && <Empty icon="microphone">no recordings: the r1's recorder keeps them here</Empty>}
      {list.map((m) => (
        <Item key={m.id} label={m.label || (m.transcript ? m.transcript.slice(0, 80) : "a recording")} sub={`${ago(m.at)} · ${Math.round((m.duration_ms ?? 0) / 1000)} s · ${m.source}`} wide>
          {m.transcript && m.label && <p className="transcript">{m.transcript}</p>}
          <div className="row-buttons">
            <Player r1={r1} m={m} />
            <button className="btn small" onClick={() => save(m)}>
              <Icon name="download-simple" weight="bold" size={14} /> save
            </button>
            <Twice label="delete" armed="click again" icon="trash" onConfirm={() => void api(r1.id, "DELETE", `/api/media/${m.id}`).then(reload).catch(fail)} />
          </div>
        </Item>
      ))}
    </motion.div>
  );
}

/** Plays a recording here: fetched the first time it's played. */
function Player({ r1, m }: { r1: R1Info; m: Media }) {
  const audio = useRef<HTMLAudioElement | null>(null);
  const [playing, setPlaying] = useState(false);
  const [loading, setLoading] = useState(false);
  useEffect(() => () => audio.current?.pause(), []);
  const toggle = async () => {
    if (!audio.current) {
      setLoading(true);
      const url = await mediaUrl(r1.id, m.id).finally(() => setLoading(false));
      audio.current = new Audio(url);
      audio.current.onended = () => setPlaying(false);
    }
    if (playing) {
      audio.current.pause();
      setPlaying(false);
    } else {
      await audio.current.play();
      setPlaying(true);
    }
  };
  return (
    <motion.button className="btn small" whileTap={{ scale: 0.95 }} onClick={() => void toggle()}>
      <Icon name={playing ? "pause" : "play"} weight="fill" size={14} />
      {loading ? "…" : playing ? "pause" : "play"}
    </motion.button>
  );
}

// ---- the journal -------------------------------------------------------------------------------------------

function Journal({ r1, fail }: { r1: R1Info; fail: (e: unknown) => void }) {
  const { data, reload } = useGet<{ entries: Entry[] }>(r1.id, "/api/journal", ["journal_changed"]);
  const list = data?.entries ?? [];
  const words = (e: Entry) => e.text || e.media_item?.transcript || (e.kind === "photo" ? "a photo" : e.kind === "recording" ? "a voice note" : "a note");
  return (
    <motion.div className="list-card">
      <Section title={`the journal${list.length ? ` · ${list.length}` : ""}`} />
      {data && list.length === 0 && <Empty icon="notebook">nothing in the journal yet</Empty>}
      {list.map((e) => (
        <Item key={e.id} label={words(e)} sub={`${ago(e.at)} · ${e.kind}`} wide>
          {e.kind === "photo" && e.media_item && (
            <div className="journal-photo">
              <Pic r1={r1} m={e.media_item} />
            </div>
          )}
          <div className="row-buttons">
            {e.kind === "recording" && e.media_item && <Player r1={r1} m={e.media_item} />}
            <Twice label="delete" armed="click again" icon="trash" onConfirm={() => void api(r1.id, "DELETE", `/api/journal/${e.id}`).then(reload).catch(fail)} />
          </div>
        </Item>
      ))}
    </motion.div>
  );
}
