// How a component takes the R1's wheel (r1os ADR-0038). The runtime knows nothing of where input comes from; the
// shell's focus ring (shell/src/apps/focus.ts) owns the wheel and the button, and hands them to a component that
// asks for them:
//
// - A component that can take the wheel marks its element `data-r1-wheel` (and is a focus stop: a button or
//   role=button). Clicked while focused, it decides to take the wheel and marks itself `data-r1-engaged`.
// - While an element is engaged, the shell sends it, as DOM events on the element:
//     r1:wheel  {delta}  each detent (+1 down, −1 up)
//     r1:press           a click
//     r1:cancel          a double click (the shell's Back, while engaged)
//     r1:button {down}   the side button's press and release, for games that need them as they happen
//     r1:release         the shell takes the wheel back (the app was hidden): keep what you have, or pause
//   and the focus doesn't move. Removing `data-r1-engaged` hands the wheel back.

import { type RefObject, useEffect, useRef } from "react";

export const WHEEL_ATTR = "data-r1-wheel";
export const ENGAGED_ATTR = "data-r1-engaged";

export const WHEEL_EVENT = "r1:wheel";
export const PRESS_EVENT = "r1:press";
export const CANCEL_EVENT = "r1:cancel";
export const BUTTON_EVENT = "r1:button";
export const RELEASE_EVENT = "r1:release";

export interface WheelHandlers {
  wheel?: (delta: number) => void;
  press?: () => void;
  cancel?: () => void;
  button?: (down: boolean) => void;
  release?: () => void;
}

/** Listens for the shell's input on `ref` while it's engaged; handlers can change every render. */
export function useWheelInput(ref: RefObject<HTMLElement | null>, handlers: WheelHandlers): void {
  const current = useRef(handlers);
  current.current = handlers;
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const wheel = (e: Event) => current.current.wheel?.((e as CustomEvent<{ delta: number }>).detail.delta);
    const press = () => current.current.press?.();
    const cancel = () => current.current.cancel?.();
    const button = (e: Event) => current.current.button?.((e as CustomEvent<{ down: boolean }>).detail.down);
    const release = () => current.current.release?.();
    el.addEventListener(RELEASE_EVENT, release);
    el.addEventListener(WHEEL_EVENT, wheel);
    el.addEventListener(PRESS_EVENT, press);
    el.addEventListener(CANCEL_EVENT, cancel);
    el.addEventListener(BUTTON_EVENT, button);
    return () => {
      el.removeEventListener(WHEEL_EVENT, wheel);
      el.removeEventListener(PRESS_EVENT, press);
      el.removeEventListener(CANCEL_EVENT, cancel);
      el.removeEventListener(BUTTON_EVENT, button);
      el.removeEventListener(RELEASE_EVENT, release);
    };
  }, [ref]);
}
