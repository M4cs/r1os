// Minimal in-app toasts. State lives in a tiny external store so a toast never re-renders the app tree.

import { useSyncExternalStore } from "react";

import type { ToastTone } from "../types";

export interface Toast {
  id: number;
  message: string;
  tone: ToastTone;
  leaving: boolean;
}

const VISIBLE = 3;
const DURATION_MS = 2400;
const EXIT_MS = 160;

export interface ToastStore {
  push(message: string, tone?: ToastTone): void;
  dismiss(id: number): void;
  subscribe(listener: () => void): () => void;
  getSnapshot(): readonly Toast[];
  dispose(): void;
}

export function createToastStore(): ToastStore {
  let toasts: readonly Toast[] = [];
  let nextId = 1;
  const listeners = new Set<() => void>();
  const timers = new Map<number, ReturnType<typeof setTimeout>>();
  const emit = () => listeners.forEach((l) => l());

  const remove = (id: number) => {
    timers.delete(id);
    toasts = toasts.filter((t) => t.id !== id);
    emit();
  };
  const leave = (id: number) => {
    const t = toasts.find((x) => x.id === id);
    if (!t || t.leaving) return;
    toasts = toasts.map((x) => (x.id === id ? { ...x, leaving: true } : x));
    emit();
    clearTimeout(timers.get(id));
    timers.set(id, setTimeout(() => remove(id), EXIT_MS));
  };

  return {
    push(message, tone = "default") {
      const id = nextId++;
      // Oldest live toasts beyond the cap leave immediately (keeps bursts, e.g. stress runs, cheap).
      const live = toasts.filter((t) => !t.leaving);
      const overflow = live.length - (VISIBLE - 1);
      const evict = overflow > 0 ? new Set(live.slice(0, overflow).map((t) => t.id)) : new Set<number>();
      toasts = [...toasts.filter((t) => !evict.has(t.id)), { id, message, tone, leaving: false }];
      for (const old of evict) {
        clearTimeout(timers.get(old));
        timers.delete(old);
      }
      timers.set(id, setTimeout(() => leave(id), tone === "danger" ? DURATION_MS * 1.6 : DURATION_MS));
      emit();
    },
    dismiss: leave,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    getSnapshot: () => toasts,
    dispose() {
      timers.forEach((t) => clearTimeout(t));
      timers.clear();
      toasts = [];
    },
  };
}

export function Toasts({ store }: { store: ToastStore }) {
  const toasts = useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
  return (
    <div className="enee-toasts" role="status" aria-live="polite">
      {toasts.map((t) => (
        <div
          key={t.id}
          className="enee-toast"
          data-tone={t.tone}
          data-leaving={t.leaving ? "true" : undefined}
          onClick={() => store.dismiss(t.id)}
        >
          <span className="enee-toast__dot" aria-hidden />
          <span className="enee-toast__text">{t.message}</span>
        </div>
      ))}
    </div>
  );
}
