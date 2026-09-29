// Plans that come from somewhere else: checked over so only what the planner knows goes in, and what changed from one
// version to the next, in words. For a team imported from a file, a plan Claude changes while planning with the user
// (see Together.tsx), and changes for a team at work (see Running.tsx).

import type { ChatIcon } from "./Avatar";
import { H, MODELS, TOOLS, W, type Agent, type Edge, type Model, type Plan, type Role } from "./Planner";

/** A team's name, agents, and arrows, from something that claims to be one, or why it can't be read. Agents it
 *  couldn't place, or put on top of another, go under the agent that hands them work. */
export function checkTeam(raw: unknown): { name: string; agents: Agent[]; edges: Edge[] } | string {
  const unreadable = "That isn't a team Headroom can read.";
  const p = raw as { name?: unknown; agents?: unknown; edges?: unknown } | null;
  if (!p || !Array.isArray(p.agents) || !Array.isArray(p.edges) || !p.agents.length) return unreadable;
  const words = (v: unknown, max = 4000) => (typeof v === "string" ? v.slice(0, max) : "");
  const number = (v: unknown) => (typeof v === "number" && isFinite(v) ? Math.max(0, Math.min(20000, Math.round(v))) : null);
  const icon = (v: unknown): ChatIcon => {
    const i = v as { emoji?: unknown; image?: unknown };
    if (typeof i?.emoji === "string" && i.emoji.length <= 16) return { emoji: i.emoji };
    if (typeof i?.image === "string" && i.image.startsWith("data:image/") && i.image.length < 300_000) return { image: i.image };
    return { emoji: "🐝" };
  };
  const agents: Agent[] = [];
  const unplaced = new Set<string>();
  for (const raw of p.agents as Record<string, unknown>[]) {
    const role = raw?.role as Role;
    const model = raw?.model as Model;
    if (typeof raw?.id !== "string" || !(["Lead", "Manager", "Worker"] as Role[]).includes(role) || !MODELS.includes(model)) {
      return "One of the team's agents isn't set up in a way Headroom can read.";
    }
    if (agents.some((a) => a.id === raw.id)) return "Two of the team's agents are the same one.";
    const [x, y] = [number(raw.x), number(raw.y)];
    if (x === null || y === null) unplaced.add(raw.id);
    agents.push({
      id: raw.id,
      name: words(raw.name, 80) || "Agent",
      icon: icon(raw.icon),
      model,
      role,
      x: x ?? 0,
      y: y ?? 0,
      brief: words(raw.brief),
      commands: Array.isArray(raw.commands) ? raw.commands.filter((c): c is string => typeof c === "string").map((c) => c.slice(0, 80)) : [],
      tools: Array.isArray(raw.tools) ? TOOLS.filter((t) => (raw.tools as unknown[]).includes(t)) : [],
      ...(typeof raw.until === "string" ? { until: words(raw.until, 200) } : {}),
    });
  }
  const known = (id: unknown) => agents.some((a) => a.id === id);
  const edges: Edge[] = [];
  for (const raw of p.edges as Record<string, unknown>[]) {
    if (!known(raw?.from) || !known(raw?.to) || raw.from === raw.to) continue;
    if (edges.some((e) => e.from === raw.from && e.to === raw.to)) continue;
    const loop = raw.loop as { until?: unknown; rounds?: unknown } | undefined;
    const rounds = typeof loop?.rounds === "number" ? Math.max(1, Math.min(10, Math.round(loop.rounds))) : 0;
    edges.push({
      from: raw.from as string,
      to: raw.to as string,
      ...(rounds ? { loop: { until: words(loop!.until, 200) || "approved", rounds } } : {}),
    });
  }
  return { name: words(p.name, 80).trim(), agents: place(agents, edges, unplaced), edges };
}

/** Agents that weren't placed, or were put on top of one before them, moved under whoever hands them work, and along
 *  until there's room. */
function place(agents: Agent[], edges: Edge[], unplaced: Set<string>): Agent[] {
  const out: Agent[] = [];
  const overlaps = (a: Agent) => out.some((b) => Math.abs(a.x - b.x) < W && Math.abs(a.y - b.y) < H);
  for (const agent of agents) {
    let a = agent;
    if (unplaced.has(a.id) || overlaps(a)) {
      const parent = out.find((b) => edges.some((e) => e.from === b.id && e.to === a.id && !e.loop));
      a = parent ? { ...a, x: parent.x, y: parent.y + 130 } : { ...a, y: Math.max(a.y, 40) };
      for (let tries = 0; overlaps(a) && tries < 50; tries++) a = { ...a, x: a.x + 180 };
    }
    out.push(a);
  }
  return out;
}

const TOOL_WORDS: Record<string, string> = {
  "Read files": "read files",
  "Edit files": "edit files",
  "Run commands": "run commands",
  "Browse the web": "browse the web",
};

/** One thing that changed: the agent it's about, if it's about one, and what happened, in words that follow "Claude"
 *  or "You", like "added Tester under Coder". */
export type Change = { agent?: string; text: string };

/** What changed from `before` to `after`. Moving agents around the grid isn't counted: it changes nothing a team
 *  does. */
export function changes(before: Plan, after: Plan): Change[] {
  const out: Change[] = [];
  const was = (id: string) => before.agents.find((a) => a.id === id);
  const now = (id: string) => after.agents.find((a) => a.id === id);
  const name = (id: string) => now(id)?.name ?? was(id)?.name ?? "an agent";
  if (before.name !== after.name) out.push({ text: `renamed the team ${after.name}` });
  for (const a of after.agents) {
    const b = was(a.id);
    if (!b) {
      const parent = after.edges.find((e) => e.to === a.id && !e.loop);
      out.push({ agent: a.id, text: `added ${a.name}${parent ? ` under ${name(parent.from)}` : ""}, on ${a.model}` });
      continue;
    }
    if (b.name !== a.name) out.push({ agent: a.id, text: `renamed ${b.name} ${a.name}` });
    if (b.role !== a.role) out.push({ agent: a.id, text: `made ${a.name} ${a.role === "Worker" ? "a Worker" : `the ${a.role}`}` });
    if (b.model !== a.model) out.push({ agent: a.id, text: `put ${a.name} on ${a.model}` });
    if (b.brief.trim() !== a.brief.trim()) out.push({ agent: a.id, text: `changed ${a.name}'s standing instructions` });
    const gained = a.tools.filter((t) => !b.tools.includes(t)).map((t) => TOOL_WORDS[t]);
    const lost = b.tools.filter((t) => !a.tools.includes(t)).map((t) => TOOL_WORDS[t]);
    if (gained.length) out.push({ agent: a.id, text: `let ${a.name} ${gained.join(" and ")}` });
    if (lost.length) out.push({ agent: a.id, text: `stopped ${a.name} being able to ${lost.join(" or ")}` });
    const added = a.commands.filter((c) => !b.commands.includes(c));
    const taken = b.commands.filter((c) => !a.commands.includes(c));
    if (added.length) out.push({ agent: a.id, text: `gave ${a.name} ${added.join(" and ")}` });
    if (taken.length) out.push({ agent: a.id, text: `took ${taken.join(" and ")} from ${a.name}` });
    if (b.until !== a.until) {
      out.push({ agent: a.id, text: a.until === undefined ? `stopped ${a.name} keeping going` : `had ${a.name} keep going until ${a.until}` });
    }
    if (b.icon && a.icon && JSON.stringify(b.icon) !== JSON.stringify(a.icon)) out.push({ agent: a.id, text: `changed ${a.name}'s icon` });
  }
  for (const b of before.agents) if (!now(b.id)) out.push({ agent: b.id, text: `removed ${b.name}` });
  const same = (x: Edge, y: Edge) => x.from === y.from && x.to === y.to;
  for (const e of after.edges) {
    const old = before.edges.find((x) => same(x, e));
    const loop = e.loop ? `${name(e.to)} sends ${name(e.from)}'s work back until ${e.loop.until}, at most ${e.loop.rounds} rounds` : "";
    // An agent that's new comes with its arrows, which its "added" says enough about
    if (!old && was(e.from) && was(e.to))
      out.push({ agent: e.to, text: `had ${name(e.from)} hand work to ${name(e.to)}${loop ? `, and ${loop}` : ""}` });
    else if (old && !old.loop && e.loop) out.push({ agent: e.to, text: `made a loop: ${loop}` });
    else if (old?.loop && !e.loop) out.push({ agent: e.to, text: `took away the loop from ${name(e.to)} back to ${name(e.from)}` });
    else if (old?.loop && e.loop && (old.loop.until !== e.loop.until || old.loop.rounds !== e.loop.rounds)) {
      out.push({ agent: e.to, text: `changed the loop: ${loop}` });
    }
  }
  for (const e of before.edges) {
    if (!after.edges.some((x) => same(x, e)) && now(e.from) && now(e.to)) {
      out.push({ agent: e.to, text: `took away the arrow from ${name(e.from)} to ${name(e.to)}` });
    }
  }
  return out;
}
