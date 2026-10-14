import { defineRegistry } from "@json-render/react";
import { Component, type ContextType, type ReactNode } from "react";

import { catalog } from "./catalog";
import { Card, EmptyState, Gauge, Heading, Progress, Row, Stack, Stat, Text, Timer } from "./components/basic";
import { BarChart, CalendarHeatmap, DonutChart, LineChart } from "./components/charts";
import { Button, Input, Toggle } from "./components/controls";
import { Game } from "./components/game";
import { LogicView, registryRef } from "./components/logic-view";
import { List } from "./components/list";
import { Wheel } from "./components/wheel";
import { RuntimeContext } from "./context";
import { ENEE_ID_PROP } from "./spec";
import { EneeError } from "./types";

/**
 * Per-element error boundary. json-render's own boundary only logs and drops the element; this one also reports
 * the failure with the element id (for "Fix with AI") and leaves a visible placeholder.
 */
class Guard extends Component<{ type: string; props: object; children: ReactNode }, { failed: boolean }> {
  static override contextType = RuntimeContext;
  declare context: ContextType<typeof RuntimeContext>;
  override state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  private get id(): string | undefined {
    const id = (this.props.props as Record<string, unknown>)[ENEE_ID_PROP];
    return typeof id === "string" ? id : undefined;
  }

  override componentDidCatch(error: unknown) {
    const message = error instanceof Error ? error.message : String(error);
    const id = this.id;
    this.context.reportError(
      new EneeError("E_RENDER", `${this.props.type} failed to render: ${message}`, {
        cause: error,
        ...(id ? { element: id } : {}),
      }),
    );
  }

  override render() {
    if (!this.state.failed) return this.props.children;
    return (
      <div className="enee-unknown" data-enee-id={this.id}>
        {this.props.type} couldn’t render
      </div>
    );
  }
}

// defineRegistry invokes these as plain functions inside its own wrapper component, so each entry renders a real
// component element (own hooks, own React identity) inside a Guard.
export const { registry } = defineRegistry(catalog, {
  components: {
    Stack: (ctx) => <Guard type="Stack" props={ctx.props}><Stack {...ctx} /></Guard>,
    Row: (ctx) => <Guard type="Row" props={ctx.props}><Row {...ctx} /></Guard>,
    Card: (ctx) => <Guard type="Card" props={ctx.props}><Card {...ctx} /></Guard>,
    Heading: (ctx) => <Guard type="Heading" props={ctx.props}><Heading {...ctx} /></Guard>,
    Text: (ctx) => <Guard type="Text" props={ctx.props}><Text {...ctx} /></Guard>,
    Stat: (ctx) => <Guard type="Stat" props={ctx.props}><Stat {...ctx} /></Guard>,
    Timer: (ctx) => <Guard type="Timer" props={ctx.props}><Timer {...ctx} /></Guard>,
    Gauge: (ctx) => <Guard type="Gauge" props={ctx.props}><Gauge {...ctx} /></Guard>,
    Progress: (ctx) => <Guard type="Progress" props={ctx.props}><Progress {...ctx} /></Guard>,
    Button: (ctx) => <Guard type="Button" props={ctx.props}><Button {...ctx} /></Guard>,
    Input: (ctx) => <Guard type="Input" props={ctx.props}><Input {...ctx} /></Guard>,
    Toggle: (ctx) => <Guard type="Toggle" props={ctx.props}><Toggle {...ctx} /></Guard>,
    Wheel: (ctx) => <Guard type="Wheel" props={ctx.props}><Wheel {...ctx} /></Guard>,
    Game: (ctx) => <Guard type="Game" props={ctx.props}><Game {...ctx} /></Guard>,
    LogicView: (ctx) => <Guard type="LogicView" props={ctx.props}><LogicView {...ctx} /></Guard>,
    BarChart: (ctx) => <Guard type="BarChart" props={ctx.props}><BarChart {...ctx} /></Guard>,
    LineChart: (ctx) => <Guard type="LineChart" props={ctx.props}><LineChart {...ctx} /></Guard>,
    DonutChart: (ctx) => <Guard type="DonutChart" props={ctx.props}><DonutChart {...ctx} /></Guard>,
    CalendarHeatmap: (ctx) => <Guard type="CalendarHeatmap" props={ctx.props}><CalendarHeatmap {...ctx} /></Guard>,
    List: (ctx) => <Guard type="List" props={ctx.props}><List {...ctx} /></Guard>,
    EmptyState: (ctx) => <Guard type="EmptyState" props={ctx.props}><EmptyState {...ctx} /></Guard>,
  },
  actions: {
    // `run` needs per-app state (host, store, runner), so <EneeApp> registers the real handler on its own
    // ActionProvider. defineRegistry requires an entry because the catalog declares the action.
    run: async () => {
      throw new Error("`run` is dispatched by <EneeApp>, not by the static registry");
    },
  },
});

// The logic lane's views render with this registry too (circular otherwise).
registryRef.current = registry;
