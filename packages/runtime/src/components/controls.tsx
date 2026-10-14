import { useBoundProp } from "@json-render/react";
import { useId } from "react";

import { settle, useRuntime } from "../context";
import { eneeId } from "../spec";
import { Icon } from "./icons";
import type { Ctx } from "./types";
import { WheelControl } from "./wheel";

/** A choice among more options than this is a reel turned with the wheel, not a row of buttons. */
const REEL_OPTIONS = 4;

export function Button({ props, emit }: Ctx<"Button">) {
  const { reportError } = useRuntime();
  return (
    <button
      type="button"
      className="enee-button"
      data-variant={props.variant ?? "secondary"}
      onClick={() => settle(emit("press"), reportError)}
      {...eneeId(props)}
    >
      {props.icon ? <Icon name={props.icon} size={15} /> : null}
      <span>{props.label}</span>
    </button>
  );
}

type FieldValue = string | number | null;

function toInputText(v: unknown): string {
  if (v === null || v === undefined) return "";
  return typeof v === "number" ? (Number.isFinite(v) ? String(v) : "") : String(v);
}

export function Input({ props, bindings }: Ctx<"Input">) {
  const { requestFocus } = useRuntime();
  const [value, setValue] = useBoundProp<FieldValue>(props.value ?? null, bindings?.value);
  const id = useId();
  const type = props.type ?? "text";

  return (
    <div className="enee-field" {...eneeId(props)}>
      <label className="enee-field__label" htmlFor={id} id={`${id}-label`}>
        {props.label}
      </label>
      {type === "select" && (props.options ?? []).length > REEL_OPTIONS ? (
        <WheelControl label={props.label} value={value ?? null} onChange={setValue} spec={{ options: props.options }} labelled={false} />
      ) : type === "select" ? (
        <div id={id} className="enee-choices" role="radiogroup" aria-labelledby={`${id}-label`}>
          {(props.options ?? []).map((option) => (
            <button
              key={option}
              type="button"
              role="radio"
              aria-checked={value === option}
              className="enee-choice"
              data-selected={value === option ? "" : undefined}
              onClick={() => setValue(option)}
            >
              {option}
            </button>
          ))}
        </div>
      ) : type === "number" ? (
        // Turned with the wheel, not typed (r1os ADR-0025, ADR-0038).
        <WheelControl
          label={props.label}
          value={typeof value === "number" ? value : value === null ? null : Number(value) || 0}
          onChange={setValue}
          spec={{ min: props.min, max: props.max, step: props.step }}
          labelled={false}
        />
      ) : (
        <input
          id={id}
          className="enee-input"
          type={type}
          value={toInputText(value)}
          placeholder={props.placeholder}
          spellCheck={false}
          autoComplete="off"
          onPointerDown={requestFocus}
          onChange={(e) => setValue(e.currentTarget.value)}
        />
      )}
    </div>
  );
}

export function Toggle({ props, bindings }: Ctx<"Toggle">) {
  const [value, setValue] = useBoundProp<boolean | null>(props.value ?? null, bindings?.value);
  const on = value === true;
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      className="enee-toggle"
      onClick={() => setValue(!on)}
      {...eneeId(props)}
    >
      <span>{props.label}</span>
      <span className="enee-switch" aria-hidden />
    </button>
  );
}
