// Wheel focus for apps (ADR-0006 §4, ADR-0025).
//
// The runtime's components are touch/mouse controls. Rather than teach each one about the wheel,
// the shell keeps a focus on whatever is interactive inside the app: the wheel moves it and a tap
// activates it. Inputs get the IME, buttons and toggles get clicked. Works for every catalog
// component without touching the runtime. apps/theme.css draws it as the native lists do: a solid
// fill in the app's accent.
//
// Like the native lists, the wheel also scrolls: when the next stop is off screen (a chart, a long
// list without buttons in between), or there is none, a tick scrolls instead, just far enough to show
// the next block whole (a chart, a card, a list row), so nothing stays cut off at the edge. Back (one
// tick above the first stop) only comes at the top.
//
// A component can take the wheel (r1os ADR-0038; @r1os/runtime's wheel-input.ts): a value wheel, a game. While one is
// engaged (`data-r1-engaged`), the wheel, the click, the double click and the button go to it as DOM events instead
// of moving the focus, and the shell is told (`onCapture`) so it passes the double click and the button too.

import { BUTTON_EVENT, CANCEL_EVENT, ENGAGED_ATTR, PRESS_EVENT, RELEASE_EVENT, WHEEL_EVENT } from "@r1os/runtime";

const INTERACTIVE = [
  "button:not([disabled])",
  "input:not([disabled]):not([type=hidden])",
  "textarea:not([disabled])",
  "select:not([disabled])",
  "[role=button]",
  "[role=switch]",
  "[role=checkbox]",
  "[role=tab]",
  "a[href]",
].join(",");

const RING = "r1-focus";
/** A tick's scroll when there's no block to bring in (the page's end padding), in CSS px (dp on the R1). */
const STEP = 72;
/** The blocks a tick brings fully into view: an app's top-level blocks, a card's, and list rows. */
const UNITS = ".enee-scroll > * > *, .enee-card > *, .enee-list__row, .enee-tabs";
/** Room left past a revealed block, and kept above or below it when one is taller than the screen. */
const MARGIN = 12;

export class WheelFocus {
  /** What has the focus: the element, not its place, so it stays put when stops appear around it (a list growing,
   *  a logic view filling in above it). */
  private focused: HTMLElement | null = null;
  private observer: MutationObserver | null = null;
  private wasCaptured = false;

  /** `onCapture(true)` when a component takes the wheel, `(false)` when it gives it back. */
  constructor(
    private root: HTMLElement,
    onCapture?: (on: boolean) => void,
  ) {
    if (onCapture) {
      this.observer = new MutationObserver(() => {
        const now = this.captured();
        if (now !== this.wasCaptured) {
          this.wasCaptured = now;
          onCapture(now);
        }
      });
      this.observer.observe(root, { subtree: true, childList: true, attributes: true, attributeFilter: [ENGAGED_ATTR] });
    }
  }

  /** The component holding the wheel, if one is. */
  engaged(): HTMLElement | null {
    return this.root.querySelector<HTMLElement>(`[${ENGAGED_ATTR}]`);
  }

  captured(): boolean {
    return this.engaged() !== null;
  }

  private send(name: string, detail?: unknown): boolean {
    const el = this.engaged();
    if (!el) return false;
    el.dispatchEvent(new CustomEvent(name, { detail }));
    return true;
  }

  /** A double click while a component has the wheel: it puts back (a wheel) or pauses (a game). */
  cancel(): boolean {
    return this.send(CANCEL_EVENT);
  }

  /** The side button's press or release, for the component holding the wheel. */
  button(down: boolean): boolean {
    return this.send(BUTTON_EVENT, { down });
  }

  /** The shell takes the wheel back (the app was hidden): the component keeps what it has, or pauses. */
  release(): boolean {
    return this.send(RELEASE_EVENT);
  }

  dispose(): void {
    this.observer?.disconnect();
  }

  private targets(): HTMLElement[] {
    return [...this.root.querySelectorAll<HTMLElement>(INTERACTIVE)].filter((el) => {
      const r = el.getBoundingClientRect();
      return r.width > 0 && r.height > 0 && !el.closest("[aria-hidden=true]");
    });
  }

  /** What scrolls: the app's scroller, else the page. */
  private scroller(): HTMLElement {
    return this.root.querySelector<HTMLElement>(".enee-scroll") ?? (document.scrollingElement as HTMLElement);
  }

  private current(list = this.targets()): HTMLElement | undefined {
    const el = this.focused;
    return el && list.includes(el) ? el : undefined;
  }

  private focus(list: HTMLElement[], i: number): void {
    this.focused?.classList.remove(RING);
    const el = list[i]!;
    this.focused = el;
    el.classList.add(RING);
    // Into view if it isn't, scrolling only the app (scrollIntoView would move the page's frame too).
    const view = this.view();
    const r = this.pos(el);
    if (r.top < view.top + MARGIN / 2) this.scroll(r.top - view.top - MARGIN);
    else if (r.bottom > view.bottom - MARGIN / 2) this.scroll(r.bottom - view.bottom + MARGIN);
  }

  /** The scroller's box on screen. */
  private view(): { top: number; bottom: number } {
    const r = this.scroller().getBoundingClientRect();
    return { top: r.top, bottom: r.bottom };
  }

  /** Where `el` will be on screen once a scroll under way ends. */
  private pos(el: HTMLElement): { top: number; bottom: number } {
    const s = this.scroller();
    const shift = s.scrollTop - this.base(s);
    const r = el.getBoundingClientRect();
    return { top: r.top + shift, bottom: r.bottom + shift };
  }

  /** Where the page is going: a smooth scroll still under way counts as done, so quick ticks add up. */
  private pending: { top: number; at: number } | null = null;

  private base(s: HTMLElement): number {
    return this.pending && performance.now() - this.pending.at < 400 ? this.pending.top : s.scrollTop;
  }

  private scroll(px: number): boolean {
    const s = this.scroller();
    const before = this.base(s);
    const max = s.scrollHeight - s.clientHeight;
    const to = Math.max(0, Math.min(max, Math.round(before + px)));
    if (to === before) return false;
    this.pending = { top: to, at: performance.now() };
    s.scrollTo({ top: to, behavior: "smooth" });
    return true;
  }

  /** The smallest blocks that can be shown whole (a list's rows rather than the list). */
  private units(): HTMLElement[] {
    const all = [...this.root.querySelectorAll<HTMLElement>(UNITS)].filter((el) => el.getBoundingClientRect().height > 0);
    return all.filter((el) => !all.some((other) => other !== el && el.contains(other)));
  }

  /** One tick with no stop to go to: bring the next block that's cut off (or hidden) fully into view. */
  private reveal(dir: 1 | -1): boolean {
    const s = this.scroller();
    const rect = s.getBoundingClientRect();
    // In the page's own coordinates, as it will be once a scroll under way ends.
    const at = (el: HTMLElement) => {
      const r = el.getBoundingClientRect();
      return { top: r.top - rect.top + s.scrollTop, bottom: r.bottom - rect.top + s.scrollTop };
    };
    const top = this.base(s);
    const bottom = top + s.clientHeight;
    const most = s.clientHeight - 4 * MARGIN;
    const units = this.units().map(at);
    if (dir > 0) {
      const u = units.find((u) => u.bottom > bottom + 1);
      return this.scroll(u ? Math.min(u.bottom - bottom + MARGIN, most) : STEP);
    }
    const u = units.filter((u) => u.top < top - 1).pop();
    return this.scroll(u ? -Math.min(top - u.top + MARGIN, most) : -STEP);
  }

  /** One tick down (+1) or up (-1). Returns false when nothing moved (the end of the app). */
  private tick(dir: 1 | -1): boolean {
    const list = this.targets();
    const view = this.view();
    const inView = (el: HTMLElement) => {
      const r = this.pos(el);
      return r.bottom > view.top && r.top < view.bottom;
    };
    let cur = this.current(list);
    // The focus scrolled away, or its stop is gone: carry on from what's on screen.
    if (!cur || !inView(cur)) {
      this.reset();
      cur = undefined;
    }
    let next: number;
    if (cur) {
      next = list.indexOf(cur) + dir;
    } else if (dir > 0) {
      next = list.findIndex((el) => this.pos(el).bottom > view.top);
      if (next < 0) next = list.length;
    } else {
      next = -1;
      list.forEach((el, i) => {
        if (this.pos(el).top < view.bottom) next = i;
      });
    }
    const target = list[next];
    if (!target) return this.reveal(dir);
    const r = this.pos(target);
    // The next stop is off screen: show what's between first, a whole block at a time.
    if ((dir > 0 && r.top >= view.bottom) || (dir < 0 && r.bottom <= view.top)) return this.reveal(dir);
    this.focus(list, next);
    return true;
  }

  /** Moves the focus by `delta` ticks, scrolling where there's no stop on screen; or turns what has the wheel. */
  move(delta: number): void {
    if (this.send(WHEEL_EVENT, { delta })) return;
    const dir = delta > 0 ? 1 : -1;
    for (let i = 0; i < Math.abs(delta); i++) {
      if (!this.tick(dir)) break;
    }
  }

  /** Nothing above: the page is at its top, and the focus is on the first stop (or there's none). */
  atTop(): boolean {
    if (this.captured()) return false;
    const s = this.scroller();
    const list = this.targets();
    const cur = this.current(list);
    return (!cur || list.indexOf(cur) <= 0) && this.base(s) <= 0;
  }

  /** Activates the focused target (a click on what has the wheel). Returns false when nothing is focused. */
  activate(): boolean {
    if (this.send(PRESS_EVENT)) return true;
    const el = this.current();
    if (!el) return false;
    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
      el.focus();
      if (el instanceof HTMLInputElement && (el.type === "checkbox" || el.type === "radio")) el.click();
    } else {
      el.click();
    }
    return true;
  }

  /** Drops the focus (e.g. the view changed and the target may be gone). */
  reset(): void {
    this.focused?.classList.remove(RING);
    this.focused = null;
  }
}
