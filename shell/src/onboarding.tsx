// First run (docs/specs/experience.md §2.4): hello → Wi-Fi → make your agent → connect a brain → ready.
// Every step is saved in r1d, so a reboot resumes where you were.

import { useEffect, useState } from "react";
import { agentsApi, onboardingApi, type Agent, type OnboardingStep } from "./api";
import { useAppInput } from "./input";
import { AgentDesigner, BrainSetup, WifiSetup } from "./setup";
import { Character } from "./ui/Character";

const ORDER: OnboardingStep[] = ["hello", "wifi", "agent", "brain", "ready"];

export function Onboarding({ step, agent, onStep, onAgent }: { step: OnboardingStep; agent: Agent; onStep: (s: OnboardingStep) => void; onAgent: (a: Agent) => void }) {
  const go = (s: OnboardingStep) => {
    void onboardingApi.set(s).catch(() => {});
    onStep(s);
  };
  const next = () => go(ORDER[ORDER.indexOf(step) + 1] ?? "done");
  const at = ORDER.indexOf(step);
  return (
    <div className="onboarding">
      {step !== "hello" && (
        <div className="progress-dots">
          {ORDER.slice(1).map((s, i) => (
            <span key={s} className={i + 1 <= at ? "on" : ""} />
          ))}
        </div>
      )}
      <div className="onboarding-body" key={step}>
        {step === "hello" && <Hello agent={agent} onNext={next} />}
        {step === "wifi" && <WifiSetup onDone={next} />}
        {step === "agent" && (
          <AgentDesigner
            initial={agent}
            onSave={agentsApi.put}
            doneLabel="Continue"
            onDone={(a) => {
              onAgent(a);
              next();
            }}
          />
        )}
        {step === "brain" && <BrainSetup agent={agent} onDone={next} />}
        {step === "ready" && <Ready agent={agent} onNext={() => go("done")} />}
      </div>
    </div>
  );
}

function Hello({ agent, onNext }: { agent: Agent; onNext: () => void }) {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    const t = window.setTimeout(() => setShown(true), 400);
    return () => window.clearTimeout(t);
  }, []);
  useAppInput((ev) => {
    if (ev.type === "gesture" && ev.kind === "tap") {
      onNext();
      return true;
    }
    return ev.type === "wheel";
  });
  return (
    <div className="hello" onClick={onNext}>
      <div className={`hatch${shown ? " in" : ""}`}>
        <Character form={agent.form} color={agent.color} finish={agent.finish} eyes={agent.eyes} wear={agent.wear} state={shown ? "idle" : "sleeping"} size={190} />
      </div>
      <div className={`hello-text${shown ? " in" : ""}`}>
        <div className="hello-title">Hi.</div>
        <div className="muted">Let's set me up. It takes a minute.</div>
      </div>
      <div className={`hint${shown ? " in" : ""}`}>press the side button to start</div>
    </div>
  );
}

function Ready({ agent, onNext }: { agent: Agent; onNext: () => void }) {
  useAppInput((ev) => {
    if (ev.type === "gesture" && ev.kind === "tap") {
      onNext();
      return true;
    }
    return ev.type === "wheel";
  });
  return (
    <div className="hello" onClick={onNext}>
      <Character form={agent.form} color={agent.color} finish={agent.finish} eyes={agent.eyes} wear={agent.wear} state="speaking" size={170} />
      <div className="hello-text in">
        <div className="hello-title">I'm {agent.name}.</div>
        <div className="muted">Hold the button to talk. Ask for anything, or ask me to make you an app.</div>
        <div className="muted small">Double-tap anywhere for your apps. Make more agents at the end of the wheel.</div>
      </div>
      <div className="hint in">press to start</div>
    </div>
  );
}
