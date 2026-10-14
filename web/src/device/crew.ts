// The agents on the page: the names r1os suggests in its designer (host/…/shell/SetupPanels.kt) and the R1's own looks,
// never anyone's real agents.

export interface Member {
  name: string;
  form: string;
  color: string;
  finish: string;
  eyes?: string;
  wear?: string[];
  role: string;
  brain: "claude" | "codex";
}

export const CREW: Member[] = [
  { name: "pip", form: "orb", color: "orange", finish: "soft", role: "a bit of everything", brain: "claude" },
  { name: "kiwi", form: "bean", color: "green", finish: "grain", role: "health and habits", brain: "claude" },
  { name: "nova", form: "cloud", color: "purple", finish: "aurora", role: "research and news", brain: "codex" },
  { name: "mochi", form: "sprite", color: "cyan", finish: "grainy", role: "apps and tools", brain: "claude" },
  { name: "bolt", form: "bot", color: "lime", finish: "matte", eyes: "dots", wear: ["glasses"], role: "works on your pc", brain: "codex" },
  { name: "sol", form: "ghost", color: "blue", finish: "grain", eyes: "happy", role: "plans and lists", brain: "claude" },
  { name: "echo", form: "blob", color: "pink", finish: "soft", role: "someone to talk to", brain: "claude" },
  { name: "juniper", form: "cat", color: "teal", finish: "gradient", role: "plans and lists", brain: "codex" },
];

export const member = (name: string): Member => CREW.find((m) => m.name === name) ?? CREW[0]!;
