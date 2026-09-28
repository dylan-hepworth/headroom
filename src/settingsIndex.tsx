// Everything the search can find: each setting, which pane it's in, its tile, and the other words someone might use
// for it. A result is found on screen by its title (a row's, or a section's heading), so these titles match the panes.

import type { ReactNode } from "react";
import {
  Bell,
  Calendar,
  Chat,
  Check,
  Clock,
  Download,
  Gauge,
  Hook,
  Hourglass,
  Info,
  Key,
  Layers,
  Pace,
  MenuBar,
  Mic,
  Power,
  Refresh,
  Shield,
  Sparkle,
  Terminal,
} from "./icons";
import { score, type Hit } from "./searchMatch";

export type PaneId = "general" | "usage" | "alerts" | "sessions" | "hooks" | "recap" | "menu_bar";

type Setting = {
  pane: PaneId;
  title: string;
  tile: [string, ReactNode];
  detail?: string;
  /** The section it's under, for the result's second line. */
  section?: string;
  /** Other words it goes by. */
  words?: string;
  /** Where it is on screen, when that isn't a row with this title: a section's heading, or the top of the pane. */
  at?: { section: string } | "hero";
};

const icon = (Icon: (p: { size?: number }) => ReactNode) => <Icon size={13} />;

export const SETTINGS: Setting[] = [
  {
    pane: "general",
    title: "Weekly limit",
    tile: ["clay", icon(Gauge)],
    words: "usage ring percent 5-hour five hour fable model limits reset pace",
    at: "hero",
  },
  { pane: "general", title: "Refresh now", tile: ["green", icon(Refresh)], words: "check reload update usage", at: "hero" },
  { pane: "general", title: "Open usage page", tile: ["blue", icon(Gauge)], words: "claude.ai settings website", at: "hero" },
  {
    pane: "general",
    title: "Sign in",
    tile: ["yellow", icon(Key)],
    words: "token claude code login setup-token keychain account",
    at: { section: "Sign In" },
  },
  {
    pane: "general",
    title: "Open at login",
    tile: ["gray", icon(Power)],
    section: "General",
    detail: "Waits in the menu bar.",
    words: "launch startup start boot",
  },
  {
    pane: "general",
    title: "Check usage every",
    tile: ["green", icon(Clock)],
    section: "General",
    words: "interval frequency often minutes seconds poll refresh",
  },
  {
    pane: "menu_bar",
    title: "Pace arrows",
    tile: ["orange", icon(Pace)],
    words: "ahead behind faster slower up down indicator on track rings",
  },
  { pane: "general", title: "Version", tile: ["gray", icon(Info)], section: "About", words: "about" },
  { pane: "general", title: "Updates", tile: ["blue", icon(Download)], section: "About", words: "install upgrade new version release" },
  { pane: "general", title: "Check for updates automatically", tile: ["green", icon(Refresh)], section: "About", words: "auto update daily" },

  {
    pane: "usage",
    title: "Limits",
    tile: ["clay", icon(Gauge)],
    words: "5-hour weekly opus sonnet fable pace forecast reset",
    at: { section: "Limits" },
  },
  {
    pane: "usage",
    title: "Claude Code this window",
    tile: ["clay", icon(Gauge)],
    words: "projects share usage by project tokens cost",
    at: { section: "Claude Code This Window" },
  },

  {
    pane: "alerts",
    title: "5-hour limit",
    tile: ["clay", icon(Gauge)],
    section: "Usage",
    detail: "Once per window, when usage passes this.",
    words: "alert notification threshold percent five hour",
  },
  {
    pane: "alerts",
    title: "Weekly limit",
    tile: ["clay", icon(Gauge)],
    section: "Usage",
    detail: "Once per week, when usage passes this.",
    words: "alert notification threshold percent",
  },
  {
    pane: "alerts",
    title: "When a limit resets",
    tile: ["green", icon(Refresh)],
    section: "Usage",
    words: "alert notification resume rate limited stopped",
  },
  {
    pane: "alerts",
    title: "A session needs an answer",
    tile: ["yellow", icon(Chat)],
    section: "Sessions",
    words: "alert notification permission question waiting",
  },
  { pane: "alerts", title: "A session finishes", tile: ["blue", icon(Check)], section: "Sessions", words: "alert notification done finished reply" },
  { pane: "alerts", title: "Only when I'm not looking at it", tile: ["blue", icon(Check)], section: "Sessions", words: "away front focused app" },
  { pane: "alerts", title: "Only after runs of at least", tile: ["blue", icon(Check)], section: "Sessions", words: "minimum duration long runs" },
  {
    pane: "alerts",
    title: "Context is filling up",
    tile: ["purple", icon(Layers)],
    section: "Conversations",
    words: "alert notification context window tokens compact compaction full conversation 1m 200k",
  },
  {
    pane: "alerts",
    title: "Keep alerts on screen",
    tile: ["gray", icon(Bell)],
    words: "persistent banner sticky dismiss stay notification style temporary",
  },
  { pane: "alerts", title: "Test alert", tile: ["red", icon(Bell)], words: "send test notification permission" },

  {
    pane: "sessions",
    title: "Claude Code sessions",
    tile: ["orange", icon(Terminal)],
    words: "working waiting permission question allow deny needs you chats",
    at: { section: "Needs You" },
  },

  { pane: "hooks", title: "Hooks", tile: ["purple", icon(Hook)], words: "claude code settings.json install" },
  {
    pane: "hooks",
    title: "Approve from the menu bar",
    tile: ["green", icon(Shield)],
    section: "Approvals",
    words: "approvals allow deny permission popover panel answer",
  },
  {
    pane: "hooks",
    title: "If I don't answer, ask in the terminal after",
    tile: ["green", icon(Shield)],
    section: "Approvals",
    words: "timeout hold countdown minutes",
  },
  {
    pane: "hooks",
    title: "Open automatically when a session needs an answer",
    tile: ["green", icon(Shield)],
    section: "Approvals",
    words: "popover auto panel",
  },
  {
    pane: "general",
    title: "Talk to a chat",
    tile: ["red", icon(Mic)],
    section: "General",
    words: "voice speech dictation dictate microphone mic talk speak shortcut hotkey keyboard",
  },
  {
    pane: "hooks",
    title: "Compact cards",
    tile: ["green", icon(Shield)],
    section: "Approvals",
    words: "compact small slim smaller short minimal card panel popover distracting",
  },
  {
    pane: "hooks",
    title: "Have Claude ask what's next",
    tile: ["green", icon(Shield)],
    section: "Approvals",
    words: "question next step keep going end of turn done finish follow up",
  },
  {
    pane: "hooks",
    title: "Hands-free",
    tile: ["green", icon(Shield)],
    section: "Approvals",
    words:
      "reply summary read review follow along away from chat without claude code last message prompt click list pending waiting working interject message send now stop",
  },
  {
    pane: "hooks",
    title: "Add a note when I'm near a limit",
    tile: ["clay", icon(Chat)],
    section: "Tell Claude About Your Limits",
    words: "note warn message context",
  },
  { pane: "hooks", title: "Once usage passes", tile: ["clay", icon(Chat)], section: "Tell Claude About Your Limits", words: "threshold percent" },
  {
    pane: "hooks",
    title: "Send it",
    tile: ["clay", icon(Chat)],
    section: "Tell Claude About Your Limits",
    words: "right away next message ask claude to instructions",
  },

  { pane: "recap", title: "Daily recap", tile: ["teal", icon(Calendar)], words: "summary notification end of day report" },
  { pane: "recap", title: "Send it at", tile: ["teal", icon(Calendar)], words: "time evening" },
  { pane: "recap", title: "Today so far", tile: ["teal", icon(Calendar)], words: "today summary", at: { section: "Today So Far" } },

  { pane: "menu_bar", title: "Shows", tile: ["blue", icon(MenuBar)], words: "title numbers percent rings three rings fable 5-hour weekly menu bar" },
  { pane: "menu_bar", title: "Usage ring", tile: ["clay", icon(Sparkle)], words: "icon progress circle" },
  { pane: "menu_bar", title: "Sessions waiting on you", tile: ["orange", icon(Hourglass)], words: "dots count yellow blue" },
];

export type Result = {
  setting: Setting;
  /** Where the query matched the title, to show in bold. */
  hits: Hit[];
  score: number;
};

/** The settings that match a query, best first. */
export function searchSettings(query: string, limit = 12): Result[] {
  if (!query.trim()) return [];
  const results: Result[] = [];
  for (const s of SETTINGS) {
    const found = score(query, [
      { text: s.title, weight: 1, shown: true },
      { text: s.words ?? "", weight: 0.85 },
      { text: s.section ?? "", weight: 0.6 },
      { text: s.detail ?? "", weight: 0.45 },
    ]);
    // A slip that only lands in a description isn't worth showing
    if (found.score >= 30) results.push({ setting: s, hits: found.hits, score: found.score });
  }
  return results.sort((a, b) => b.score - a.score || a.setting.title.length - b.setting.title.length).slice(0, limit);
}

/** Scroll a result into view and outline it for a moment. */
export function reveal(s: Setting) {
  const el =
    s.at === "hero"
      ? document.querySelector<HTMLElement>(".hero")
      : s.at
        ? [...document.querySelectorAll<HTMLElement>(".section-title")]
            .find((h) => h.textContent === (s.at as { section: string }).section)
            ?.closest("section")
        : [...document.querySelectorAll<HTMLElement>(".row .title")]
            .find((t) => t.textContent?.trim().toLowerCase().startsWith(s.title.toLowerCase()))
            ?.closest(".row");
  if (!(el instanceof HTMLElement)) return;
  el.scrollIntoView({ block: "center", behavior: "smooth" });
  // Reading the layout in between restarts the outline if it's still showing from the last time
  el.classList.remove("found");
  void el.offsetWidth;
  el.classList.add("found");
  window.setTimeout(() => el.classList.remove("found"), 2400);
}
