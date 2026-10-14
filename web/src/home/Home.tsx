// The home page: the hero, then one idea a section, few words each. Agents using your apps come first, then the
// agents and the apps they build, the whole OS, dock mode, speed, and what keeps it yours.

import { Hero } from "../hero/Hero";
import { MeetAgents } from "./Agents";
import { AppControl } from "./Android";
import { AskForApps } from "./Apps";
import { CompanionSection } from "./Companion";
import { DockMode } from "./Dock";
import { Everything } from "./Everything";
import { Speed } from "./Speed";
import { OpenSource, Private } from "./Under";
import "../styles/home.css";
import "../styles/sections.css";

export function Home() {
  return (
    <>
      <Hero />
      <AppControl />
      <MeetAgents />
      <AskForApps />
      <Everything />
      <DockMode />
      <Speed />
      <Private />
      <CompanionSection />
      <OpenSource />
    </>
  );
}
