// Push-to-talk (ADR-0009, docs/specs/experience.md §2.5): hold the side button and talk.
//
// r1d does the listening (crate::voice) and streams `voice` events: listening → partial… → final
// (or cancelled / error), plus `voice_level` for the animation. This file draws them: the agent,
// breathing with your voice, and your words as they're recognised. App.tsx routes the final text to
// a chat.
//
// In the simulator the page is the mic: r1d asks over /api/mic, and your PC's microphone answers.

import { useEffect, useRef } from "react";
import type { Agent } from "./api";
import { Character, colorOf } from "./ui/Character";

/** `released`: the button is up and r1d is catching the tail of what you said. */
export type VoiceUi = { phase: "listening" | "sending" | "error"; text: string; level: number; released?: boolean };

const BARS = [0.55, 0.8, 1, 0.8, 0.55];

export function VoiceOverlay({ v, agent }: { v: VoiceUi; agent: Agent }) {
  const words = useRef<HTMLDivElement>(null);
  // Long utterances: keep the newest words in view.
  useEffect(() => {
    words.current?.scrollTo({ top: words.current.scrollHeight });
  }, [v.text]);
  const lvl = v.phase === "listening" ? v.level : 0;
  const size = v.text.length > 160 ? "s" : v.text.length > 70 ? "m" : "l";
  return (
    <div className={`ptt ptt-${v.phase}`} style={{ ["--accent" as string]: colorOf(agent.color), ["--lvl" as string]: lvl }}>
      <div className="ptt-agent">
        <div className="ptt-ring" />
        <Character form={agent.form} color={agent.color} finish={agent.finish} eyes={agent.eyes} wear={agent.wear} state={v.phase === "error" ? "error" : "listening"} size={92} />
      </div>
      <div className={`ptt-words ptt-${size}`} ref={words}>
        <span className={v.text ? "ptt-text" : "ptt-text ptt-placeholder"}>{v.text || "Listening…"}</span>
      </div>
      <div className="ptt-foot">
        {v.phase === "listening" ? (
          <>
            <div className="ptt-bars" aria-hidden>
              {BARS.map((k, i) => (
                <span key={i} style={{ transform: `scaleY(${0.15 + Math.min(1, lvl * k * 1.3)})` }} />
              ))}
            </div>
            <div className="ptt-hint">{v.released ? "Finishing…" : "Let go to send · turn the wheel to cancel"}</div>
          </>
        ) : v.phase === "sending" ? (
          <div className="ptt-hint">Sent to {agent.name}</div>
        ) : null}
      </div>
    </div>
  );
}

// ---- the simulator's mic ------------------------------------------------------------------------

const WORKLET = `
class Tap extends AudioWorkletProcessor {
  process(inputs) {
    const ch = inputs[0] && inputs[0][0];
    if (ch) this.port.postMessage(ch.slice(0));
    return true;
  }
}
registerProcessor("r1os-tap", Tap);
`;

/** Answers r1d's mic requests with the PC's microphone (16 kHz mono s16le over /api/mic). */
export function useSimMic(enabled: boolean) {
  useEffect(() => {
    if (!enabled || !navigator.mediaDevices?.getUserMedia) return;
    let ws: WebSocket | null = null;
    let ctx: AudioContext | null = null;
    let live = false;
    let closed = false;
    let pending: number[] = [];
    let retry: number | undefined;

    const ensureAudio = async () => {
      if (ctx) return ctx;
      const stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
      const c = new AudioContext();
      await c.audioWorklet.addModule(URL.createObjectURL(new Blob([WORKLET], { type: "text/javascript" })));
      const node = new AudioWorkletNode(c, "r1os-tap");
      c.createMediaStreamSource(stream).connect(node);
      const ratio = c.sampleRate / 16000;
      let acc = 0;
      let n = 0;
      let phase = 0;
      node.port.onmessage = (e: MessageEvent<Float32Array>) => {
        if (!live) return;
        // Box-filter decimation to 16 kHz: good enough for speech.
        for (const s of e.data) {
          acc += s;
          n++;
          phase += 1;
          if (phase >= ratio) {
            phase -= ratio;
            pending.push(acc / n);
            acc = 0;
            n = 0;
          }
        }
        if (pending.length >= 1600 && ws?.readyState === WebSocket.OPEN) {
          const pcm = new Int16Array(pending.length);
          pending.forEach((s, i) => (pcm[i] = Math.max(-32768, Math.min(32767, Math.round(s * 32767)))));
          ws.send(pcm.buffer);
          pending = [];
        }
      };
      ctx = c;
      return c;
    };

    const connect = () => {
      if (closed) return;
      ws = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/api/mic`);
      ws.binaryType = "arraybuffer";
      ws.onmessage = (e) => {
        if (e.data === "start") {
          pending = [];
          live = true;
          void ensureAudio().then((c) => c.resume(), (err) => console.warn("sim mic:", err));
        } else if (e.data === "stop") {
          live = false;
          void ctx?.suspend();
        }
      };
      ws.onclose = () => {
        live = false;
        retry = window.setTimeout(connect, 1500);
      };
    };
    connect();
    return () => {
      closed = true;
      window.clearTimeout(retry);
      ws?.close();
      void ctx?.close();
    };
  }, [enabled]);
}
