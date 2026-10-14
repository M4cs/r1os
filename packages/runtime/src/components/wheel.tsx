// A value you turn with the R1's wheel (r1os ADR-0038): a number between two ends, or one of a few options.
//
// At rest it's a row, its label and its value. A click takes the wheel: the row opens into a reel, with the values
// either side of this one above and below it, and each detent turns it (a quick spin moves five at a time over a long
// range). The bound state changes as it turns, so whatever reads it (a query, a stat) follows live. A click keeps
// the value; a double click puts back the one it had. By touch: a tap takes the wheel, a tap on a neighbour steps
// to it, and a tap on the value keeps it.

import { useBoundProp } from "@json-render/react";
import { useRef, useState } from "react";

import { eneeId } from "../spec";
import { ENGAGED_ATTR, useWheelInput, WHEEL_ATTR } from "../wheel-input";
import type { Ctx } from "./types";
import { neighbours, show, stride, turn, type WheelSpec, type WheelValue } from "./wheel-math";

export function Wheel({ props, bindings }: Ctx<"Wheel">) {
  const [value, setValue] = useBoundProp<WheelValue>(props.value ?? null, bindings?.value);
  const spec: WheelSpec = { min: props.min, max: props.max, step: props.step, options: props.options, wrap: props.wrap };
  return <WheelControl label={props.label} unit={props.unit} value={value ?? null} onChange={setValue} spec={spec} idProps={eneeId(props)} />;
}

export function WheelControl({
  label,
  unit,
  value,
  onChange,
  spec,
  idProps,
  labelled = true,
}: {
  label: string;
  unit?: string | undefined;
  value: WheelValue;
  onChange: (v: WheelValue) => void;
  spec: WheelSpec;
  idProps?: Record<string, unknown>;
  /** Draws its own label (a field's label is drawn by the field). */
  labelled?: boolean;
}) {
  const [engaged, setEngaged] = useState(false);
  const [dir, setDir] = useState(0);
  const ref = useRef<HTMLDivElement>(null);
  const before = useRef<WheelValue>(value);
  const last = useRef(0);
  // The latest value for event handlers between renders (a quick spin outruns React).
  const now = useRef(value);
  now.current = value;

  const step = (delta: number) => {
    const t = performance.now();
    const by = stride(spec, t - last.current);
    last.current = t;
    const next = turn(now.current, delta, spec, by);
    if (next === now.current) return;
    now.current = next;
    setDir(delta > 0 ? 1 : -1);
    onChange(next);
  };
  const take = () => {
    before.current = now.current;
    setEngaged(true);
  };
  const keep = () => setEngaged(false);
  const putBack = () => {
    onChange(before.current);
    setEngaged(false);
  };

  useWheelInput(ref, { wheel: step, press: keep, cancel: putBack, release: keep });

  const [prev, next] = neighbours(value, spec);
  const valueText = `${show(value, spec)}${unit ? ` ${unit}` : ""}`;
  return (
    <div
      ref={ref}
      role="button"
      tabIndex={0}
      className="enee-wheel"
      aria-label={label}
      aria-valuetext={valueText}
      {...{ [WHEEL_ATTR]: "" }}
      {...(engaged ? { [ENGAGED_ATTR]: "" } : {})}
      onClick={() => (engaged ? keep() : take())}
      onKeyDown={(e) => {
        if (!engaged) return;
        if (e.key === "ArrowUp" || e.key === "ArrowDown") {
          e.preventDefault();
          step(e.key === "ArrowDown" ? 1 : -1);
        } else if (e.key === "Escape") {
          putBack();
        }
      }}
      onWheel={(e) => {
        if (!engaged) return;
        e.preventDefault();
        step(e.deltaY > 0 ? 1 : -1);
      }}
      {...idProps}
    >
      {labelled ? <span className="enee-wheel__label">{label}</span> : null}
      <span className="enee-wheel__reel">
        {engaged ? (
          <button
            type="button"
            className="enee-wheel__near"
            tabIndex={-1}
            aria-hidden
            data-empty={prev === null ? "" : undefined}
            onClick={(e) => {
              e.stopPropagation();
              step(-1);
            }}
          >
            {prev === null ? " " : show(prev, spec)}
          </button>
        ) : null}
        <span key={`${show(value, spec)}`} className="enee-wheel__value" data-dir={dir}>
          {show(value, spec)}
          {unit ? <small className="enee-wheel__unit">{unit}</small> : null}
        </span>
        {engaged ? (
          <button
            type="button"
            className="enee-wheel__near"
            tabIndex={-1}
            aria-hidden
            data-empty={next === null ? "" : undefined}
            onClick={(e) => {
              e.stopPropagation();
              step(1);
            }}
          >
            {next === null ? " " : show(next, spec)}
          </button>
        ) : null}
      </span>
    </div>
  );
}
