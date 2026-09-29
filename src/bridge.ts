// What the settings window knows and can change. Inside the app it calls the Rust side (the `#[tauri::command]`
// functions in main.rs, like `get_state` and `set_setting`). In a plain browser (`npm run ui`) it runs on the made-up
// data at the bottom, so the design can be worked on without building the app.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Pending } from "./Pending";
import type { ChatIcon } from "./Avatar";
import type { Heard } from "./Voice";
import type { Launch, Plan } from "./Planner";
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

/** A team from the planner, started on a chat (see team.rs): its managers, each a session of its own. */
/** Something Claude made in a turn, to look at from the panel (see made.rs): where it is, what it is, and its name. */
export type Made = { path: string; kind: "doc" | "image" | "link" | "file"; name: string };

/** How one of a manager's workers is getting on, from the hooks: by its name as a subagent. */
export type WorkerLive = { working: boolean; activity: string | null; steps: string[]; finished: number; said: string | null };

export type TeamRun = {
  id: string;
  /** The plan it was started from, as it was then */
  plan: Plan;
  name: string;
  lead: string;
  started: number;
  /** Whether a message for the lead or a manager at work goes in after its current step (see `messages_go_in`) */
  canMessage: boolean;
  /** How the lead's chat is getting on */
  leadState: { state: "working" | "needs-you" | "done"; doing: string | null; title: string } | null;
  members: {
    agent: string;
    name: string;
    model: string;
    session: string;
    state: "working" | "done" | "failed" | "stopped";
    /** Its report, or why it stopped */
    text: string;
    /** Each worker's name as a subagent, and which agent in the plan it is */
    workers: Record<string, string>;
    /** What it and its workers are doing, once its hooks have said */
    live: { activity: string | null; workers: Record<string, WorkerLive> } | null;
  }[];
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
  /** The shortcut for talking to a chat from anywhere, as the app reads it ("Control+Alt+Space"), or null when off. */
  talkShortcut: string | null;
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
      onPopover: (event: "popover-open" | "popover-hide" | "popover-arrow" | "talk-key", then: (payload: unknown) => void) =>
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
      /** Open the planner, following the team at work on `plan` if there's one. */
      lookAt: (path: string) => invoke("look_at", { item: path }),
      looking: () => invoke<boolean>("looking"),
      showMade: (path: string) => invoke("show_made", { item: path }),
      madePicture: (path: string) => invoke<string>("made_picture", { item: path }),
      readMade: (path: string) => invoke<string>("read_made", { item: path }),
      planFile: (id: string) => invoke<string | null>("plan_file", { id }),
      planPath: (id: string) => invoke<string | null>("plan_path", { id }),
      /** Save a team to a file the user picks; false if they cancel. */
      exportPlan: (name: string, text: string) => invoke<boolean>("export_plan", { name, text }),
      openPlanner: (plan?: string) => invoke("open_planner_now", { plan: plan ?? null }),
      onPlannerFollow: (then: (plan: string) => void) => listen<string>("planner-follow", (e) => then(e.payload)),
      lastMessage: (session: string) => invoke<string | null>("last_message", { session }),
      lastChat: () => invoke<string | null>("last_chat"),
      listenStart: () => invoke("listen_start"),
      listenStop: () => invoke("listen_stop"),
      listenCancel: () => invoke("listen_cancel"),
      onVoice: (then: (heard: Heard) => void) => listen<Heard>("voice", (e) => then(e.payload)),
      startTeam: (lead: string, plan: Plan, name: string, managers: Launch[]) => invoke<string>("start_team", { lead, plan, name, managers }),
      stopTeam: (run: string) => invoke("stop_team", { run }),
      clearTeam: (run: string) => invoke("clear_team", { run }),
      updateTeamPlan: (run: string, plan: Plan) => invoke("update_team_plan", { run, plan }),
      addManager: (run: string, manager: Launch) => invoke("add_manager", { run, manager }),
      stopMember: (run: string, agent: string) => invoke("stop_member", { run, agent }),
      messageManager: (run: string, agent: string, text: string) => invoke("message_manager", { run, agent, text }),
      teams: () => invoke<TeamRun[]>("teams"),
      headroomInFront: () => invoke<boolean>("headroom_in_front"),
      popoverCards: () => invoke("popover_cards"),
      /** The user's writing a reply in the list, which stays down meanwhile. */
      saveToken: (token: string) => invoke("save_token", { token }),
      clearToken: () => invoke("clear_token"),
    }
  : mockBridge();

/** My company partway through, for `?planner&running`: Marketing's done, Development's Coder is on its second round
 *  of review, and R&D's Summarizer is writing while the Lead waits on them. */
function mockRun(): TeamRun {
  const worker = (finished: number, working: boolean, steps: string[], said: string | null = null) => ({
    working,
    activity: steps.at(-1) ?? null,
    steps,
    finished,
    said,
  });
  return {
    id: "team-1",
    plan: myCompany as Plan,
    name: "My company",
    lead: "lead1",
    started: Date.now() - 18 * 60_000,
    canMessage: true,
    leadState: { state: "done", doing: "Handed the launch to the team.", title: "Launch the sign-in redesign" },
    members: [
      {
        agent: "mkt",
        name: "Marketing",
        model: "opus",
        session: "m1",
        state: "done",
        text: "The launch post and three social posts are ready.\n\n- **Post**: drafts/launch.md, reviewed for tone\n- **Social**: three posts scheduled for Tuesday at 9:00 AM",
        workers: { copywriter: "copy", "social-posts": "social" },
        live: {
          activity: null,
          workers: {
            copywriter: worker(1, false, ["Read docs/sign-in.md", "Edited launch.md"], "Wrote the launch post in drafts/launch.md."),
            "social-posts": worker(1, false, ["Read launch.md", "Edited social.md"], "Scheduled 3 posts for Tuesday."),
          },
        },
      },
      {
        agent: "dev",
        name: "Development",
        model: "opus",
        session: "m2",
        state: "working",
        text: "",
        workers: { coder: "coder", "code-reviewer": "review", tester: "tests" },
        live: {
          activity: null,
          workers: {
            coder: worker(1, true, ["Read the reviewer's notes", "Edited session.ts", "Ran npm test", "Edited session.ts"]),
            "code-reviewer": worker(1, false, ["Read session.ts", "Read magic-link.ts"], "Round 1: split `createSession` into smaller functions."),
          },
        },
      },
      {
        agent: "rnd",
        name: "R&D",
        model: "opus",
        session: "m3",
        state: "working",
        text: "",
        workers: { researcher: "research", summarizer: "sum", findings: "brief" },
        live: {
          activity: null,
          workers: {
            researcher: worker(1, false, ["Looked something up", "Looked something up", "Edited services.md"], "Compared 6 magic link services."),
            summarizer: worker(0, true, ["Read services.md", "Edited summary.md"]),
          },
        },
      },
    ],
  };
}

/** A made-up voice for the mic: louder and quieter by turns, a sentence coming in a few words at a time, then quiet,
 *  so a clicked mic stops itself the way it would for real. */
function mockVoice() {
  const said = "Tell the coder to keep the session code in one file";
  const heard = new Set<(heard: Heard) => void>();
  const tell = (h: Heard) => heard.forEach((f) => f(h));
  let timer: ReturnType<typeof setInterval> | undefined;
  let words = "";
  const end = () => clearInterval(timer);
  return {
    listenStart: async () => {
      end();
      let t = 0;
      words = "";
      timer = setInterval(() => {
        t++;
        const all = said.split(" ");
        const now = all.slice(0, Math.floor(t / 4)).join(" ");
        const talking = Math.floor(t / 4) <= all.length;
        tell({ kind: "level", level: talking ? 0.25 + 0.6 * Math.abs(Math.sin(t / 2.3)) * Math.random() : 0.02 });
        if (now !== words) tell({ kind: "words", text: (words = now) });
      }, 80);
    },
    listenStop: async () => {
      end();
      tell({ kind: "done", text: words });
    },
    listenCancel: async () => end(),
    onVoice: async (then: (heard: Heard) => void) => {
      heard.add(then);
      return () => void heard.delete(then);
    },
  };
}

/** Plans' files, as the preview keeps them. `headroomPlanFile` stands in for Claude changing one. */
const mockPlanFiles: Record<string, string> = { "my-company": JSON.stringify(myCompany, null, 2) };

/** The same calls as the app, on made-up data kept in memory. */
function mockBridge() {
  (window as unknown as { headroomPlanFile: (id: string, text: string) => void }).headroomPlanFile = (id, text) => (mockPlanFiles[id] = text);
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
    pendingSessions: async (): Promise<Pending[]> => [
      {
        id: "c3",
        title: "Blog redesign",
        project: "blog",
        state: "working",
        since: Date.now() - 14 * 60_000,
        what: "Editing posts/2026-09-rust-notes.md",
        icon: { emoji: "📚" },
      },
    ],
    openPending: async (_session: string) => {},
    sendToSession: async (_session: string, _text: string) => {},
    stopStep: async (_session: string) => {},
    markSeen: async (_sessions: string[]) => {},
    setIcon: async (_session: string, _icon: ChatIcon | null, _wholeProject: boolean) => {},
    plans: async (): Promise<Plan[]> => [myCompany as Plan],
    savePlan: async (plan: Plan) => void (mockPlanFiles[plan.id] = JSON.stringify(plan, null, 2)),
    planFile: async (id: string) => mockPlanFiles[id] ?? null,
    planPath: async (id: string) => `~/Library/Application Support/io.github.dylan-hepworth.headroom/plans/${id}.json`,
    deletePlan: async (_id: string) => {},
    lookAt: async (path: string) => void (path.startsWith("http") && window.open(path)),
    looking: async () => false,
    showMade: async (_path: string) => {},
    madePicture: async (_path: string) => {
      const picture = (await import("./Popover")).PREVIEW_PICTURE;
      return "image" in picture ? picture.image : "";
    },
    readMade: async (_path: string) =>
      "# Duplicates\n\n214 pairs that look like the same photo, grouped by how they differ:\n\n| Kind | Pairs | Keep |\n| --- | ---: | --- |\n| Only the size | 180 | The larger file |\n| Seconds apart | 29 | Ask each time |\n| Different edits | 5 | Both |\n\n## Next\n\n- Nothing's deleted until you say\n- The groups are in `duplicates.json`",
    // In a browser, it downloads
    exportPlan: async (name: string, text: string) => {
      const link = document.createElement("a");
      link.href = URL.createObjectURL(new Blob([text], { type: "application/json" }));
      link.download = `${name}.json`;
      link.click();
      return true;
    },
    openPlanner: async (_plan?: string) => {},
    onPlannerFollow: async (_then: (plan: string) => void) => () => {},
    lastChat: async (): Promise<string | null> => "c3",
    ...mockVoice(),
    lastMessage: async (_session: string): Promise<string | null> =>
      "Launch the sign-in redesign: new login page, magic links, and the help docs for it.",
    startTeam: async (_lead: string, _plan: Plan, _name: string, _managers: Launch[]) => "team-1",
    stopTeam: async (_run: string) => {},
    clearTeam: async (_run: string) => {},
    updateTeamPlan: async (_run: string, _plan: Plan) => {},
    addManager: async (_run: string, _manager: Launch) => {},
    stopMember: async (_run: string, _agent: string) => {},
    messageManager: async (_run: string, _agent: string, _text: string) => {},
    teams: async (): Promise<TeamRun[]> => (location.search.includes("running") ? [mockRun()] : []),
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
    talkShortcut: "Control+Alt+Space",
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
