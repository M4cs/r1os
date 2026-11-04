// Typed calls for the agents, onboarding, Wi-Fi, accounts, chats and memory (r1d;
// docs/specs/agents.md §6, docs/specs/experience.md §4).

import type { AgentLive } from "./r1";

export interface Agent {
  id: string;
  name: string;
  form: string;
  color: string;
  /** How the colour is rendered: soft, matte, grain, noise, gradient, grainy, aurora. */
  finish: string;
  /** Its eyes and what it wears (ADR-0043; Character's EYES, HATS, FACE_WEAR, NECK_WEAR). */
  eyes?: string;
  wear?: string[];
  personality: string;
  custom: string;
  /** What it's for, in a few words ("" = a bit of everything). */
  role: string;
  brain: "claude" | "codex" | "hermes";
  /** What it may change on the R1 (ADR-0045): look, device, apps → ask (unset), allow or never. */
  can?: Record<string, "ask" | "allow" | "never">;
}

/** An agent as `/api/agents` returns it: plus what it's doing now, and in which chat. */
export interface AgentInfo extends Agent {
  state: AgentLive;
  chat?: string;
  /** Replies in its chats the user hasn't seen. */
  unread: number;
  /** Its last tool while working (the shell tracks it from `chat` tool events). */
  tool?: string;
}

export type OnboardingStep = "hello" | "wifi" | "agent" | "brain" | "ready" | "done";

export interface WifiStatus {
  enabled: boolean;
  ssid?: string;
  ip?: string;
  rssi?: number;
}
export interface WifiNetwork {
  ssid: string;
  rssi: number;
  security: string;
  bars: number;
}

export interface Accounts {
  claude: boolean;
  codex: boolean;
  /** The owner's own Hermes gateway (ADR-0070), and where. Its key never comes back. */
  hermes: boolean;
  hermes_url?: string | null;
}

export interface Chat {
  id: string;
  agent: string;
  title: string;
  brain: string;
  session?: string;
  created: number;
  updated: number;
  last?: string;
  /** Tokens the chat's session holds (0 = not known). */
  context: number;
  /** Replies the user hasn't seen. */
  unread: number;
}
export interface Message {
  id: number;
  role: "user" | "assistant" | "error" | "build" | "note";
  text: string;
  at: number;
  meta?: Record<string, unknown>;
}

export interface Memory {
  id: number;
  text: string;
  at: number;
  /** Every agent sees it (a fact about the user). */
  shared: boolean;
  agent?: string;
  /** The agent that wrote it. */
  by?: string;
}

async function req<T>(method: string, path: string, body?: unknown): Promise<T> {
  const r = await fetch(path, {
    method,
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const json = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error((json as { error?: string }).error ?? `${path}: HTTP ${r.status}`);
  return json as T;
}

/**
 * `agent`'s current conversation: the chat you had with it most lately, however long ago (ADR-0057). Talking carries
 * it on; a new chat starts only when you start one. Undefined when it has none yet (= a new chat).
 */
export async function currentChat(agent: string): Promise<string | undefined> {
  const cs = await chatApi.list(agent).catch(() => []);
  return cs[0]?.id;
}

export const agentsApi = {
  list: () => req<{ agents: AgentInfo[]; focus: string; max: number }>("GET", "/api/agents"),
  create: (a: Omit<Agent, "id">) => req<{ agent: AgentInfo }>("POST", "/api/agents", a).then((r) => r.agent),
  put: (a: Agent) => req<{ agent: AgentInfo }>("PUT", `/api/agents/${a.id}`, a).then((r) => r.agent),
  remove: (id: string) => req("DELETE", `/api/agents/${id}`),
  order: (ids: string[]) => req("PUT", "/api/agents/order", { ids }),
};

export const onboardingApi = {
  get: () => req<{ step: OnboardingStep }>("GET", "/api/onboarding").then((r) => r.step),
  set: (step: OnboardingStep) => req("PUT", "/api/onboarding", { step }),
};

export const wifiApi = {
  get: () => req<{ status: WifiStatus; networks: WifiNetwork[] }>("GET", "/api/wifi"),
  scan: () => req("POST", "/api/wifi/scan", {}),
  connect: (ssid: string, security: string, password?: string) =>
    req("POST", "/api/wifi/connect", { ssid, security, password }),
};

export const accountsApi = {
  get: () => req<{ accounts: Accounts }>("GET", "/api/accounts").then((r) => r.accounts),
  codexLogin: () => req<{ url: string; code: string; expires_in: number }>("POST", "/api/accounts/codex/login", {}),
  claudeLogin: () => req<{ pair_url: string; auth_url: string }>("POST", "/api/accounts/claude/login", {}),
  logout: (brain: "claude" | "codex" | "hermes") => req("POST", `/api/accounts/${brain}/logout`, {}),
  qrUrl: (text: string) => `/api/qr?text=${encodeURIComponent(text)}`,
};

export const chatApi = {
  /** One agent's chats, newest first. */
  list: (agent: string) => req<{ chats: Chat[] }>("GET", `/api/chats?agent=${encodeURIComponent(agent)}`).then((r) => r.chats),
  create: (agent: string) => req<{ chat: Chat }>("POST", "/api/chats", { agent }).then((r) => r.chat),
  get: (id: string) => req<{ chat: Chat; messages: Message[]; busy: boolean }>("GET", `/api/chats/${id}`),
  /** `app`: the app on screen when the user spoke (the agent is told). */
  send: (id: string, text: string, app?: string) => req("POST", `/api/chats/${id}/messages`, { text, app }),
  /** Starts `agent`'s process for `chat` (or a spare for a new chat) while the user is still talking. */
  warm: (agent: string, chat?: string) => req("POST", "/api/chats/warm", { agent, chat }).catch(() => {}),
  remove: (id: string) => req("DELETE", `/api/chats/${id}`),
  /** The chat is on screen: its replies are seen. */
  seen: (id: string) => req("POST", `/api/chats/${id}/seen`, {}).catch(() => {}),
};

export const memoryApi = {
  /** The shared facts, plus `agent`'s own when given. */
  list: (agent?: string) =>
    req<{ memories: Memory[] }>("GET", `/api/memories${agent ? `?agent=${encodeURIComponent(agent)}` : ""}`).then((r) => r.memories),
  forget: (id: number) => req("DELETE", `/api/memories/${id}`),
};

export const PERSONALITIES = [
  { id: "friendly", label: "Friendly", hint: "Warm and encouraging" },
  { id: "concise", label: "Concise", hint: "Straight to the point" },
  { id: "playful", label: "Playful", hint: "Curious, a bit of wit" },
  { id: "calm", label: "Calm", hint: "Unhurried and steady" },
];

/** What an agent can be for (the designer's role step); "" = a bit of everything. */
export const ROLES = [
  { id: "", label: "A bit of everything" },
  { id: "apps and tools", label: "Apps and tools" },
  { id: "plans, lists and reminders", label: "Plans and lists" },
  { id: "research and the news", label: "Research and news" },
  { id: "health, fitness and habits", label: "Health and habits" },
  { id: "someone to talk to", label: "Someone to talk to" },
];

export const NAMES = ["Pip", "Nova", "Mochi", "Echo", "Juniper", "Bolt", "Kiwi", "Sol"];
