// Parts the pages share beyond the list's own (./Settings): a text field that saves when you're done, a button you
// click twice for what can't be undone, a sheet over the page, and what an empty list says.

import { Icon } from "@r1os/icons";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { soft, spring } from "../motion";

/** Text that's saved when you press Enter or leave it (Escape puts it back). */
export function Field({ value, onSave, placeholder, max, wide, password }: { value: string; onSave: (v: string) => void; placeholder?: string; max?: number; wide?: boolean; password?: boolean }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  const done = () => v.trim() !== value && onSave(v.trim());
  return (
    <input
      className={`field ${wide ? "wide" : ""}`}
      type={password ? "password" : "text"}
      value={v}
      maxLength={max}
      placeholder={placeholder}
      spellCheck={false}
      onChange={(e) => setV(e.target.value)}
      onBlur={done}
      onKeyDown={(e) => {
        if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        if (e.key === "Escape") {
          setV(value);
          setTimeout(() => (e.target as HTMLInputElement).blur());
        }
      }}
    />
  );
}

/** A button for what can't be undone: the first click arms it ("click again: …") for three seconds. */
export function Twice({ label, armed, onConfirm, disabled, icon }: { label: string; armed: string; onConfirm: () => void; disabled?: boolean; icon?: string }) {
  const [on, setOn] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  return (
    <motion.button
      className={`btn small ${on ? "danger" : ""}`}
      disabled={disabled}
      whileTap={{ scale: 0.95 }}
      onClick={() => {
        if (!on) {
          setOn(true);
          timer.current = setTimeout(() => setOn(false), 3000);
          return;
        }
        clearTimeout(timer.current);
        setOn(false);
        onConfirm();
      }}
    >
      {icon && <Icon name={icon} weight="bold" size={14} />}
      {on ? armed : label}
    </motion.button>
  );
}

/** A sheet over the window (a chat's messages, a photo): Escape or a click outside closes it. It's drawn at the top
 *  of the page's tree, since a page moving in would otherwise carry it along. */
export function Sheet({ open, onClose, title, children }: { open: boolean; onClose: () => void; title?: string; children: React.ReactNode }) {
  useEffect(() => {
    if (!open) return;
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [open, onClose]);
  return createPortal(
    <AnimatePresence>
      {open && (
        <motion.div className="sheet-back" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} onClick={onClose}>
          <motion.div
            className="sheet"
            initial={{ opacity: 0, y: 24, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1, transition: spring }}
            exit={{ opacity: 0, y: 16, transition: { duration: 0.15 } }}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="sheet-head">
              <div className="sheet-title">{title}</div>
              <button className="winbtn" onClick={onClose} title="close">
                <Icon name="x" size={16} />
              </button>
            </div>
            <div className="sheet-body">{children}</div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>,
    document.body,
  );
}

/** A list with nothing in it yet. */
export function Empty({ icon, children }: { icon: string; children: React.ReactNode }) {
  return (
    <motion.div className="empty" initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0, transition: soft }}>
      <Icon name={icon} weight="bold" size={22} />
      <span>{children}</span>
    </motion.div>
  );
}

/** "3 min ago", "yesterday", "mar 4": when something happened, as you'd say it. */
export function ago(ms: number): string {
  const s = (Date.now() - ms) / 1000;
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  if (s < 172800) return "yesterday";
  return new Date(ms).toLocaleDateString("en-US", { month: "short", day: "numeric" }).toLowerCase();
}

/** 1.2 mb, 340 kb. */
export function size(bytes: number): string {
  if (bytes >= 1 << 30) return `${(bytes / (1 << 30)).toFixed(1)} gb`;
  if (bytes >= 1 << 20) return `${(bytes / (1 << 20)).toFixed(1)} mb`;
  return `${Math.max(1, Math.round(bytes / 1024))} kb`;
}
