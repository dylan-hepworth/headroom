// The agent planner: a team of agents laid out on a grid and wired together, saved as a template, and started on a
// Claude Code chat as instructions for it.
//
// Each agent has a role, a model, standing instructions, commands it may use, what it may do, and something to keep
// going until, if it should. An arrow from a lead or a manager hands a job down; an arrow from a worker hands its work
// on, so the next one waits for it. An arrow can loop, sending the work back until the second agent is happy.
//
// For now the chat's Claude runs the whole team itself: the lead is the chat, and every worker one of its subagents,
// with its model. Claude Code's subagents can't start their own, so a manager can't run its team yet; what's written
// for it guides the lead instead, and what an agent may do is asked of it rather than enforced.

import { useEffect, useRef, useState } from "react";
import { Avatar, IconPicker, type ChatIcon } from "./Avatar";
import { bridge, type TeamRun } from "./bridge";
import { SpokenField } from "./Voice";
import { changes, checkTeam, type Change } from "./planChanges";
import { LivePanel, liveOf, RunBar } from "./Running";
import { keepShare, savedShare, ShareSheet, TogetherPanel, type Said, type Share } from "./Together";
import "./plan.css";
import "./popover.css";

export type Model = "Opus" | "Sonnet" | "Haiku";
export type Role = "Lead" | "Manager" | "Worker";
export type Agent = {
  id: string;
  name: string;
  icon: ChatIcon;
  model: Model;
  role: Role;
  x: number;
  y: number;
  /** Its standing instructions */
  brief: string;
  commands: string[];
  tools: string[];
  /** Keep going until this holds, if set */
  until?: string;
};
/** Work handed from one agent to another. A loop sends it back until the second one's happy, up to `rounds` times. */
export type Edge = { from: string; to: string; loop?: { until: string; rounds: number } };
/** A team, saved to start on any chat. `updated` is when it was last saved, in milliseconds since 1970. */
export type Plan = { id: string; name: string; agents: Agent[]; edges: Edge[]; updated?: number };

const GRID = 20;
export const W = 160;
export const H = 64;
export const MODELS: Model[] = ["Opus", "Sonnet", "Haiku"];
export const TOOLS = ["Read files", "Edit files", "Run commands", "Browse the web"];
const snap = (v: number) => Math.round(v / GRID) * GRID;
export const edgeId = (e: Edge) => `${e.from}>${e.to}`;

/** Would an arrow from `from` to `to` lead back round to `from`? Only loops may do that, and they say so. */
function makesCircle(edges: Edge[], from: string, to: string) {
  const seen = new Set<string>();
  const walk = (at: string): boolean => {
    if (at === from) return true;
    if (seen.has(at)) return false;
    seen.add(at);
    return edges.filter((e) => e.from === at).some((e) => walk(e.to));
  };
  return walk(to);
}

/** What's worth a word about the plan: no lead, agents the lead can't reach, a worker two managers share. */
export function problems(plan: Plan): string[] {
  const out: string[] = [];
  const lead = plan.agents.find((a) => a.role === "Lead");
  if (!lead) return ["Make one of the agents the Lead."];
  const reached = new Set<string>();
  const walk = (id: string) => {
    if (reached.has(id)) return;
    reached.add(id);
    plan.edges.filter((e) => e.from === id).forEach((e) => walk(e.to));
  };
  walk(lead.id);
  const lost = plan.agents.filter((a) => !reached.has(a.id));
  if (lost.length) out.push(`${lost.map((a) => a.name).join(", ")} ${lost.length === 1 ? "isn't" : "aren't"} connected to the Lead.`);
  for (const a of plan.agents) {
    const managers = new Set(teamOf(plan, a.id));
    if (managers.size > 1) out.push(`${a.name} is in more than one team.`);
  }
  return out;
}

/** The managers whose teams an agent is in: the ones it's reached from without passing through the lead. */
function teamOf(plan: Plan, id: string): string[] {
  const byId = (x: string) => plan.agents.find((a) => a.id === x);
  const found = new Set<string>();
  const seen = new Set<string>();
  const up = (at: string) => {
    if (seen.has(at)) return;
    seen.add(at);
    for (const e of plan.edges.filter((e) => e.to === at)) {
      const from = byId(e.from);
      if (!from || from.role === "Lead") continue;
      if (from.role === "Manager") found.add(from.id);
      else up(from.id);
    }
  };
  up(id);
  return [...found];
}

const lower = (s: string) => s.charAt(0).toLowerCase() + s.slice(1);
/** Someone's words, to go into a sentence: without the full stop they ended with, and lower case to start. */
const clause = (s: string) => lower(s.trim().replace(/[.!\s]+$/, ""));
export const list = (xs: string[]) => (xs.length < 2 ? xs.join("") : `${xs.slice(0, -1).join(", ")} and ${xs[xs.length - 1]}`);

/** What's said about one agent: what it's for, what it waits for, how it loops, and what it may use. */
function describe(plan: Plan, a: Agent) {
  const byId = (id: string) => plan.agents.find((x) => x.id === id)!;
  const waits = plan.edges.filter((e) => e.to === a.id && !e.loop && byId(e.from).role === "Worker").map((e) => byId(e.from).name);
  const parts = [a.brief.trim() ? `${clause(a.brief)}.` : ""];
  if (waits.length) parts.push(`It starts once ${list(waits)} ${waits.length === 1 ? "is" : "are"} done.`);
  for (const e of plan.edges.filter((e) => e.to === a.id && e.loop)) {
    const from = byId(e.from).name;
    parts.push(`It reviews ${from}'s work: send it back to ${from} until ${clause(e.loop!.until)}, ${e.loop!.rounds} rounds at most.`);
  }
  if (a.until) parts.push(`It keeps going until ${clause(a.until)}.`);
  if (a.commands.length) parts.push(`It may use ${list(a.commands)}.`);
  return parts.filter(Boolean).join(" ");
}

/** The workers in the order their work flows: each after the workers that hand it theirs. */
function inFlow(plan: Plan): Agent[] {
  const byId = (id: string) => plan.agents.find((x) => x.id === id)!;
  const placed: Agent[] = [];
  const place = (a: Agent, path = new Set<string>()) => {
    if (placed.includes(a) || path.has(a.id)) return;
    path.add(a.id);
    plan.edges.filter((e) => e.to === a.id && !e.loop && byId(e.from).role === "Worker").forEach((e) => place(byId(e.from), path));
    placed.push(a);
  };
  plan.agents.filter((a) => a.role === "Worker").forEach((a) => place(a));
  return placed;
}

/** The workers on a manager's team, or with none, the ones that report to the lead directly. One in more than one
 *  team goes with the first (the planner says so). */
export const teamFor = (plan: Plan, manager: string | null) => inFlow(plan).filter((a) => (teamOf(plan, a.id)[0] ?? null) === manager);

/** The plan in words, for the chat to run it all itself: every worker a subagent of its own, grouped under its manager,
 *  whose words guide the lead. */
export function verbalize(plan: Plan): string {
  const lead = plan.agents.find((a) => a.role === "Lead");
  if (!lead) return "";
  const lines: string[] = [
    `Work through this with a team, the way I've planned it. You're the ${lead.name}${lead.brief.trim() ? `: ${clause(lead.brief)}` : ""}.`,
    `Run each agent below as a subagent of your own, with the model it's given, and use its name as the subagent's description, so I can follow each one. Where an agent waits for others, start it once they're done.`,
  ];
  const say = (a: Agent) => {
    const tools = a.tools.length && a.tools.length < TOOLS.length ? ` It should only ${list(a.tools.map(lower))}.` : "";
    lines.push(`- ${a.name} (${a.model}): ${describe(plan, a)}${tools}`.replace(": .", "."));
  };
  const managers = plan.agents.filter((a) => a.role === "Manager");
  for (const m of managers) {
    const team = teamFor(plan, m.id);
    if (!team.length) continue;
    lines.push(
      "",
      `${m.name} team${m.brief.trim() ? `, with this in mind: ${clause(m.brief)}.` : ":"}${m.commands.length ? ` It may use ${list(m.commands)}.` : ""}`,
    );
    team.forEach(say);
  }
  const loose = teamFor(plan, null);
  if (loose.length) {
    lines.push("", managers.length ? "Reporting to you directly:" : "The team:");
    loose.forEach(say);
  }
  lines.push("", "Bring their work together when they're done, and tell me what each one did.");
  return lines.join("\n");
}

/** Claude Code's tools for what the planner's boxes allow. */
const TOOL_NAMES: Record<string, string[]> = {
  "Read files": ["Read", "Glob", "Grep"],
  "Edit files": ["Edit", "Write", "MultiEdit", "NotebookEdit"],
  "Run commands": ["Bash"],
  "Browse the web": ["WebFetch", "WebSearch"],
};
const toolNames = (tools: string[]) => [...new Set(tools.flatMap((t) => TOOL_NAMES[t] ?? []))];

/** A manager to start as a session of its own (see team.rs). */
export type Launch = {
  agent: string;
  name: string;
  model: string;
  prompt: string;
  tools: string[];
  agents: Record<string, unknown>;
  /** Each worker's name as a subagent, and which agent in the plan it is */
  workers: Record<string, string>;
};

/** How a plan starts on a chat, for `work`: what the lead's told, and each manager that runs as a session of its own,
 *  with its workers as its subagents. A plan without managers is all the lead's, as `verbalize` says it. */
export function launch(plan: Plan, work: string): { lead: string; managers: Launch[] } {
  const lead = plan.agents.find((a) => a.role === "Lead");
  const managers = plan.agents.filter((a) => a.role === "Manager" && teamFor(plan, a.id).length);
  const said = work.trim() ? `The work: ${work.trim()}\n\n` : "";
  if (!lead || !managers.length) return { lead: said + verbalize(plan), managers: [] };

  const lines = [
    `${said}Work through this with a team, the way I've planned it. You're the ${lead.name}${lead.brief.trim() ? `: ${clause(lead.brief)}` : ""}.`,
    `Headroom is running ${list(managers.map((m) => m.name))} as sessions of their own, on this same work, each with its own team. Don't do their parts yourself. Their reports will come to this chat as they finish; once they're all in, bring them together and tell me what each one did.`,
  ];
  const own = teamFor(plan, null);
  if (own.length) {
    lines.push("", "Your own part, with these as your subagents, each with the model given and its name as the subagent's description:");
    own.forEach((a) => lines.push(`- ${a.name} (${a.model}): ${describe(plan, a)}`.replace(": .", ".")));
  }

  const specs: Launch[] = managers.map((m) => {
    const team = teamFor(plan, m.id);
    // Each worker's name as a subagent, one of its own even when two share a name, and which agent that is
    const keys = new Map<string, string>();
    for (const a of team) {
      const base =
        a.name
          .toLowerCase()
          .replace(/[^a-z0-9]+/g, "-")
          .replace(/^-|-$/g, "") || a.id;
      let key = base;
      for (let n = 2; [...keys.values()].includes(key); n++) key = `${base}-${n}`;
      keys.set(a.id, key);
    }
    const agents = Object.fromEntries(
      team.map((a) => [
        keys.get(a.id)!,
        {
          description: a.name + (a.brief.trim() ? `: ${clause(a.brief)}` : ""),
          prompt: `You're ${a.name}. ${describe(plan, a)}`,
          model: a.model.toLowerCase(),
          tools: toolNames(a.tools),
        },
      ]),
    );
    // A manager can do what any of its team can, since it's its session they run in, and it starts them
    const tools = [
      ...new Set([
        ...toolNames(m.tools),
        ...team.flatMap((a) => toolNames(a.tools)),
        "Task",
        "Agent",
        "TodoWrite",
        ...(m.commands.length ? ["Skill", "SlashCommand"] : []),
      ]),
    ];
    const prompt = [
      work.trim(),
      `You're ${m.name}, one of the managers on a team the user planned in Headroom${m.brief.trim() ? `: ${clause(m.brief)}` : ""}. The other managers handle their parts, and the lead brings everything together; stick to yours.`,
      `Run your team as your subagents. They're set up with their models and what they may do:`,
      ...team.map((a) => `- ${a.name}: ${describe(plan, a)}`.replace(": .", ".")),
      m.commands.length ? `You may use ${list(m.commands)}.` : "",
      m.until ? `Keep going until ${clause(m.until)}.` : "",
      "When your team's done, end with a short report of what it did and anything the lead should know. That report is what the lead gets.",
    ].filter(Boolean);
    const workers = Object.fromEntries(team.map((a) => [keys.get(a.id)!, a.id]));
    return { agent: m.id, name: m.name, model: m.model.toLowerCase(), prompt: prompt.join("\n\n"), tools, agents, workers };
  });
  return { lead: lines.join("\n"), managers: specs };
}

/** How far the grid can be zoomed out and in. */
const ZOOM = { min: 0.3, max: 2 };

/** The zoom the planner was left at, for the next time it's open. */
function savedZoom() {
  try {
    return Math.min(ZOOM.max, Math.max(ZOOM.min, Number(localStorage.getItem("planner-zoom")) || 1));
  } catch {
    return 1;
  }
}

/** What a drag on the grid is doing: panning it, moving agents, or drawing a box to pick the agents inside. */
type Gesture =
  | { kind: "pan"; x: number; y: number; left: number; top: number }
  | { kind: "move"; x: number; y: number; from: Record<string, { x: number; y: number }> }
  | { kind: "box"; x: number; y: number; add: string[] };

export function Canvas({
  agents,
  edges,
  selected,
  group = [],
  onSelect,
  onGroup,
  onMove,
  onConnect,
  live,
  rounds,
}: {
  agents: Agent[];
  edges: Edge[];
  selected: string | null;
  /** The agents picked together, when it's more than one */
  group?: string[];
  onSelect: (id: string | null) => void;
  /** Pick these agents together */
  onGroup?: (ids: string[]) => void;
  /** Agents dragged somewhere, by ID */
  onMove?: (moves: Record<string, { x: number; y: number }>) => void;
  onConnect?: (from: string, to: string) => void;
  /** While it runs, how each agent's getting on (see Running.tsx) */
  live?: Record<string, { status: string; doing: string }>;
  /** While it runs, the round each loop's on, by arrow */
  rounds?: Record<string, number>;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [wire, setWire] = useState<{ from: string; x: number; y: number } | null>(null);
  const gesture = useRef<Gesture | null>(null);
  const [marquee, setMarquee] = useState<{ x0: number; y0: number; x1: number; y1: number } | null>(null);
  const [grabbing, setGrabbing] = useState(false);
  // Held down, the space bar turns a drag into panning, the way it does in design apps
  const [space, setSpace] = useState(false);

  const [zoom, setZoomNow] = useState(savedZoom);
  const scale = useRef(zoom);
  scale.current = zoom;
  /** Zoom to `next`, keeping the point at (cx, cy) in the view where it is: the pointer's, or the middle. */
  const zoomTo = (next: number, cx?: number, cy?: number) => {
    const el = box.current;
    if (!el) return;
    const to = Math.min(ZOOM.max, Math.max(ZOOM.min, next));
    const [px, py] = [cx ?? el.clientWidth / 2, cy ?? el.clientHeight / 2];
    const [gx, gy] = [(el.scrollLeft + px) / scale.current, (el.scrollTop + py) / scale.current];
    scale.current = to;
    setZoomNow(to);
    try {
      localStorage.setItem("planner-zoom", String(to));
    } catch {
      // Not kept for next time, which is fine
    }
    requestAnimationFrame(() => {
      el.scrollLeft = gx * to - px;
      el.scrollTop = gy * to - py;
    });
  };
  const zoomer = useRef(zoomTo);
  zoomer.current = zoomTo;

  useEffect(() => {
    const el = box.current!;
    const inBox = (e: { clientX: number; clientY: number }) => {
      const r = el.getBoundingClientRect();
      return [e.clientX - r.left, e.clientY - r.top] as const;
    };
    // A pinch on the trackpad comes as a wheel with ⌃ held, or as a gesture; ⌘ with the wheel zooms too
    const wheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      zoomer.current(scale.current * Math.exp(-e.deltaY * 0.01), ...inBox(e));
    };
    let base = 1;
    const pinchStart = (e: Event) => {
      e.preventDefault();
      base = scale.current;
    };
    const pinch = (e: Event) => {
      e.preventDefault();
      const g = e as Event & { scale: number; clientX: number; clientY: number };
      zoomer.current(base * g.scale, ...inBox(g));
    };
    const typing = (e: KeyboardEvent) => !!(e.target as Element).closest?.("input, textarea, [contenteditable]");
    const down = (e: KeyboardEvent) => {
      if (e.metaKey && (e.key === "=" || e.key === "+")) {
        e.preventDefault();
        zoomer.current(scale.current * 1.25);
      } else if (e.metaKey && e.key === "-") {
        e.preventDefault();
        zoomer.current(scale.current / 1.25);
      } else if (e.metaKey && e.key === "0") {
        e.preventDefault();
        zoomer.current(1);
      } else if (e.key === " " && !typing(e)) {
        e.preventDefault();
        setSpace(true);
      }
    };
    const up = (e: KeyboardEvent) => e.key === " " && setSpace(false);
    const away = () => setSpace(false);
    el.addEventListener("wheel", wheel, { passive: false });
    el.addEventListener("gesturestart", pinchStart);
    el.addEventListener("gesturechange", pinch);
    window.addEventListener("keydown", down);
    window.addEventListener("keyup", up);
    window.addEventListener("blur", away);
    return () => {
      el.removeEventListener("wheel", wheel);
      el.removeEventListener("gesturestart", pinchStart);
      el.removeEventListener("gesturechange", pinch);
      window.removeEventListener("keydown", down);
      window.removeEventListener("keyup", up);
      window.removeEventListener("blur", away);
    };
  }, []);

  /** Where the pointer is on the grid, in the plan's own units, whatever the zoom and scroll. */
  const at = (e: React.PointerEvent) => {
    const el = box.current!;
    const r = el.getBoundingClientRect();
    return { x: (e.clientX - r.left + el.scrollLeft) / zoom, y: (e.clientY - r.top + el.scrollTop) / zoom };
  };
  const byId = (id: string) => agents.find((n) => n.id === id);
  const width = Math.max(...agents.map((a) => a.x + W), 0) + 200;
  const height = Math.max(...agents.map((a) => a.y + H), 0) + 200;
  const isBackground = (t: EventTarget) =>
    t === box.current || ["grid-stage", "grid-content", "grid-wires"].some((c) => (t as Element).classList?.contains(c));
  const inside = (m: NonNullable<typeof marquee>) => {
    const [left, right, top, bottom] = [Math.min(m.x0, m.x1), Math.max(m.x0, m.x1), Math.min(m.y0, m.y1), Math.max(m.y0, m.y1)];
    return agents.filter((n) => n.x < right && n.x + W > left && n.y < bottom && n.y + H > top).map((n) => n.id);
  };
  const picked = (id: string) => selected === id || group.includes(id);

  return (
    <div className="grid-wrap">
      <div
        className={["grid-canvas", space && "panning", grabbing && "grabbing"].filter(Boolean).join(" ")}
        ref={box}
        style={{ backgroundSize: `${20 * zoom}px ${20 * zoom}px` }}
        onPointerDown={(e) => {
          const el = box.current!;
          if (space || e.button === 1) {
            e.preventDefault();
            gesture.current = { kind: "pan", x: e.clientX, y: e.clientY, left: el.scrollLeft, top: el.scrollTop };
            setGrabbing(true);
            el.setPointerCapture(e.pointerId);
            return;
          }
          if (!isBackground(e.target)) return;
          if (!onGroup || !onMove) return onSelect(null);
          const p = at(e);
          const already = group.length ? group : selected && byId(selected) ? [selected] : [];
          gesture.current = { kind: "box", ...p, add: e.shiftKey ? already : [] };
          setMarquee({ x0: p.x, y0: p.y, x1: p.x, y1: p.y });
          el.setPointerCapture(e.pointerId);
        }}
        onPointerMove={(e) => {
          const g = gesture.current;
          const p = at(e);
          if (wire) setWire({ ...wire, ...p });
          if (!g) return;
          if (g.kind === "pan") {
            box.current!.scrollLeft = g.left - (e.clientX - g.x);
            box.current!.scrollTop = g.top - (e.clientY - g.y);
          } else if (g.kind === "move" && onMove) {
            const [dx, dy] = [snap(p.x - g.x), snap(p.y - g.y)];
            onMove(Object.fromEntries(Object.entries(g.from).map(([id, f]) => [id, { x: Math.max(0, f.x + dx), y: Math.max(0, f.y + dy) }])));
          } else if (g.kind === "box" && marquee && onGroup) {
            const next = { ...marquee, x1: p.x, y1: p.y };
            setMarquee(next);
            onGroup([...new Set([...g.add, ...inside(next)])]);
          }
        }}
        onPointerUp={(e) => {
          if (wire && onConnect) {
            const p = at(e);
            const onto = agents.find((n) => p.x >= n.x && p.x <= n.x + W && p.y >= n.y && p.y <= n.y + H);
            if (onto && onto.id !== wire.from) onConnect(wire.from, onto.id);
          }
          // A click on the grid, rather than a drag across it, lets go of what's picked
          const g = gesture.current;
          if (g?.kind === "box" && marquee && !g.add.length && Math.hypot(marquee.x1 - marquee.x0, marquee.y1 - marquee.y0) < 4) onSelect(null);
          gesture.current = null;
          setWire(null);
          setMarquee(null);
          setGrabbing(false);
        }}
      >
        <div className="grid-stage" style={{ width: width * zoom, height: height * zoom }}>
          <div className="grid-content" style={{ width, height, transform: `scale(${zoom})` }}>
            <svg className="grid-wires" width={width} height={height}>
              <defs>
                <marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                  <path d="M0 1L9 5L0 9z" />
                </marker>
              </defs>
              {edges.map((e) => {
                const [a, b] = [byId(e.from), byId(e.to)];
                if (!a || !b) return null;
                return (
                  <g
                    key={edgeId(e)}
                    className={selected === edgeId(e) ? "wire selected" : "wire"}
                    onPointerDown={(ev) => !space && (ev.stopPropagation(), onSelect(edgeId(e)))}
                  >
                    <path className="wire-hit" d={path(a, b)} />
                    <path d={path(a, b)} markerEnd="url(#arrow)" />
                    {e.loop && (
                      <>
                        <path className="wire-hit" d={loopPath(a, b)} />
                        <path className="wire-loop" d={loopPath(a, b)} markerEnd="url(#arrow)" />
                        {/* Wide enough for a long "until": the tag sits in the middle of it */}
                        <foreignObject x={loopMiddle(a, b).x - 240} y={loopMiddle(a, b).y - 15} width={480} height={30}>
                          <div className="loop-tag-box">
                            <div className="loop-tag">
                              {rounds?.[edgeId(e)]
                                ? `↻ round ${rounds[edgeId(e)]} of ${e.loop.rounds}`
                                : `↻ until ${e.loop.until} · max ${e.loop.rounds}`}
                            </div>
                          </div>
                        </foreignObject>
                      </>
                    )}
                  </g>
                );
              })}
              {wire && byId(wire.from) && (
                <path className="wire drawing" d={`M${byId(wire.from)!.x + W / 2} ${byId(wire.from)!.y + H} L${wire.x} ${wire.y}`} />
              )}
            </svg>
            {agents.map((n) => (
              <div
                key={n.id}
                className={["grid-node", n.role.toLowerCase(), live?.[n.id]?.status, picked(n.id) && "selected"].filter(Boolean).join(" ")}
                style={{ left: n.x, top: n.y, width: W, height: H }}
                onPointerDown={(e) => {
                  // Panning, the grid has it
                  if (space || e.button === 1) return;
                  e.stopPropagation();
                  // ⇧-click adds it to what's picked, or takes it out
                  if (e.shiftKey && onGroup) {
                    const already = group.length ? group : selected && byId(selected) ? [selected] : [];
                    onGroup(already.includes(n.id) ? already.filter((id) => id !== n.id) : [...already, n.id]);
                    return;
                  }
                  const moving = group.includes(n.id) ? group : [n.id];
                  if (!group.includes(n.id)) onSelect(n.id);
                  if (onMove) {
                    const p = at(e);
                    const from = Object.fromEntries(moving.map((id) => [id, { x: byId(id)!.x, y: byId(id)!.y }]));
                    gesture.current = { kind: "move", ...p, from };
                    box.current!.setPointerCapture(e.pointerId);
                  }
                }}
              >
                <Avatar icon={n.icon} size={28} corner={live && <i className={`plan-dot ${live[n.id]?.status}`} />} />
                <div className="grid-node-text">
                  <b>{n.name}</b>
                  {live ? (
                    <span className="grid-doing">{live[n.id]?.doing}</span>
                  ) : (
                    <>
                      <span className={`model ${n.model.toLowerCase()}`}>{n.model}</span>
                      {n.until && <span className="until">↻ until {n.until}</span>}
                    </>
                  )}
                </div>
                {onConnect && (
                  <span
                    className="grid-port"
                    title="Drag onto another agent to hand it work"
                    onPointerDown={(e) => {
                      if (space) return;
                      e.stopPropagation();
                      setWire({ from: n.id, ...at(e) });
                      box.current!.setPointerCapture(e.pointerId);
                    }}
                  />
                )}
              </div>
            ))}
            {marquee && (
              <div
                className="grid-marquee"
                style={{
                  left: Math.min(marquee.x0, marquee.x1),
                  top: Math.min(marquee.y0, marquee.y1),
                  width: Math.abs(marquee.x1 - marquee.x0),
                  height: Math.abs(marquee.y1 - marquee.y0),
                }}
              />
            )}
          </div>
        </div>
      </div>
      <div className="grid-zoom">
        <button onClick={() => zoomTo(zoom / 1.25)} title="Zoom Out (⌘−)" disabled={zoom <= ZOOM.min}>
          −
        </button>
        <button className="grid-zoom-level" onClick={() => zoomTo(1)} title="Actual Size (⌘0)">
          {Math.round(zoom * 100)}%
        </button>
        <button onClick={() => zoomTo(zoom * 1.25)} title="Zoom In (⌘+)" disabled={zoom >= ZOOM.max}>
          +
        </button>
      </div>
    </div>
  );
}

/** A line from the bottom of one agent to the top of another, bending smoothly between. */
function path(a: Agent, b: Agent) {
  const [x1, y1, x2, y2] = [a.x + W / 2, a.y + H, b.x + W / 2, b.y];
  const bend = Math.max(30, Math.abs(y2 - y1) / 2);
  return `M${x1} ${y1} C${x1} ${y1 + bend} ${x2} ${y2 - bend} ${x2} ${y2}`;
}

/** A loop's way back: out of the left side of the second agent and round into the left side of the first. */
function loopPath(a: Agent, b: Agent) {
  const [x1, y1, x2, y2] = [b.x, b.y + H / 2, a.x, a.y + H / 2];
  const out = Math.min(x1, x2) - 44;
  return `M${x1} ${y1} C${out} ${y1} ${out} ${y2} ${x2} ${y2}`;
}

/** The middle of a loop's way back, where its tag sits. */
function loopMiddle(a: Agent, b: Agent) {
  const out = Math.min(a.x, b.x) - 44;
  return { x: (b.x + 6 * out + a.x) / 8, y: (a.y + b.y + H) / 2 };
}

/** Several agents picked together: their model, all at once, and deleting them. */
function GroupInspector({ agents, onAgents, onDelete }: { agents: Agent[]; onAgents: (agents: Agent[]) => void; onDelete: () => void }) {
  const model = agents.every((a) => a.model === agents[0].model) ? agents[0].model : null;
  return (
    <div className="inspector">
      <div className="inspector-title">{agents.length} agents</div>
      <div className="inspector-sub">{list(agents.map((a) => a.name))}</div>
      <div className="inspector-label">Model</div>
      <div className="segmented">
        {MODELS.map((m) => (
          <button key={m} className={model === m ? "on" : ""} onClick={() => onAgents(agents.map((a) => ({ ...a, model: m })))}>
            {m}
          </button>
        ))}
      </div>
      <div className="inspector-hint loop-hint">Drag any of them to move them all. ⇧-click an agent to add it or take it out.</div>
      <span className="inspector-delete-room" />
      <button className="ask-btn delete" onClick={onDelete} title="Their arrows go with them">
        Delete {agents.length} Agents <kbd>⌫</kbd>
      </button>
    </div>
  );
}

function Inspector({
  plan,
  selected,
  onAgent,
  onEdge,
  onDelete,
}: {
  plan: Plan;
  selected: string | null;
  onAgent: (a: Agent) => void;
  onEdge: (e: Edge) => void;
  /** Take away what's picked */
  onDelete: () => void;
}) {
  const [picking, setPicking] = useState(false);
  const [command, setCommand] = useState("");
  useEffect(() => setPicking(false), [selected]);
  const a = plan.agents.find((x) => x.id === selected);
  const e = plan.edges.find((x) => edgeId(x) === selected);
  if (e) {
    const [from, to] = [plan.agents.find((x) => x.id === e.from)!, plan.agents.find((x) => x.id === e.to)!];
    return (
      <div className="inspector">
        <div className="inspector-title">
          {from.name} → {to.name}
        </div>
        <div className="inspector-sub">
          {from.role === "Worker" ? `${to.name} gets ${from.name}'s work when it's done.` : `${from.name} hands ${to.name} its job.`}
        </div>
        <label className="inspector-check">
          <input
            type="checkbox"
            checked={!!e.loop}
            onChange={(ev) => onEdge({ ...e, loop: ev.target.checked ? { until: "approved", rounds: 3 } : undefined })}
          />
          Loop: send it back until {to.name} is happy
        </label>
        {e.loop && (
          <>
            <div className="inspector-label">Until</div>
            <input
              className="inspector-field"
              value={e.loop.until}
              onChange={(ev) => onEdge({ ...e, loop: { ...e.loop!, until: ev.target.value } })}
            />
            <div className="inspector-label">At most</div>
            <div className="segmented">
              {[2, 3, 5, 10].map((r) => (
                <button key={r} className={e.loop!.rounds === r ? "on" : ""} onClick={() => onEdge({ ...e, loop: { ...e.loop!, rounds: r } })}>
                  {r} rounds
                </button>
              ))}
            </div>
          </>
        )}
        <span className="inspector-delete-room" />
        <button className="ask-btn delete" onClick={onDelete}>
          Delete Arrow <kbd>⌫</kbd>
        </button>
      </div>
    );
  }
  if (!a) {
    return (
      <div className="inspector empty">
        Pick an agent or an arrow to set it up. Drag from the dot under an agent onto another to hand it work; a line into one from several means it
        waits for them all.
      </div>
    );
  }
  const addCommand = () => {
    const c = command.trim();
    if (c && !a.commands.includes(c)) onAgent({ ...a, commands: [...a.commands, c] });
    setCommand("");
  };
  return (
    <div className="inspector">
      <div className="inspector-head">
        <Avatar icon={a.icon} size={34} onClick={() => setPicking(!picking)} title="Change its icon" />
        <input className="inspector-name" value={a.name} onChange={(ev) => onAgent({ ...a, name: ev.target.value })} />
      </div>
      {picking && (
        <IconPicker
          icon={a.icon}
          project="this plan"
          onPick={(icon) => {
            onAgent({ ...a, icon });
            setPicking(false);
          }}
          onPicture={() => setPicking(false)}
          onReset={() => setPicking(false)}
        />
      )}
      <div className="inspector-label">Role</div>
      <div className="segmented">
        {(["Lead", "Manager", "Worker"] as Role[]).map((r) => (
          <button key={r} className={a.role === r ? "on" : ""} onClick={() => onAgent({ ...a, role: r })}>
            {r}
          </button>
        ))}
      </div>
      <div className="inspector-hint">
        {a.role === "Lead"
          ? "The chat's own Claude. There's one Lead."
          : a.role === "Manager"
            ? "Runs its team as a session of its own, in the background, and reports to the Lead's chat."
            : "Runs as a subagent, with its own model."}
      </div>
      <div className="inspector-label">Model</div>
      <div className="segmented">
        {MODELS.map((m) => (
          <button key={m} className={a.model === m ? "on" : ""} onClick={() => onAgent({ ...a, model: m })}>
            {m}
          </button>
        ))}
      </div>
      <div className="inspector-label">Standing instructions</div>
      <SpokenField
        className="inspector-field"
        rows={3}
        value={a.brief}
        placeholder="What it's for, and what it always keeps in mind"
        onChange={(brief) => onAgent({ ...a, brief })}
      />
      <div className="inspector-label">Commands and skills</div>
      <div className="chips">
        {a.commands.map((c) => (
          <span key={c} className="chip">
            {c}
            <button title="Take it off" onClick={() => onAgent({ ...a, commands: a.commands.filter((x) => x !== c) })}>
              ×
            </button>
          </span>
        ))}
        <input
          className="chip-input"
          placeholder="/command"
          value={command}
          onChange={(ev) => setCommand(ev.target.value)}
          onKeyDown={(ev) => ev.key === "Enter" && addCommand()}
          onBlur={addCommand}
        />
      </div>
      <div className="inspector-label">It should</div>
      {TOOLS.map((t) => (
        <label key={t} className="inspector-check">
          <input
            type="checkbox"
            checked={a.tools.includes(t)}
            onChange={() => onAgent({ ...a, tools: a.tools.includes(t) ? a.tools.filter((x) => x !== t) : [...a.tools, t] })}
          />
          {t}
        </label>
      ))}
      <label className="inspector-check">
        <input
          type="checkbox"
          checked={a.until !== undefined}
          onChange={(ev) => onAgent({ ...a, until: ev.target.checked ? "it's done" : undefined })}
        />
        Keep going until…
      </label>
      {a.until !== undefined && <input className="inspector-field" value={a.until} onChange={(ev) => onAgent({ ...a, until: ev.target.value })} />}
      <div className="inspector-hint loop-hint">
        To send work back until it's right, like a review: drag from an agent's bottom dot onto the one that checks it, click that arrow, and turn on
        Loop.
      </div>
      <span className="inspector-delete-room" />
      <button className="ask-btn delete" onClick={onDelete} title="Its arrows go with it">
        Delete Agent <kbd>⌫</kbd>
      </button>
    </div>
  );
}

/** A chat the plan can go to, and how it gets there: after the step it's on, as the reply to its finished turn, or
 *  copied, to paste in there. */
export type Chat = { id: string; title: string; project: string; icon?: ChatIcon; takes: "working" | "reply" | null };

/** Every chat Headroom knows about, the ones that can take a message now first. */
export function useChats(): Chat[] {
  const [chats, setChats] = useState<Chat[]>([]);
  useEffect(() => {
    Promise.all([bridge.pendingSessions(), bridge.load()]).then(([pending, app]) => {
      const now: Chat[] = pending
        .filter((p) => p.state === "working" || p.replyId)
        .map((p) => ({ id: p.id, title: p.title, project: p.project, icon: p.icon, takes: p.state === "working" ? "working" : "reply" }));
      const rest: Chat[] = app.sessions
        .filter((s) => !now.some((c) => c.id === s.id))
        .map((s) => ({ id: s.id, title: s.title ?? s.project, project: s.project, icon: s.icon, takes: null }));
      setChats([...now, ...rest]);
    });
  }, []);
  return chats;
}

/** Give a chat some words: straight in, if it's at work or its finished turn is held for a reply, or copied, with the
 *  chat opened to paste them into. Says which. */
export async function deliver(chat: string, text: string): Promise<"sent" | "copied"> {
  const now = (await bridge.pendingSessions()).find((p) => p.id === chat);
  if (now && (now.state === "working" || now.replyId)) {
    await bridge.sendToSession(chat, text);
    return "sent";
  }
  await navigator.clipboard.writeText(text);
  await bridge.openPending(chat);
  return "copied";
}

/** Where the plan goes: a chat that can take it now (at work, or finished with its turn held open for a reply), after
 *  a look at exactly what it'll be told. Any other chat gets it copied, to paste in there. */
function StartSheet({ plan, onClose }: { plan: Plan; onClose: () => void }) {
  const [chats, setChats] = useState<Chat[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);
  const [work, setWork] = useState("");
  const [usage, setUsage] = useState("");
  const [problem, setProblem] = useState("");
  const [done, setDone] = useState("");
  const [busy, setBusy] = useState(false);
  const { lead: text, managers } = launch(plan, work);
  useEffect(() => {
    // Every chat Headroom knows about, the ones that can take it now first
    Promise.all([bridge.pendingSessions(), bridge.load()]).then(([pending, app]) => {
      const now: Chat[] = pending
        .filter((p) => p.state === "working" || p.replyId)
        .map((p) => ({ id: p.id, title: p.title, project: p.project, icon: p.icon, takes: p.state === "working" ? "working" : "reply" }));
      const rest: Chat[] = app.sessions
        .filter((s) => !now.some((c) => c.id === s.id))
        .map((s) => ({ id: s.id, title: s.title ?? s.project, project: s.project, icon: s.icon, takes: null }));
      setChats([...now, ...rest]);
      setUsage(app.limits.map((l) => `${Math.round(l.pct)}% of your ${l.label} limit`).join(", "));
    });
  }, []);
  // The work starts as what the user last asked the chat, for them to keep, change, or write over
  const pick = (id: string) => {
    setChosen(id);
    bridge.lastMessage(id).then((said) => said && setWork(said));
  };
  const takes = (c: Chat) => c.takes !== null;
  const chat = chats.find((c) => c.id === chosen);
  const send = async () => {
    if (!chat || busy) return;
    setProblem("");
    setBusy(true);
    try {
      if (managers.length) await bridge.startTeam(chat.id, plan, plan.name, managers);
      const started = managers.length ? `Started ${list(managers.map((m) => m.name))}. ` : "";
      if (takes(chat)) {
        await bridge.sendToSession(chat.id, text);
        setDone(`${started}Sent to ${chat.title}.`);
      } else {
        await navigator.clipboard.writeText(text);
        await bridge.openPending(chat.id);
        setDone(`${started}Copied the lead's part: paste it into ${chat.title}.`);
      }
    } catch (e) {
      setProblem(`Couldn't start it: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="sheet-backdrop" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="sheet">
        <div className="inspector-title">Add {plan.name} to a chat</div>
        <div className="inspector-hint">The chat is the {plan.agents.find((a) => a.role === "Lead")?.name ?? "lead"}.</div>
        <div className="inspector-label">Chat</div>
        <div className="sheet-chats">
          {chats.length === 0 && <div className="inspector-hint">No chats are open. Start one in Claude or a terminal first.</div>}
          {chats.map((c) => (
            <button key={c.id} className={chosen === c.id ? "sheet-chat on" : "sheet-chat"} onClick={() => pick(c.id)}>
              {c.icon && <Avatar icon={c.icon} size={22} />}
              <span>
                <b>{c.title}</b> <span className="inspector-hint">{c.project}</span>
              </span>
              <span className="sheet-how">
                {c.takes === "working" ? "After its current step" : c.takes === "reply" ? "As your reply" : "Copy and paste"}
              </span>
            </button>
          ))}
        </div>
        <div className="inspector-label">The work</div>
        <SpokenField
          className="inspector-field"
          rows={3}
          value={work}
          placeholder={chosen ? "What the team's for" : "Pick a chat, and what you last asked it shows here to start from"}
          onChange={setWork}
        />
        {managers.length > 0 && (
          <div className="sheet-managers">
            Starts {managers.length} {managers.length === 1 ? "session" : "sessions"} in the background, in the chat's folder:{" "}
            {managers.map((m) => `${m.name} (${m.model.replace(/^./, (c) => c.toUpperCase())})`).join(", ")}. Each may only do what its team's boxes
            allow, and uses your Claude limits like any session{usage && ` (you're at ${usage})`}. They share the folder, so two editing the same
            files can clash. They stop if Headroom quits.
          </div>
        )}
        <div className="inspector-label">What the {plan.agents.find((a) => a.role === "Lead")?.name ?? "lead"} will be told</div>
        <pre className="sheet-text">{text}</pre>
        {problem && <div className="ask-problem">{problem}</div>}
        {done && <div className="pending-sent">{done}</div>}
        <div className="plan-foot">
          <span className="ask-spacer" />
          <button className="ask-btn ghost" onClick={onClose}>
            {done ? "Done" : "Cancel"}
          </button>
          {!done && (
            <button className="ask-btn primary" disabled={!chat || busy} onClick={send}>
              {managers.length ? "Start" : chat && !takes(chat) ? "Copy and Open" : "Send"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

let made = 0;
const newId = () => `a${Date.now().toString(36)}${made++}`;

/** A team as a file, for another Mac or another person: what it's called, its agents, and its arrows. */
const exportText = (plan: Plan) =>
  JSON.stringify({ kind: "headroom-team", version: 1, plan: { name: plan.name, agents: plan.agents, edges: plan.edges } }, null, 2);

/** A team from a file exported from Headroom, checked over and made one of this Mac's own, with a name none of
 *  `taken` has. Or, if it can't be read, why. Only what the planner knows goes in, so a file can't sneak in tools. */
/** Whether two versions of a plan are the same team: its name, its agents, and its arrows, as read from a file. */
function sameTeam(a: Plan, b: Plan) {
  const read = (p: Plan) => JSON.stringify(checkTeam({ name: p.name, agents: p.agents, edges: p.edges }));
  return read(a) === read(b);
}

export function readPlan(text: string, taken: string[]): Plan | string {
  const unreadable = "That file isn't a team exported from Headroom.";
  let data: unknown;
  try {
    data = JSON.parse(text);
  } catch {
    return unreadable;
  }
  const file = data as { kind?: unknown; plan?: unknown };
  if (file?.kind !== "headroom-team") return unreadable;
  const team = checkTeam(file.plan);
  if (typeof team === "string") return team;
  const base = team.name || "Imported team";
  let name = base;
  for (let n = 2; taken.includes(name); n++) name = `${base} ${n}`;
  return { id: newId(), ...team, name };
}

/** A file the user picks, as text, or null if they don't pick one. */
function chooseFile(accept: string): Promise<string | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = accept;
    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) return resolve(null);
      file.text().then(resolve, () => resolve(null));
    };
    input.addEventListener("cancel", () => resolve(null));
    input.click();
  });
}

/** "just now", "5m ago", "3h ago", "2d ago" */
function ago(ms?: number) {
  if (!ms) return "";
  const mins = Math.floor((Date.now() - ms) / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  if (mins < 60 * 24) return `${Math.floor(mins / 60)}h ago`;
  return `${Math.floor(mins / 1440)}d ago`;
}

/** A name for another version of a team: "My company" gives "My company v2", and "My company v2" gives v3, and so on,
 *  past any that are taken. */
function nextVersion(name: string, taken: string[]) {
  const base = name.replace(/ v\d+$/, "");
  let n = Number(/ v(\d+)$/.exec(name)?.[1] ?? 1) + 1;
  while (taken.includes(`${base} v${n}`)) n++;
  return `${base} v${n}`;
}

/** The planner window: one team open at a time, saved as it is or as a new version, and any saved one opened. */
export function PlannerWindow() {
  const [plans, setPlans] = useState<Plan[]>([]);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [saved, setSaved] = useState(true);
  const [selected, setSelectedNow] = useState<string | null>(null);
  // Agents picked together, by a box drawn round them, ⇧-clicks, or ⌘A: two or more, or none
  const [group, setGroupNow] = useState<string[]>([]);
  const setSelected = (id: string | null) => {
    setSelectedNow(id);
    setGroupNow([]);
  };
  const setGroup = (ids: string[]) => {
    setGroupNow(ids.length > 1 ? ids : []);
    setSelectedNow(ids.length === 1 ? ids[0] : null);
  };
  const [opening, setOpening] = useState(false);
  const [starting, setStarting] = useState(false);
  // Saving as a new version, with its name
  const [savingAs, setSavingAs] = useState<string | null>(null);
  // Something that would drop unsaved changes, waiting on whether to save them first
  const [unsaved, setUnsaved] = useState<{ then: () => void; doing: string } | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  // Teams at work on chats, and ones that finished in the last while, to follow and stop
  const [runs, setRuns] = useState<TeamRun[]>([]);
  const loadRuns = () => bridge.teams().then(setRuns);
  useEffect(() => {
    loadRuns();
    const timer = setInterval(loadRuns, 2000);
    return () => clearInterval(timer);
  }, []);
  // The open plan's latest run, followed on the grid until the user goes back to editing
  const run = plan ? runs.filter((r) => r.plan.id === plan.id).at(-1) : undefined;
  const [editing, setEditing] = useState<string | null>(null);
  const watching = !!run && editing !== run.id;
  const live = run && watching ? liveOf(run) : undefined;
  const busy = !!run?.members.some((m) => m.state === "working");
  // Asked, from the list, to follow a team while the planner's open
  const follow = useRef<(id: string) => void>(() => {});
  follow.current = (id) => {
    const r = runs.filter((x) => x.plan.id === id).at(-1);
    setEditing(null);
    if (plan?.id !== id && r) open(plans.find((p) => p.id === id) ?? r.plan);
  };
  useEffect(() => {
    const stop = bridge.onPlannerFollow((id) => follow.current(id));
    return () => void stop.then((f) => f());
  }, []);
  useEffect(() => {
    // Opened from a team's row in the list, on the plan it's following
    const follow = new URLSearchParams(location.search).get("follow");
    Promise.all([bridge.plans(), bridge.teams()]).then(([all, teams]) => {
      setPlans(all);
      const run = teams.filter((r) => r.plan.id === follow).at(-1);
      setPlan(all.find((p) => p.id === follow) ?? run?.plan ?? all[0] ?? blank());
    });
  }, []);

  const change = (next: Plan) => {
    setPlan(next);
    setSaved(false);
  };
  const store = async (p: Plan) => {
    const stamped = { ...p, updated: Date.now() };
    await bridge.savePlan(stamped);
    setPlans((all) => (all.some((x) => x.id === p.id) ? all.map((x) => (x.id === p.id ? stamped : x)) : [...all, stamped]));
    setPlan(stamped);
    setSaved(true);
  };
  const save = () => plan && store(plan);
  const saveAs = (name: string) => {
    if (!plan) return;
    store({ ...plan, id: newId(), name: name.trim() || plan.name });
    setSavingAs(null);
  };
  /** Go ahead with something that would replace what's open, once any unsaved changes are saved or let go. */
  const leave = (doing: string, then: () => void) => {
    setOpening(false);
    if (saved) return then();
    setUnsaved({ then, doing });
  };
  // Why an import or export didn't work, until the next one
  const [problem, setProblem] = useState("");

  // Planning with Claude (see Together.tsx): the plan shared with a chat, what each side's changed, and, for this
  // window, the version last written or taken from the file, the file as last read, and what Undo goes back to
  const [share, setShareNow] = useState<Share | null>(savedShare);
  const setShare = (next: Share | null) => {
    keepShare(next);
    setShareNow(next);
  };
  const [sharing, setSharing] = useState(false);
  const [said, setSaid] = useState<Said[]>([]);
  const [shareProblem, setShareProblem] = useState("");
  const shared = !!share && !!plan && share.plan === plan.id;
  const together = useRef<{ saved: Plan | null; file: string; history: Plan[]; undoing: boolean }>({
    saved: null,
    file: "",
    history: [],
    undoing: false,
  });
  const current = useRef(plan);
  current.current = plan;
  const tell = (who: Said["who"], list: Change[]) => {
    if (!list.length) return;
    const at = Date.now();
    setSaid((now) => [...list.map((c) => ({ who, text: c.text, at })).reverse(), ...now].slice(0, 40));
  };
  // Claude's changes, from the file, as they land
  useEffect(() => {
    if (!shared || !plan) return;
    together.current = { saved: plan, file: "", history: [], undoing: false };
    const check = async () => {
      const mine = current.current;
      if (!mine) return;
      const text = await bridge.planFile(mine.id);
      const t = together.current;
      if (text == null || text === t.file) return;
      t.file = text;
      let raw: unknown;
      try {
        raw = JSON.parse(text);
      } catch {
        return setShareProblem("Claude's last change couldn't be read. It'll show once it can be.");
      }
      const team = checkTeam(raw);
      if (typeof team === "string") return setShareProblem(`Claude's last change couldn't be read: ${team}`);
      setShareProblem("");
      const theirs: Plan = { ...mine, name: team.name || mine.name, agents: team.agents, edges: team.edges };
      // Ours, written a moment ago, or nothing that changes the team
      if (sameTeam(theirs, mine)) return;
      const unwritten = t.saved && !sameTeam(mine, t.saved);
      tell("claude", changes(mine, theirs));
      if (unwritten) tell("claude", [{ text: "changed it before your last edit was saved, so yours was set aside: Undo brings it back" }]);
      t.history.push(mine);
      t.saved = theirs;
      setPlan(theirs);
      setSaved(true);
    };
    check();
    const timer = setInterval(check, 1000);
    return () => clearInterval(timer);
  }, [shared, plan?.id]);
  // Ours, saved as they're made, for Claude to see
  useEffect(() => {
    const t = together.current;
    if (!shared || !plan || !t.saved || sameTeam(plan, t.saved)) return;
    const timer = setTimeout(() => {
      const before = t.saved!;
      if (t.undoing) tell("you", [{ text: "undid the last change" }]);
      else {
        t.history.push(before);
        tell("you", changes(before, plan));
      }
      t.undoing = false;
      t.saved = plan;
      store(plan);
    }, 500);
    return () => clearTimeout(timer);
  }, [plan, shared]);
  const undo = () => {
    const back = together.current.history.pop();
    if (!back || !plan) return;
    together.current.undoing = true;
    setPlan({ ...back, id: plan.id });
  };
  const startSharing = async () => {
    if (!plan) return;
    if (!saved) await store(plan);
    setSharing(true);
  };
  /** A team from a file: saved as one of this Mac's, and opened. */
  const importPlan = async () => {
    setOpening(false);
    setProblem("");
    const text = await chooseFile(".json,application/json");
    if (text === null) return;
    const read = readPlan(
      text,
      plans.map((p) => p.name),
    );
    if (typeof read === "string") return setProblem(read);
    leave(`importing ${read.name}`, () => {
      store(read);
      setSelected(null);
    });
  };
  const open = (p: Plan) =>
    leave(`opening ${p.name}`, () => {
      setPlan(p);
      setSaved(true);
      setSelected(null);
    });
  const fresh = () =>
    leave("starting a new team", () => {
      setPlan(blank());
      setSaved(false);
      setSelected(null);
    });

  // ⌫ takes away what's picked, unless it's being typed in; ⌘S saves, ⇧⌘S saves as a new version, ⌘N starts a new
  // team, and ⌘O opens one
  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e) => {
    const key = e.key.toLowerCase();
    if (e.metaKey && ["s", "n", "o"].includes(key)) {
      e.preventDefault();
      if (key === "s" && e.shiftKey && plan)
        setSavingAs(
          nextVersion(
            plan.name,
            plans.map((p) => p.name),
          ),
        );
      else if (key === "s") save();
      else if (key === "n") fresh();
      else setOpening(!opening);
      return;
    }
    // Following a team, the grid's the plan as it started, which isn't for editing
    if (watching || (e.target as Element).closest("input, textarea")) return;
    if (e.metaKey && key === "z" && shared) {
      e.preventDefault();
      undo();
    } else if (e.metaKey && key === "a" && plan) {
      e.preventDefault();
      setGroup(plan.agents.map((a) => a.id));
    } else if (e.key === "Backspace" || e.key === "Delete") {
      e.preventDefault();
      remove();
    }
  };
  // The agent or arrow that's picked, and an agent's arrows with it
  const remove = () => {
    const going = group.length ? group : selected ? [selected] : [];
    if (!plan || !going.length) return;
    const agents = plan.agents.filter((a) => !going.includes(a.id));
    const edges = plan.edges.filter((x) => !going.includes(edgeId(x)) && !going.includes(x.from) && !going.includes(x.to));
    change({ ...plan, agents, edges });
    setSelected(null);
  };
  useEffect(() => {
    const listener = (e: KeyboardEvent) => keys.current(e);
    window.addEventListener("keydown", listener);
    return () => window.removeEventListener("keydown", listener);
  }, []);

  if (!plan) return <div className="plan-window planner pop-vars" />;
  const issues = problems(plan);
  const counts = MODELS.map((m) => [m, plan.agents.filter((a) => a.model === m).length] as const)
    .filter(([, n]) => n)
    .map(([m, n]) => `${n} ${m}`)
    .join(" · ");
  const setAgent = (a: Agent) => {
    // One lead: making another the lead makes the old one a manager
    const agents = plan.agents.map((x) => (x.id === a.id ? a : a.role === "Lead" && x.role === "Lead" ? { ...x, role: "Manager" as Role } : x));
    change({ ...plan, agents });
  };
  const addAgent = () => {
    const y = Math.max(0, ...plan.agents.map((a) => a.y)) + (plan.agents.length ? 120 : 20);
    const a = agent(plan.agents.length ? "worker" : "lead", 40, y);
    change({ ...plan, agents: [...plan.agents, a] });
    setSelected(a.id);
  };

  return (
    <div className="plan-window planner pop-vars">
      <div className="plan-toolbar" data-tauri-drag-region>
        <input className="template-name" value={plan.name} onChange={(e) => change({ ...plan, name: e.target.value })} title="Rename it" />
        <span className="plan-sub">
          {[
            shared ? "Saves as you go" : saved ? plan.updated && `Saved ${ago(plan.updated)}` : "Edited",
            `${plan.agents.length} agents`,
            counts,
          ]
            .filter(Boolean)
            .join(" · ")}
        </span>
        {shared && share && (
          <span className="share-pill">
            {share.icon && <Avatar icon={share.icon} size={18} />}
            Planning with <b>{share.title}</b>
            <button onClick={() => setShare(null)} title="Stop planning this with Claude">
              Stop
            </button>
          </span>
        )}
        <span className="ask-spacer" />
        <button className="ask-btn ghost" onClick={fresh} title="⌘N">
          New
        </button>
        <div className="template">
          <button className="ask-btn ghost" onClick={() => setOpening(!opening)} title="⌘O">
            Open <span className="menu-caret">▾</span>
          </button>
          {opening && (
            <div className="template-list" onPointerLeave={() => (setOpening(false), setDeleting(null))}>
              {plans.length === 0 && <div className="template-none">No saved teams yet</div>}
              {[...plans]
                .sort((a, b) => (b.updated ?? 0) - (a.updated ?? 0))
                .map((p) => (
                  <div key={p.id} className={p.id === plan.id ? "template-row on" : "template-row"}>
                    <button className="template-open" onClick={() => open(p)}>
                      <span>{p.name}</span>
                      <span className="template-when">
                        {p.agents.length} agents{p.updated ? ` · ${ago(p.updated)}` : ""}
                      </span>
                    </button>
                    {deleting === p.id ? (
                      <button
                        className="template-delete sure"
                        onClick={async () => {
                          await bridge.deletePlan(p.id);
                          setPlans((all) => all.filter((x) => x.id !== p.id));
                          setDeleting(null);
                          if (p.id === plan.id) setSaved(false);
                        }}
                      >
                        Delete
                      </button>
                    ) : (
                      <button className="template-delete" title={`Delete ${p.name}`} onClick={() => setDeleting(p.id)}>
                        ×
                      </button>
                    )}
                  </div>
                ))}
              <hr />
              <button onClick={importPlan}>Import…</button>
              <button
                onClick={() => {
                  setOpening(false);
                  bridge
                    .exportPlan(plan.name, exportText(plan))
                    .catch((e) => setProblem(`Couldn't export it: ${e instanceof Error ? e.message : String(e)}`));
                }}
              >
                Export {plan.name}…
              </button>
            </div>
          )}
        </div>
        <button className="ask-btn ghost" disabled={saved} onClick={save} title="⌘S">
          Save
        </button>
        <button
          className="ask-btn ghost"
          onClick={() =>
            setSavingAs(
              nextVersion(
                plan.name,
                plans.map((p) => p.name),
              ),
            )
          }
          title="⇧⌘S"
        >
          Save As…
        </button>
        <span className="toolbar-gap" />
        {!shared && (
          <button className="ask-btn ghost" disabled={watching} onClick={startSharing} title="Plan this team with Claude, in a chat">
            Plan with Claude…
          </button>
        )}
        <button className="ask-btn ghost" disabled={watching} onClick={addAgent}>
          + Agent
        </button>
        <button
          className="ask-btn primary"
          disabled={(issues.length > 0 && !plan.agents.some((a) => a.role === "Lead")) || busy}
          title={busy ? "This team's already at work. Stop it first to start it again." : undefined}
          onClick={() => setStarting(true)}
        >
          Add to a Chat…
        </button>
      </div>
      {run && <RunBar run={run} watching={watching} onWatch={(watch) => setEditing(watch ? null : run.id)} onStopped={loadRuns} />}
      {runs
        .filter((r) => r.id !== run?.id && r.members.some((m) => m.state === "working"))
        .map((r) => (
          <div key={r.id} className="plan-running">
            <span className="plan-dot working" />
            <b>{r.plan.name} is at work</b>
            <span>on {r.leadState?.title ?? "its chat"}</span>
            <span className="ask-spacer" />
            <button className="ask-btn ghost" onClick={() => open(plans.find((p) => p.id === r.plan.id) ?? r.plan)}>
              Follow the Team
            </button>
          </div>
        ))}
      {problem && <div className="plan-issues">{problem}</div>}
      {issues.length > 0 && !watching && <div className="plan-issues">{issues.join(" ")}</div>}
      {run && live ? (
        <div className="plan-body">
          <Canvas agents={run.plan.agents} edges={run.plan.edges} selected={selected} onSelect={setSelected} live={live.live} rounds={live.rounds} />
          <LivePanel run={run} selected={selected} />
        </div>
      ) : (
        <div className="plan-body">
          <Canvas
            agents={plan.agents}
            edges={plan.edges}
            selected={selected}
            group={group}
            onSelect={setSelected}
            onGroup={setGroup}
            onMove={(moves) => change({ ...plan, agents: plan.agents.map((a) => (moves[a.id] ? { ...a, ...moves[a.id] } : a)) })}
            onConnect={(from, to) => {
              if (plan.edges.some((e) => e.from === from && e.to === to) || makesCircle(plan.edges, from, to)) return;
              change({ ...plan, edges: [...plan.edges, { from, to }] });
            }}
          />
          {shared && share && !group.length && !selected ? (
            <TogetherPanel share={share} said={said} problem={shareProblem} canUndo={together.current.history.length > 0} onUndo={undo} />
          ) : group.length ? (
            <GroupInspector
              agents={plan.agents.filter((a) => group.includes(a.id))}
              onAgents={(changed) => change({ ...plan, agents: plan.agents.map((a) => changed.find((c) => c.id === a.id) ?? a) })}
              onDelete={remove}
            />
          ) : (
            <Inspector
              plan={plan}
              selected={selected}
              onAgent={setAgent}
              onEdge={(e) => change({ ...plan, edges: plan.edges.map((x) => (edgeId(x) === edgeId(e) ? e : x)) })}
              onDelete={remove}
            />
          )}
        </div>
      )}
      {starting && <StartSheet plan={plan} onClose={() => setStarting(false)} />}
      {sharing && (
        <ShareSheet
          plan={plan}
          onClose={() => setSharing(false)}
          onShared={(next) => {
            setShare(next);
            setSaid([]);
            setSharing(false);
            setSelected(null);
          }}
        />
      )}
      {savingAs !== null && (
        <div className="sheet-backdrop" onPointerDown={(e) => e.target === e.currentTarget && setSavingAs(null)}>
          <div className="sheet small">
            <div className="inspector-title">Save as a new version</div>
            <div className="inspector-hint">{plan.name} stays as it was last saved.</div>
            <input
              className="inspector-field sheet-name"
              autoFocus
              value={savingAs}
              onChange={(e) => setSavingAs(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && saveAs(savingAs)}
            />
            <div className="plan-foot">
              <span className="ask-spacer" />
              <button className="ask-btn ghost" onClick={() => setSavingAs(null)}>
                Cancel
              </button>
              <button className="ask-btn primary" onClick={() => saveAs(savingAs)}>
                Save
              </button>
            </div>
          </div>
        </div>
      )}
      {unsaved && (
        <div className="sheet-backdrop">
          <div className="sheet small">
            <div className="inspector-title">Save the changes to {plan.name}?</div>
            <div className="inspector-hint">They'll be lost, {unsaved.doing}, if they aren't saved.</div>
            <div className="plan-foot">
              <button
                className="ask-btn ghost"
                onClick={() => {
                  unsaved.then();
                  setUnsaved(null);
                }}
              >
                Don't Save
              </button>
              <span className="ask-spacer" />
              <button className="ask-btn ghost" onClick={() => setUnsaved(null)}>
                Cancel
              </button>
              <button
                className="ask-btn primary"
                onClick={async () => {
                  await save();
                  unsaved.then();
                  setUnsaved(null);
                }}
              >
                Save
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/** A new agent: a team's first is its lead. */
function agent(kind: "lead" | "worker", x: number, y: number): Agent {
  const lead = kind === "lead";
  return {
    id: newId(),
    name: lead ? "Lead" : "New agent",
    icon: { emoji: lead ? "🧭" : "🐝" },
    model: lead ? "Opus" : "Sonnet",
    role: lead ? "Lead" : "Worker",
    x,
    y,
    brief: "",
    commands: [],
    tools: ["Read files", "Edit files"],
  };
}

/** A new team, with just its lead to start from. */
const blank = (): Plan => ({ id: newId(), name: "New team", agents: [agent("lead", 460, 40)], edges: [] });
