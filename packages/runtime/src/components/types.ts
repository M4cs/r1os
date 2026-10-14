import type { InferComponentProps } from "@json-render/core";
import type { BaseComponentProps } from "@json-render/react";

import type { EneeCatalog } from "../catalog";

export type ComponentName = keyof EneeCatalog["data"]["components"];

/** Render context for catalog component `K` (resolved props + json-render's emit/on/bindings). */
export type Ctx<K extends ComponentName> = BaseComponentProps<InferComponentProps<EneeCatalog, K>>;
