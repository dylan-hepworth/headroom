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
import { bridge } from "./bridge";
import type { Pending } from "./Pending";
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
export type Plan = { id: string; name: string; agents: Agent[]; edges: Edge[] };

const GRID = 20;
export const W = 160;
export const H = 64;
export const MODELS: Model[] = ["Opus", "Sonnet", "Haiku"];
const TOOLS = ["Read files", "Edit files", "Run commands", "Browse the web"];
const snap = (v: number) => Math.round(v / GRID) * GRID;
const edgeId = (e: Edge) => `${e.from}>${e.to}`;

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
const list = (xs: string[]) => (xs.length < 2 ? xs.join("") : `${xs.slice(0, -1).join(", ")} and ${xs[xs.length - 1]}`);

/** The plan in words, for the chat to run. Workers come in the order their work flows: each one after the workers that
 *  hand it theirs. */
export function verbalize(plan: Plan): string {
  const lead = plan.agents.find((a) => a.role === "Lead");
  if (!lead) return "";
  const byId = (id: string) => plan.agents.find((a) => a.id === id)!;
  const lines: string[] = [
    `Work through this with a team, the way I've planned it. You're the ${lead.name}${lead.brief.trim() ? `: ${clause(lead.brief)}` : ""}.`,
    `Run each agent below as a subagent of your own, with the model it's given, and use its name as the subagent's description, so I can follow each one. Where an agent waits for others, start it once they're done.`,
  ];
  const say = (a: Agent) => {
    const waits = plan.edges.filter((e) => e.to === a.id && !e.loop && byId(e.from).role === "Worker").map((e) => byId(e.from).name);
    const parts = [`- ${a.name} (${a.model})${a.brief.trim() ? `: ${clause(a.brief)}` : ""}.`];
    if (waits.length) parts.push(`It starts once ${list(waits)} ${waits.length === 1 ? "is" : "are"} done.`);
    for (const e of plan.edges.filter((e) => e.to === a.id && e.loop)) {
      parts.push(
        `It reviews ${byId(e.from).name}'s work: send it back to ${byId(e.from).name} until ${clause(e.loop!.until)}, ${e.loop!.rounds} rounds at most.`,
      );
    }
    if (a.until) parts.push(`It keeps going until ${clause(a.until)}.`);
    if (a.commands.length) parts.push(`It may use ${list(a.commands)}.`);
    if (a.tools.length && a.tools.length < TOOLS.length) parts.push(`It should only ${list(a.tools.map(lower))}.`);
    lines.push(parts.join(" "));
  };
  // Workers in flow order, grouped under the manager whose team they're in
  const workers = plan.agents.filter((a) => a.role === "Worker");
  const placed: Agent[] = [];
  const place = (a: Agent, path = new Set<string>()) => {
    if (placed.includes(a) || path.has(a.id)) return;
    path.add(a.id);
    plan.edges.filter((e) => e.to === a.id && !e.loop && byId(e.from).role === "Worker").forEach((e) => place(byId(e.from), path));
    placed.push(a);
  };
  workers.forEach((a) => place(a));
  const managers = plan.agents.filter((a) => a.role === "Manager");
  for (const m of managers) {
    // One in more than one team goes with the first (the planner says so)
    const team = placed.filter((a) => teamOf(plan, a.id)[0] === m.id);
    if (!team.length) continue;
    lines.push(
      "",
      `${m.name} team${m.brief.trim() ? `, with this in mind: ${clause(m.brief)}.` : ":"}${m.commands.length ? ` It may use ${list(m.commands)}.` : ""}`,
    );
    team.forEach(say);
  }
  const loose = placed.filter((a) => teamOf(plan, a.id).length === 0);
  if (loose.length) {
    lines.push("", managers.length ? "Reporting to you directly:" : "The team:");
    loose.forEach(say);
  }
  lines.push("", "Bring their work together when they're done, and tell me what each one did.");
  return lines.join("\n");
}

export function Canvas({
  agents,
  edges,
  selected,
  onSelect,
  onMove,
  onConnect,
  live,
}: {
  agents: Agent[];
  edges: Edge[];
  selected: string | null;
  onSelect: (id: string | null) => void;
  onMove?: (id: string, x: number, y: number) => void;
  onConnect?: (from: string, to: string) => void;
  /** While it runs, how each agent's getting on (see PlanPreview.tsx) */
  live?: Record<string, { status: string; doing: string }>;
}) {
  const box = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<{ id: string; dx: number; dy: number } | null>(null);
  const [wire, setWire] = useState<{ from: string; x: number; y: number } | null>(null);
  const at = (e: React.PointerEvent) => {
    const r = box.current!.getBoundingClientRect();
    return { x: e.clientX - r.left + box.current!.scrollLeft, y: e.clientY - r.top + box.current!.scrollTop };
  };
  const byId = (id: string) => agents.find((n) => n.id === id);
  const width = Math.max(...agents.map((a) => a.x + W), 0) + 200;
  const height = Math.max(...agents.map((a) => a.y + H), 0) + 200;
  return (
    <div
      className="grid-canvas"
      ref={box}
      onPointerMove={(e) => {
        const p = at(e);
        if (drag && onMove) onMove(drag.id, Math.max(0, snap(p.x - drag.dx)), Math.max(0, snap(p.y - drag.dy)));
        if (wire) setWire({ ...wire, ...p });
      }}
      onPointerUp={(e) => {
        if (wire && onConnect) {
          const p = at(e);
          const onto = agents.find((n) => p.x >= n.x && p.x <= n.x + W && p.y >= n.y && p.y <= n.y + H);
          if (onto && onto.id !== wire.from) onConnect(wire.from, onto.id);
        }
        setDrag(null);
        setWire(null);
      }}
      onPointerDown={(e) => (e.target === e.currentTarget || (e.target as Element).classList.contains("grid-wires")) && onSelect(null)}
    >
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
            <g key={edgeId(e)} className={selected === edgeId(e) ? "wire selected" : "wire"} onPointerDown={() => onSelect(edgeId(e))}>
              <path className="wire-hit" d={path(a, b)} />
              <path d={path(a, b)} markerEnd="url(#arrow)" />
              {e.loop && (
                <>
                  <path className="wire-loop" d={loopPath(a, b)} markerEnd="url(#arrow)" />
                  <foreignObject x={Math.min(a.x, b.x) - 250} y={(a.y + b.y) / 2 + H / 2 - 12} width={200} height={30}>
                    <div className="loop-tag">{live ? `↻ round 2 of ${e.loop.rounds}` : `↻ until ${e.loop.until} · max ${e.loop.rounds}`}</div>
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
          className={["grid-node", n.role.toLowerCase(), live?.[n.id]?.status, selected === n.id && "selected"].filter(Boolean).join(" ")}
          style={{ left: n.x, top: n.y, width: W, height: H }}
          onPointerDown={(e) => {
            const p = at(e);
            if (onMove) setDrag({ id: n.id, dx: p.x - n.x, dy: p.y - n.y });
            onSelect(n.id);
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
                e.stopPropagation();
                setWire({ from: n.id, ...at(e) });
              }}
            />
          )}
        </div>
      ))}
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
            ? "Runs its team. For now the Lead does, with what's written here in mind."
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
      <textarea
        className="inspector-field"
        rows={3}
        value={a.brief}
        placeholder="What it's for, and what it always keeps in mind"
        onChange={(ev) => onAgent({ ...a, brief: ev.target.value })}
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
      <span className="inspector-delete-room" />
      <button className="ask-btn delete" onClick={onDelete} title="Its arrows go with it">
        Delete Agent <kbd>⌫</kbd>
      </button>
    </div>
  );
}

/** Where the plan goes: a chat that can take it now (at work, or finished with its turn held open for a reply), after
 *  a look at exactly what it'll be told. Any other chat gets it copied, to paste in there. */
function StartSheet({ plan, onClose }: { plan: Plan; onClose: () => void }) {
  const [chats, setChats] = useState<Pending[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);
  const [problem, setProblem] = useState("");
  const [done, setDone] = useState("");
  const text = verbalize(plan);
  useEffect(() => {
    bridge.pendingSessions().then(setChats);
  }, []);
  const takes = (c: Pending) => c.state === "working" || !!c.replyId;
  const chat = chats.find((c) => c.id === chosen);
  const send = async () => {
    if (!chat) return;
    setProblem("");
    try {
      if (takes(chat)) {
        await bridge.sendToSession(chat.id, text);
        setDone(`Sent to ${chat.title}.`);
      } else {
        await navigator.clipboard.writeText(text);
        await bridge.openPending(chat.id);
        setDone(`Copied. Paste it into ${chat.title}.`);
      }
    } catch (e) {
      setProblem(`Couldn't send it: ${e instanceof Error ? e.message : String(e)}`);
    }
  };
  return (
    <div className="sheet-backdrop" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="sheet">
        <div className="inspector-title">Start {plan.name} on a chat</div>
        <div className="inspector-hint">Describe the work in the chat first. This tells its Claude how to split it up.</div>
        <div className="inspector-label">Chat</div>
        <div className="sheet-chats">
          {chats.length === 0 && <div className="inspector-hint">No chats are open. Start one in Claude or a terminal first.</div>}
          {chats.map((c) => (
            <button key={c.id} className={chosen === c.id ? "sheet-chat on" : "sheet-chat"} onClick={() => setChosen(c.id)}>
              {c.icon && <Avatar icon={c.icon} size={22} />}
              <span>
                <b>{c.title}</b> <span className="inspector-hint">{c.project}</span>
              </span>
              <span className="sheet-how">{takes(c) ? (c.state === "working" ? "After its current step" : "As your reply") : "Copy and paste"}</span>
            </button>
          ))}
        </div>
        <div className="inspector-label">What it'll be told</div>
        <pre className="sheet-text">{text}</pre>
        {problem && <div className="ask-problem">{problem}</div>}
        {done && <div className="pending-sent">{done}</div>}
        <div className="plan-foot">
          <span className="ask-spacer" />
          <button className="ask-btn ghost" onClick={onClose}>
            {done ? "Done" : "Cancel"}
          </button>
          {!done && (
            <button className="ask-btn primary" disabled={!chat} onClick={send}>
              {chat && !takes(chat) ? "Copy and Open" : "Send"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

let made = 0;
const newId = () => `a${Date.now().toString(36)}${made++}`;

/** The planner window. */
export function PlannerWindow() {
  const [plans, setPlans] = useState<Plan[]>([]);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [saved, setSaved] = useState(true);
  const [selected, setSelected] = useState<string | null>(null);
  const [menu, setMenu] = useState(false);
  const [starting, setStarting] = useState(false);
  useEffect(() => {
    bridge.plans().then((all) => {
      setPlans(all);
      setPlan(all[0] ?? blank());
    });
  }, []);

  const change = (next: Plan) => {
    setPlan(next);
    setSaved(false);
  };
  const save = async () => {
    if (!plan) return;
    await bridge.savePlan(plan);
    setPlans((all) => (all.some((p) => p.id === plan.id) ? all.map((p) => (p.id === plan.id ? plan : p)) : [...all, plan]));
    setSaved(true);
  };

  // ⌫ takes away what's picked, unless it's being typed in; ⌘S saves
  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e) => {
    if ((e.metaKey && e.key === "s") || (e.metaKey && e.key === "S")) {
      e.preventDefault();
      save();
      return;
    }
    if (e.key !== "Backspace" && e.key !== "Delete") return;
    if ((e.target as Element).closest("input, textarea")) return;
    remove();
  };
  // The agent or arrow that's picked, and an agent's arrows with it
  const remove = () => {
    if (!plan || !selected) return;
    const agents = plan.agents.filter((a) => a.id !== selected);
    const edges = plan.edges.filter((x) => edgeId(x) !== selected && x.from !== selected && x.to !== selected);
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
    const a: Agent = {
      id: newId(),
      name: plan.agents.length ? "New agent" : "Lead",
      icon: { emoji: plan.agents.length ? "🐝" : "🧭" },
      model: plan.agents.length ? "Sonnet" : "Opus",
      role: plan.agents.length ? "Worker" : "Lead",
      x: 40,
      y,
      brief: "",
      commands: [],
      tools: ["Read files", "Edit files"],
    };
    change({ ...plan, agents: [...plan.agents, a] });
    setSelected(a.id);
  };

  return (
    <div className="plan-window planner pop-vars">
      <div className="plan-toolbar" data-tauri-drag-region>
        <div className="template">
          <button className="template-menu" onClick={() => setMenu(!menu)}>
            {plan.name} <span>▾</span>
          </button>
          {menu && (
            <div className="template-list" onPointerLeave={() => setMenu(false)}>
              {plans.map((p) => (
                <button
                  key={p.id}
                  onClick={() => {
                    setPlan(p);
                    setSaved(true);
                    setSelected(null);
                    setMenu(false);
                  }}
                >
                  {p.name}
                </button>
              ))}
              <hr />
              <button
                onClick={() => {
                  change(blank());
                  setSelected(null);
                  setMenu(false);
                }}
              >
                New Template
              </button>
              <button
                onClick={() => {
                  change({ ...plan, id: newId(), name: `${plan.name} copy` });
                  setMenu(false);
                }}
              >
                Duplicate
              </button>
              {plans.some((p) => p.id === plan.id) && (
                <button
                  className="danger"
                  onClick={async () => {
                    await bridge.deletePlan(plan.id);
                    const rest = plans.filter((p) => p.id !== plan.id);
                    setPlans(rest);
                    setPlan(rest[0] ?? blank());
                    setSaved(true);
                    setMenu(false);
                  }}
                >
                  Delete {plan.name}
                </button>
              )}
            </div>
          )}
        </div>
        <input className="template-name" value={plan.name} onChange={(e) => change({ ...plan, name: e.target.value })} title="Rename it" />
        <span className="plan-sub">
          {plan.agents.length} agents{counts && ` · ${counts}`}
        </span>
        <span className="ask-spacer" />
        <button className="ask-btn ghost" onClick={addAgent}>
          + Agent
        </button>
        <button className="ask-btn ghost" disabled={saved} onClick={save}>
          {saved ? "Saved" : "Save"}
        </button>
        <button
          className="ask-btn primary"
          disabled={issues.length > 0 && !plan.agents.some((a) => a.role === "Lead")}
          onClick={() => setStarting(true)}
        >
          Start on a Chat…
        </button>
      </div>
      {issues.length > 0 && <div className="plan-issues">{issues.join(" ")}</div>}
      <div className="plan-body">
        <Canvas
          agents={plan.agents}
          edges={plan.edges}
          selected={selected}
          onSelect={setSelected}
          onMove={(id, x, y) => change({ ...plan, agents: plan.agents.map((a) => (a.id === id ? { ...a, x, y } : a)) })}
          onConnect={(from, to) => {
            if (plan.edges.some((e) => e.from === from && e.to === to) || makesCircle(plan.edges, from, to)) return;
            change({ ...plan, edges: [...plan.edges, { from, to }] });
          }}
        />
        <Inspector
          plan={plan}
          selected={selected}
          onAgent={setAgent}
          onEdge={(e) => change({ ...plan, edges: plan.edges.map((x) => (edgeId(x) === edgeId(e) ? e : x)) })}
          onDelete={remove}
        />
      </div>
      {starting && <StartSheet plan={plan} onClose={() => setStarting(false)} />}
    </div>
  );
}

const blank = (): Plan => ({ id: newId(), name: "New template", agents: [], edges: [] });
