// What the settings window knows and can change. Inside the app it calls the Rust side (the `#[tauri::command]`
// functions in main.rs, like `get_state` and `set_setting`). In a plain browser (`npm run ui`) it runs on the made-up
// data at the bottom, so the design can be worked on without building the app.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Pending } from "./Pending";
import type { ChatIcon } from "./Avatar";
import type { Plan } from "./Planner";
import type { Ask } from "./Popover";
import myCompany from "./my-company.json";

export const inApp = "__TAURI_INTERNALS__" in window;

/** A limit's key. Besides these, a model's own weekly limit that Claude reports comes as "7d_" and the model, like
 *  "7d_fable". */
export type LimitKey = "5h" | "7d" | "7d_opus" | "7d_sonnet" | (string & {});

export type Limit = {
  key: LimitKey;
  label: string;
  pct: number;
  /** When it resets, as shown: "3:10 PM", or "Tue 9:00 AM" when it isn't today. */
  resets?: string;
  /** The same, in milliseconds since 1970. */
  resetsAt?: number;
  /** How far through the window we are, 0 to 100. Usage ahead of this is running hot. */
  elapsed?: number;
  /** When usage at the current pace reaches 100%, if that's before the reset. */
  pace?: string;
  /** Its pace arrow: more than 5 points ahead of the window, or more than 5 behind. The menu bar's, worked out once. */
  paceArrow?: "ahead" | "behind" | null;
};

export type SessionState = "permission" | "question" | "waiting" | "working" | "limited" | "idle";

/** Sessions that need an answer from the user: a permission request, or a question. Yellow, like the Claude app. */
export const needsAnswer = (s: Session) => s.state === "permission" || s.state === "question";

/** Sessions that are done and waiting for the user's next message. Blue, like the Claude app. */
export const yourTurn = (s: Session) => s.state === "waiting";

export type Session = {
  id: string;
  project: string;
  /** Its chat's icon (see Avatar.tsx). */
  icon?: ChatIcon;
  /** The chat's title, as the Claude app shows it in its list of chats. */
  title?: string | null;
  path: string;
  state: SessionState;
  /** When it got to this state, in milliseconds since 1970. */
  since: number;
  /** What it's doing while working, e.g. "Running tests". */
  activity?: string;
  /** The request Headroom is holding for an answer, a permission request or a question, and whether "Allow for
   *  Session" applies (Claude Code suggested a rule for it). */
  held?: {
    id: string;
    canSession: boolean;
    kind: "permission" | "question";
    /** What it asks: the tool and its whole command or file, or for a question, the question. */
    tool: string;
    detail: string;
  } | null;
  /** What it's asking permission for, or the question it's asking. */
  request?: { tool: string; detail: string };
  /** The start of Claude's last message, once it's done and waiting on you. */
  last?: string;
};

export type Settings = {
  interval: number;
  launchAtLogin: boolean;
  autoUpdate: boolean;
  title: "both" | "5h" | "7d" | "5h_reset" | "ring" | "rings" | "rings_model";
  ring: boolean;
  waitingCount: boolean;
  alert5h: number | "off";
  alert7d: number | "off";
  /** Alert when a Claude Code conversation's context passes this share of its window. */
  contextAlert: number | "off";
  /** Leave alerts on screen until they're closed, rather than clearing them after a few seconds. */
  persistentAlerts: boolean;
  hooks: boolean;
  approvals: boolean;
  /** Does the popover open by itself when a session needs an answer? */
  popoverAuto: boolean;
  /** Does each request show in a few lines, rather than on the whole card? */
  compactCards: boolean;
  /** Does Claude end each turn by asking what's next, in the popover? */
  askNext: boolean;
  /** Hands-free: each question comes with what Claude said that turn, to follow along from the popover. */
  handsFree: boolean;
  /** Alerts and requests paused from the menu: until a time ("4:10 PM"), or until resumed. Null when they aren't. */
  paused: { until?: string } | null;
  /** An arrow for each limit, in the menu bar and in the rings up top, for whether it's ahead of or behind its window. */
  paceArrows: boolean;
  approvalTimeout: number;
  notifyWaiting: boolean;
  notifyDone: boolean;
  doneOnlyAway: boolean;
  doneMinRun: number;
  nearLimit: boolean;
  nearLimitAt: number;
  /** Also send the note to sessions that are working, after their next tool call. */
  nearLimitRightAway: boolean;
  /** The user's own words for what Claude should do. Empty for `nearLimitDefault`. */
  nearLimitMessage: string;
  nearLimitDefault: string;
  resumeAlert: boolean;
  recap: boolean;
  recapAt: string;
};

/** The desktop wallpaper, and the main screen it's on, for the menu bar preview. Sizes are in points. */
export type Wallpaper = { url: string | null; width: number; height: number; menuBar: number };

export type AppState = {
  version: string;
  /** The line under the usage in the menu: when we last checked, or why the last check failed. */
  status: string;
  /** Did the last check work? */
  ok: boolean;
  limits: Limit[];
  /** The numbers in the menu bar right now, e.g. "28% · 60%". Missing until the first good check. */
  menuTitle?: string | null;
  signIn: "token" | "claude_code";
  /** The version of an update that's ready to install, if there is one. */
  update: string | null;
  settings: Settings;
  sessions: Session[];
  /** The requests Headroom is holding for an answer, for the popover, oldest first. */
  asks: Ask[];
  /** Today's recap as the notification will read, or null on a day without Claude Code use. */
  recap: { title: string; body: string } | null;
  /** Each project's share of Claude Code use on this Mac in the current 5-hour window, in percent, biggest first. */
  shares: { project: string; pct: number }[];
};

export const bridge = inApp
  ? {
      load: () => invoke<AppState>("get_state"),
      onChange: (then: () => void) => listen("changed", then),
      onShowPane: (then: (pane: string) => void) => listen<string>("show-pane", (e) => then(e.payload)),
      onShowSetting: (then: (at: { pane: string; setting: string }) => void) =>
        listen<{ pane: string; setting: string }>("show-setting", (e) => then(e.payload)),
      set: <K extends keyof Settings>(key: K, value: Settings[K]) => invoke("set_setting", { key, value }),
      refresh: () => invoke("refresh_now"),
      openUsagePage: () => invoke("open_usage_page"),
      testAlert: () => invoke("send_test_alert"),
      checkUpdates: () => invoke("check_updates"),
      installUpdate: () => invoke("install_update_now"),
      wallpaper: () => invoke<Wallpaper>("get_wallpaper").catch(() => null),
      asks: () => invoke<Ask[]>("get_asks"),
      answer: (id: string, choice: string) => invoke("answer_request", { id, choice }),
      /** Save an image added to an answer, as a data URL, and get back where it went, for Claude to read. */
      saveAttachment: (data: string) => invoke<string>("save_attachment", { data }),
      extend: (id: string, ms: number) => invoke("extend_request", { id, ms }),
      closePopover: () => invoke("close_popover"),
      popoverPointer: () => invoke<[number, number] | null>("popover_pointer"),
      showPopover: () => invoke("show_popover_now"),
      openNotificationSettings: () => invoke("open_notification_settings"),
      popoverArrow: () => invoke<number>("popover_arrow"),
      popoverResize: (height: number) => invoke("popover_resize", { height }),
      onPopover: (event: "popover-open" | "popover-hide" | "popover-arrow", then: (payload: unknown) => void) =>
        listen(event, (e) => then(e.payload)),
      /** Hands-free: what's waiting on the user, for the list dropped down from the menu bar item. */
      pendingSessions: () => invoke<Pending[]>("pending_sessions"),
      openPending: (session: string) => invoke("open_pending", { session }),
      sendToSession: (session: string, text: string) => invoke("send_to_session", { session, text }),
      stopStep: (session: string) => invoke("stop_step", { session }),
      markSeen: (sessions: string[]) => invoke("mark_seen", { sessions }),
      setIcon: (session: string, icon: ChatIcon | null, wholeProject: boolean) => invoke("set_icon", { session, icon, wholeProject }),
      emojiNames: () => invoke<[string, string][]>("emoji_names"),
      plans: () => invoke<Plan[]>("plans"),
      savePlan: (plan: Plan) => invoke("save_plan", { plan }),
      deletePlan: (id: string) => invoke("delete_plan", { id }),
      openPlanner: () => invoke("open_planner_now"),
      headroomInFront: () => invoke<boolean>("headroom_in_front"),
      popoverCards: () => invoke("popover_cards"),
      /** The user's writing a reply in the list, which stays down meanwhile. */
      saveToken: (token: string) => invoke("save_token", { token }),
      clearToken: () => invoke("clear_token"),
    }
  : mockBridge();

/** The same calls as the app, on made-up data kept in memory. */
function mockBridge() {
  const listeners = new Set<() => void>();
  const changed = () => listeners.forEach((f) => f());
  return {
    load: async (): Promise<AppState> => structuredClone(mock),
    onChange: async (then: () => void) => {
      listeners.add(then);
      return () => void listeners.delete(then);
    },
    onShowPane: async (_then: (pane: string) => void) => () => {},
    onShowSetting: async (_then: (at: { pane: string; setting: string }) => void) => () => {},
    set: async <K extends keyof Settings>(key: K, value: Settings[K]) => {
      mock.settings[key] = value;
      changed();
    },
    refresh: async () => {},
    openUsagePage: async () => void window.open("https://claude.ai/settings/usage"),
    testAlert: async () => {},
    checkUpdates: async () => {},
    installUpdate: async () => {},
    wallpaper: async (): Promise<Wallpaper | null> => null,
    asks: async (): Promise<Ask[]> => [],
    extend: async (_id: string, _ms: number) => {},
    closePopover: async () => {},
    popoverPointer: async (): Promise<[number, number] | null> => null,
    showPopover: async () => {},
    openNotificationSettings: async () => {},
    popoverArrow: async () => 60,
    popoverResize: async (_height: number) => {},
    onPopover: async (_event: string, _then: (payload: unknown) => void) => () => {},
    pendingSessions: async (): Promise<Pending[]> => [],
    openPending: async (_session: string) => {},
    sendToSession: async (_session: string, _text: string) => {},
    stopStep: async (_session: string) => {},
    markSeen: async (_sessions: string[]) => {},
    setIcon: async (_session: string, _icon: ChatIcon | null, _wholeProject: boolean) => {},
    plans: async (): Promise<Plan[]> => [myCompany as Plan],
    savePlan: async (_plan: Plan) => {},
    deletePlan: async (_id: string) => {},
    openPlanner: async () => {},
    headroomInFront: async () => false,
    emojiNames: async (): Promise<[string, string][]> => [
      ["🦊", "fox"],
      ["🐙", "octopus"],
      ["🦉", "owl"],
      ["🐝", "honeybee"],
      ["🐢", "turtle"],
      ["🦋", "butterfly"],
      ["🐳", "spouting whale"],
      ["🦕", "sauropod"],
      ["🚀", "rocket"],
      ["🛠️", "hammer and wrench"],
      ["🧪", "test tube"],
      ["📦", "package"],
      ["🧭", "compass"],
      ["🎨", "artist palette"],
      ["📚", "books"],
      ["🔭", "telescope"],
      ["❤️", "red heart"],
      ["🐶", "dog face"],
      ["🐱", "cat face"],
      ["🐸", "frog"],
      ["🍕", "pizza"],
      ["☕", "hot beverage"],
      ["🎸", "guitar"],
      ["🏔️", "snow-capped mountain"],
      ["🐞", "lady beetle"],
    ],
    popoverCards: async () => {},
    saveAttachment: async (_data: string) =>
      "/Users/you/Library/Application Support/io.github.dylan-hepworth.headroom/attachments/2026-09-26-194512-0.png",
    answer: async (id: string, _choice: string) => {
      const s = mock.sessions.find((s) => s.held?.id === id);
      if (s) Object.assign(s, { state: "working", held: null, request: undefined, activity: "Thinking", since: Date.now() });
      changed();
    },
    saveToken: async (token: string) => {
      if (!token.trim()) throw "Paste a token first.";
      mock.signIn = "token";
      changed();
    },
    clearToken: async () => {
      mock.signIn = "claude_code";
      changed();
    },
  };
}

const mock: AppState = {
  version: "1.1.0",
  status: "Updated 2:41:08 PM",
  ok: true,
  signIn: "token",
  update: null,
  asks: [],
  recap: {
    title: "Today: 6h 20m across 11 sessions",
    body: "Mostly headroom, photo-sorter and blog. Your 5-hour limit peaked at 91%, and the weekly limit went up 14%.",
  },
  shares: [
    { project: "headroom", pct: 48.2 },
    { project: "photo-sorter", pct: 31.5 },
    { project: "blog", pct: 17.9 },
    { project: "dotfiles", pct: 2.4 },
  ],
  limits: [
    { key: "5h", label: "5-hour", pct: 71, resets: "3:10 PM", elapsed: 58, pace: "2:59 PM", paceArrow: "ahead" },
    { key: "7d", label: "Weekly", pct: 60, resets: "Tue 9:00 AM", elapsed: 51, pace: "Mon 4:20 PM", paceArrow: "ahead" },
    { key: "7d_fable", label: "Fable weekly", pct: 14, resets: "Tue 9:00 AM", elapsed: 51, paceArrow: "behind" },
  ],
  sessions: [
    {
      id: "a1",
      icon: { emoji: "🚀" },
      project: "headroom",
      title: "Settings window polish",
      path: "~/Code/headroom",
      state: "permission",
      since: Date.now() - 2 * 60_000,
      request: { tool: "Bash", detail: "npm run build" },
      held: { id: "r1", canSession: true, kind: "permission", tool: "Bash", detail: "npm run build" },
    },
    {
      id: "b2",
      icon: { emoji: "🦉" },
      project: "photo-sorter",
      title: "Group duplicates by capture time",
      path: "~/Code/photo-sorter",
      state: "waiting",
      since: Date.now() - 6 * 60_000,
      last: "Done. Duplicates are now grouped by capture time, and the import skips files it has…",
    },
    {
      id: "c3",
      icon: { emoji: "📚" },
      project: "blog",
      path: "~/Code/blog",
      state: "working",
      since: Date.now() - 14 * 60_000,
      activity: "Editing posts/2026-09-rust-notes.md",
    },
    { id: "d4", project: "dotfiles", path: "~/dotfiles", state: "idle", since: Date.now() - 60 * 60_000 },
  ],
  settings: {
    interval: 60,
    launchAtLogin: true,
    autoUpdate: true,
    title: "both",
    ring: true,
    waitingCount: true,
    alert5h: 90,
    alert7d: 80,
    contextAlert: 70,
    persistentAlerts: true,
    hooks: true,
    approvals: true,
    popoverAuto: true,
    compactCards: false,
    askNext: false,
    handsFree: false,
    paused: { until: "4:10 PM" },
    paceArrows: true,
    approvalTimeout: 120,
    notifyWaiting: true,
    notifyDone: true,
    doneOnlyAway: true,
    doneMinRun: 60,
    nearLimit: true,
    nearLimitAt: 85,
    nearLimitRightAway: true,
    nearLimitMessage: "",
    nearLimitDefault: "Prefer small steps, and save progress before starting anything long.",
    resumeAlert: true,
    recap: true,
    recapAt: "18:00",
  },
};
