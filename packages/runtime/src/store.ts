// Host-controlled StateStore (contract §2) for json-render's controlled mode (`<JSONUIProvider store>`).
//
//   /q/<query>       rows, written by the host (query refresh)
//   /settings/<key>  setting values, written by the host
//   /state/<key>     ephemeral UI state, written by the runtime ($bindState, `set` steps)
//   /services/<name> live service values (host; not in v0)
//
// json-render writes through `set`/`update` ($bindState inputs, built-in setState/pushState actions). Those writes are
// confined to `/state`: anything else is rejected and reported, so a spec can't overwrite query results or settings.
// The host side uses `hostUpdate`, which may write anywhere and batches several paths into one notification
// (one React render per query refresh, not one per query).

import { createStateStore, type StateModel, type StateStore } from "@json-render/core";

export interface EneeStore extends StateStore {
  /** Host/runtime write: any path, all paths applied atomically with a single notification. */
  hostUpdate(updates: Record<string, unknown>): void;
}

export interface EneeStoreOptions {
  /** Called when json-render tries to write outside `/state`. */
  onRejectedWrite?: (path: string) => void;
}

export function isUiWritablePath(path: string): boolean {
  return path === "/state" || path.startsWith("/state/");
}

export function createEneeStore(initial: StateModel, options: EneeStoreOptions = {}): EneeStore {
  const inner = createStateStore(initial);
  const reject = (path: string) => {
    options.onRejectedWrite?.(path);
    return false;
  };

  return {
    get: inner.get,
    getSnapshot: inner.getSnapshot,
    getServerSnapshot: inner.getServerSnapshot ?? inner.getSnapshot,
    subscribe: inner.subscribe,
    set(path, value) {
      if (!isUiWritablePath(path)) {
        reject(path);
        return;
      }
      inner.set(path, value);
    },
    update(updates) {
      const allowed: Record<string, unknown> = {};
      for (const [path, value] of Object.entries(updates)) {
        if (isUiWritablePath(path)) allowed[path] = value;
        else reject(path);
      }
      inner.update(allowed);
    },
    hostUpdate(updates) {
      inner.update(updates);
    },
  };
}
