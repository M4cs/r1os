// An installed enee app, rendered inside the shell with the ported runtime (ADR-0006).

import { Component, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { EneeApp, EneeError, type CompiledApp } from "@r1os/runtime";
import "@r1os/runtime/styles.css";
// r1os's own look over the runtime's (ADR-0025): after it, so it wins.
import "./theme.css";
import { useAppInput } from "../input";
import { native, onEvent } from "../r1";
import { appsApi, createR1Host, reportError } from "./host";
import { WheelFocus } from "./focus";

export function AppView({ id }: { id: string }) {
  const [compiled, setCompiled] = useState<CompiledApp | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const host = useMemo(() => createR1Host(id), [id]);
  const box = useRef<HTMLDivElement>(null);
  const focus = useRef<WheelFocus | null>(null);

  const [version, setVersion] = useState(0);
  useEffect(() => {
    let alive = true;
    appsApi.get(id).then(
      (r) => alive && setCompiled(r.app),
      (e) => alive && setFailed(String(e)),
    );
    return () => {
      alive = false;
    };
  }, [id, version]);
  // An edit or undo installs a new version: show it.
  useEffect(
    () =>
      onEvent((ev) => {
        if (ev.type === "app_installed" && ev.app === id) setVersion((n) => n + 1);
        if (ev.type === "app_removed" && ev.app === id) setFailed("This app was removed.");
      }),
    [id],
  );

  useEffect(() => {
    if (!box.current) return;
    // A component taking the wheel (ADR-0038): the native shell then passes the double click and the button too.
    const f = new WheelFocus(box.current, (on) => native?.capture?.(on));
    focus.current = f;
    return () => f.dispose();
  }, [compiled]);

  // The native shell takes the wheel back (the app hidden): a wheel keeps its value, a game pauses.
  useEffect(() => {
    const on = () => focus.current?.release();
    window.addEventListener("r1release", on);
    return () => window.removeEventListener("r1release", on);
  }, []);

  // The native shell's Back lit up (or went out): one highlight at a time.
  useEffect(() => {
    const on = (e: Event) => {
      if ((e as CustomEvent<boolean>).detail) focus.current?.move(1);
      else focus.current?.reset();
    };
    window.addEventListener("r1focus", on);
    return () => window.removeEventListener("r1focus", on);
  }, []);

  // Wheel = move the focus ring, tap = activate it. An unconsumed double tap goes home (App.tsx).
  // In the native shell, one tick above the first control brings its Back (ADR-0014).
  useAppInput((ev) => {
    // A component has the wheel (ADR-0038): it gets the wheel, the clicks and the button.
    const f = focus.current;
    if (f?.captured()) {
      if (ev.type === "wheel") return (f.move(ev.delta), true);
      if (ev.type === "gesture" && ev.kind === "tap") return f.activate();
      if (ev.type === "gesture" && ev.kind === "double_tap") return f.cancel();
      if (ev.type === "button") return f.button(ev.down);
    }
    if (ev.type === "wheel") {
      if (ev.delta < 0 && native && (focus.current?.atTop() ?? true)) {
        native.overscroll();
        return true;
      }
      focus.current?.move(ev.delta);
      return true;
    }
    if (ev.type === "gesture" && ev.kind === "tap") return focus.current?.activate() ?? false;
    return false;
  });

  if (failed) return <div className="center error">{failed}</div>;
  if (!compiled) return <div className="center muted">Opening…</div>;
  return (
    <div className="enee-host" ref={box}>
      <AppBoundary name={compiled.app.name} onCrash={(e) => reportError(id, new EneeError("E_RENDER", e))}>
        <EneeApp
          compiled={compiled}
          host={host}
          onNavigate={() => focus.current?.reset()}
          onError={(e) => reportError(id, e)}
        />
      </AppBoundary>
    </div>
  );
}

/** Keeps a crashing app from taking the shell down with it. */
class AppBoundary extends Component<
  { name: string; onCrash: (message: string) => void; children: ReactNode },
  { error: string | null }
> {
  state = { error: null as string | null };
  static getDerivedStateFromError(e: unknown) {
    return { error: e instanceof Error ? e.message : String(e) };
  }
  componentDidCatch(e: unknown) {
    this.props.onCrash(e instanceof Error ? e.message : String(e));
  }
  render() {
    return this.state.error ? (
      <div className="center col">
        <div className="error">{this.props.name} crashed</div>
        <div className="muted small">{this.state.error}</div>
      </div>
    ) : (
      this.props.children
    );
  }
}
