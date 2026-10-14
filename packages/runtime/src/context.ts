import { createContext, useContext } from "react";

import type { FormatOptions } from "./value";

/** Per-<EneeApp> services that catalog components need beyond json-render's own contexts. */
export interface RuntimeContextValue {
  format: FormatOptions;
  /** An input wants the keyboard (the overlay shell activates its window when it is only peeking). */
  requestFocus: () => void;
  /** Report a failure from a component (e.g. a rejected action promise). */
  reportError: (error: unknown) => void;
}

const noop = () => {};

export const RuntimeContext = createContext<RuntimeContextValue>({
  format: {},
  requestFocus: noop,
  reportError: noop,
});

export function useRuntime(): RuntimeContextValue {
  return useContext(RuntimeContext);
}

/** json-render rejects with this when the user cancels a binding-level `confirm`; that is not a failure. */
export function isCancellation(error: unknown): boolean {
  return error instanceof Error && error.message === "Action cancelled";
}

/** json-render's `emit` is typed `void` but returns the action promise; route rejections to the runtime. */
export function settle(result: unknown, reportError: (error: unknown) => void): void {
  if (result instanceof Promise) {
    result.catch((error: unknown) => {
      if (!isCancellation(error)) reportError(error);
    });
  }
}
