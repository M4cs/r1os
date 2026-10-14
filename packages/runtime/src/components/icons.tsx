// Icons for `icon` props and the runtime's own controls: Phosphor (bold weight reads best at UI sizes), loaded on
// demand through @r1os/icons. The compiler guarantees app icon names are Phosphor names.

import { Icon as PhosphorIcon } from "@r1os/icons";

/** Names used by the runtime itself before the switch to Phosphor. */
const LEGACY: Record<string, string> = { inbox: "tray", "chevron-left": "caret-left", "chevron-right": "caret-right" };

export function Icon({ name, size = 16 }: { name: string | undefined; size?: number }) {
  if (!name) return null;
  return <PhosphorIcon name={LEGACY[name] ?? name} weight="bold" size={size} className="enee-icon" />;
}
