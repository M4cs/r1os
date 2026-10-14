// The hero's story: what your agents do, each with the R1 screen that shows it, its accent (one of the R1's), how long
// it stays, and the gesture or words behind it. Agents using your apps come first; for dock mode the R1 on the page
// turns onto its side (Hero.tsx).

import type { ReactElement } from "react";
import { ACCENTS } from "@companion/theme";
import { APP_WINDOW_MS, AppWindowScreen } from "../device/screens/AppWindow";
import { BuildScreen } from "../device/screens/Build";
import { AskScreen, CardScreen, PcScreen } from "../device/screens/Chats";
import { DOCK_STORY_MS, DockScreen } from "../device/screens/Dock";
import { HomeScreen } from "../device/screens/Home";
import { TalkScreen } from "../device/screens/Talk";
import { TranslatorScreen } from "../device/screens/Tools";

export interface Feature {
  id: string;
  /** Finishes "your agents,". */
  phrase: string;
  accent: string;
  ms: number;
  /** What you do: a gesture, or what you say. */
  how: string;
  screen: (accent: string) => ReactElement;
}

const a = ACCENTS as Record<string, string>;

export const FEATURES: Feature[] = [
  { id: "android", phrase: "use your apps.", accent: a.green!, ms: APP_WINDOW_MS, how: "“get me sesame noodles on forkful”", screen: (c) => <AppWindowScreen accent={c} /> },
  { id: "build", phrase: "build new apps.", accent: a.cyan!, ms: 5200, how: "“make me a water tracker”", screen: (c) => <BuildScreen accent={c} /> },
  { id: "wheel", phrase: "live on the wheel.", accent: a.orange!, ms: 3400, how: "turn the wheel", screen: (c) => <HomeScreen accent={c} /> },
  { id: "talk", phrase: "hear you out.", accent: a.pink!, ms: 4200, how: "hold the button and talk", screen: (c) => <TalkScreen accent={c} /> },
  { id: "cards", phrase: "answer in cards.", accent: a.blue!, ms: 3900, how: "“what's the weather this week?”", screen: (c) => <CardScreen accent={c} /> },
  { id: "ask", phrase: "ask each other.", accent: a.purple!, ms: 3900, how: "“ask nova what's in the news”", screen: (c) => <AskScreen accent={c} /> },
  { id: "pc", phrase: "work on your pc.", accent: a.lime!, ms: 4300, how: "click to allow", screen: (c) => <PcScreen accent={c} /> },
  { id: "dock", phrase: "sit on your desk.", accent: a.indigo!, ms: DOCK_STORY_MS, how: "set it on its side", screen: (c) => <DockScreen accent={c} /> },
  { id: "translate", phrase: "speak italian.", accent: a.red!, ms: 4600, how: "hold and talk, in either language", screen: (c) => <TranslatorScreen accent={c} /> },
];

/** The reel as one sentence, for a screen reader, made from the phrases so it can't drift from them. */
const said = FEATURES.map((f) => f.phrase.replace(/\.$/, ""));
export const SPOKEN = `${said.slice(0, -1).join(", ")}, and ${said[said.length - 1]}.`;
