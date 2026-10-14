// Themed confirm dialog. It serves two sources:
//   1. our runner (`ActionDef.confirm`, contract §5 step 2) via a small external store, and
//   2. json-render's own binding-level `confirm`, read from `useActions().pendingConfirmation` of <EneeApp>'s inner
//      ActionProvider. JSONUIProvider's built-in, inline-styled white dialog listens to the *outer* provider, which
//      never receives an action, so it never shows.

import { useActions } from "@json-render/react";
import { useEffect, useId, useRef, useSyncExternalStore } from "react";

import { Icon } from "../components/icons";
import type { ConfirmRequest } from "../types";

export interface ConfirmController {
  /** Show the dialog; resolves `true` (confirmed) or `false` (cancelled or superseded). */
  request(request: ConfirmRequest): Promise<boolean>;
  answer(ok: boolean): void;
  subscribe(listener: () => void): () => void;
  getSnapshot(): ConfirmRequest | null;
}

export function createConfirmController(): ConfirmController {
  let pending: { request: ConfirmRequest; resolve: (ok: boolean) => void } | null = null;
  const listeners = new Set<() => void>();
  const emit = () => listeners.forEach((l) => l());
  return {
    request(request) {
      pending?.resolve(false);
      return new Promise<boolean>((resolve) => {
        pending = { request, resolve };
        emit();
      });
    },
    answer(ok) {
      const p = pending;
      pending = null;
      emit();
      p?.resolve(ok);
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    getSnapshot: () => pending?.request ?? null,
  };
}

export function ConfirmLayer({ controller }: { controller: ConfirmController }) {
  const own = useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);
  const { pendingConfirmation, confirm, cancel } = useActions();
  const native = pendingConfirmation?.action.confirm;

  if (own) return <ConfirmDialog request={own} onAnswer={controller.answer} />;
  if (native) {
    const request: ConfirmRequest = {
      title: native.title,
      message: native.message,
      variant: native.variant === "danger" ? "danger" : "default",
      ...(native.confirmLabel ? { confirmLabel: native.confirmLabel } : {}),
      ...(native.cancelLabel ? { cancelLabel: native.cancelLabel } : {}),
    };
    return <ConfirmDialog request={request} onAnswer={(ok) => (ok ? confirm() : cancel())} />;
  }
  return null;
}

function ConfirmDialog({ request, onAnswer }: { request: ConfirmRequest; onAnswer: (ok: boolean) => void }) {
  const titleId = useId();
  const messageId = useId();
  const danger = request.variant === "danger";
  const safeButton = useRef<HTMLButtonElement>(null);
  const answer = useRef(onAnswer);
  answer.current = onAnswer;

  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    safeButton.current?.focus({ preventScroll: true });
    // Capture phase: Esc closes the dialog before the overlay's own Esc handler (collapse) sees it. Also stop
    // *immediate* propagation: for events targeted at `window` itself, all window listeners run at-target, where
    // stopPropagation alone doesn't stop the shell's listener.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      e.stopImmediatePropagation();
      answer.current(false);
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      previous?.focus({ preventScroll: true });
    };
  }, []);

  return (
    <div
      className="enee-dialog-layer"
      onPointerDown={(e) => {
        if (e.target === e.currentTarget) answer.current(false);
      }}
    >
      <div
        className="enee-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={request.message ? messageId : undefined}
        data-variant={danger ? "danger" : "default"}
      >
        {danger ? (
          <span className="enee-dialog__icon" aria-hidden>
            <Icon name="trash" size={18} />
          </span>
        ) : null}
        <h2 id={titleId} className="enee-dialog__title">
          {request.title}
        </h2>
        {request.message ? (
          <p id={messageId} className="enee-dialog__message">
            {request.message}
          </p>
        ) : null}
        <div className="enee-dialog__actions">
          <button
            ref={danger ? safeButton : undefined}
            type="button"
            className="enee-button"
            data-variant="secondary"
            data-enee-confirm="cancel"
            onClick={() => answer.current(false)}
          >
            {request.cancelLabel ?? "Cancel"}
          </button>
          <button
            ref={danger ? undefined : safeButton}
            type="button"
            className="enee-button"
            data-variant={danger ? "destructive" : "primary"}
            data-enee-confirm="accept"
            onClick={() => answer.current(true)}
          >
            {request.confirmLabel ?? "Confirm"}
          </button>
        </div>
      </div>
    </div>
  );
}
