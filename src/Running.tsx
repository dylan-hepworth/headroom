// A team at work, in the planner: the plan it was started from, with how each agent's getting on, from what the
// lead's chat and the managers' hooks tell Headroom (see sessions.rs), and a word for the lead or a manager.
//
// A worker runs as its manager's subagent. Claude Code tells the hooks what it does and when it's done, but not when
// it's handed its work, so a worker shows as started from its first step. The lead's own workers are subagents of
// the user's chat, which Headroom doesn't follow one by one.

import { useEffect, useState } from "react";
import { Avatar } from "./Avatar";
import { bridge, type TeamRun } from "./bridge";
import { Markdown } from "./markdown";
import { edgeId, list, teamFor, teamOf, type Plan } from "./Planner";
import { SpokenField } from "./Voice";

export type Status = "working" | "needs-you" | "done" | "waiting" | "stopped" | "failed";
const STATUS: Record<Status, string> = {
  working: "Working",
  "needs-you": "Needs you",
  done: "Done",
  waiting: "Waiting",
  stopped: "Stopped",
  failed: "Stopped",
};

type Member = TeamRun["members"][number];

/** How one agent's getting on: its status, a line for the grid, and for the panel, what it said and did lately. */
export type AgentLive = {
  status: Status;
  doing: string;
  /** Its report, or what it said back when it was done */
  said?: string;
  steps?: string[];
  /** The lead's chat, a manager's session, a manager's worker, one of the lead's own, or not in this run */
  kind: "lead" | "manager" | "worker" | "lead-worker" | "idle";
  /** For a manager, itself; for a worker, the manager it works for */
  member?: Member;
};

/** The first line of what an agent said, without its markdown, for the grid. */
const firstLine = (text?: string | null) =>
  (text ?? "")
    .split("\n")
    .find((l) => l.trim())
    ?.replace(/^[#>*\-\s]+/, "")
    .replace(/[`*_]/g, "") ?? "";

/** Each agent's part in the run, and how many rounds each loop's gone. */
export function liveOf(run: TeamRun): { live: Record<string, AgentLive>; rounds: Record<string, number> } {
  const plan = run.plan;
  const name = (id: string) => plan.agents.find((a) => a.id === id)?.name ?? id;
  const live: Record<string, AgentLive> = {};
  const finished: Record<string, number> = {};
  const working: Record<string, boolean> = {};

  for (const m of run.members) {
    const heard = m.live?.workers ?? {};
    const workers = Object.entries(m.workers).map(([key, id]) => [id, heard[key]] as const);
    const busy = workers.filter(([, w]) => w?.working).map(([id]) => name(id));
    live[m.agent] = {
      kind: "manager",
      member: m,
      status: m.state,
      doing:
        m.state === "working"
          ? busy.length
            ? `Waiting on ${list(busy)}`
            : (m.live?.activity ?? "Getting started")
          : m.state === "done"
            ? firstLine(m.text) || "Done"
            : m.state === "failed"
              ? m.text || "Stopped before it was done"
              : "Stopped",
      said: m.state === "done" ? m.text : undefined,
    };
    for (const [id, w] of workers) {
      finished[id] = w?.finished ?? 0;
      working[id] = !!w?.working && m.state === "working";
      const base = { kind: "worker" as const, member: m, steps: w?.steps, said: w?.said ?? undefined };
      live[id] = !w
        ? { ...base, status: m.state === "working" ? "waiting" : "stopped", doing: m.state === "working" ? "Not started yet" : "Didn't start" }
        : working[id]
          ? { ...base, status: "working", doing: w.activity ?? "Getting started" }
          : w.finished
            ? { ...base, status: "done", doing: firstLine(w.said) || "Done" }
            : { ...base, status: "stopped", doing: w.activity ?? "Stopped" };
    }
  }

  // Workers given to a manager while it worked: it runs them as general-purpose subagents, which Headroom can't tell
  // apart, so they show how their manager's getting on
  for (const a of plan.agents) {
    const m = a.role === "Worker" && !live[a.id] ? run.members.find((x) => x.agent === teamOf(plan, a.id)[0]) : undefined;
    if (m) live[a.id] = { kind: "worker", member: m, status: m.state, doing: `Added while it worked: ${m.name} runs it` };
  }

  const lead = plan.agents.find((a) => a.role === "Lead");
  if (lead) {
    const busy = run.members.filter((m) => m.state === "working").map((m) => m.name);
    const chat = run.leadState;
    const status: Status = chat?.state === "needs-you" ? "needs-you" : chat?.state === "working" ? "working" : busy.length ? "waiting" : "done";
    const doing =
      status === "needs-you"
        ? `Asks: ${chat?.doing ?? "something"}`
        : status === "working"
          ? (chat?.doing ?? "Working")
          : status === "waiting"
            ? `Waiting on ${list(busy)}`
            : "Done";
    live[lead.id] = { kind: "lead", status, doing, said: status === "done" ? (chat?.doing ?? undefined) : undefined };
    for (const a of teamFor(plan, null)) {
      live[a.id] = { kind: "lead-worker", status: status === "working" ? "working" : "waiting", doing: "Part of the Lead's own work" };
    }
  }
  for (const a of plan.agents) live[a.id] ??= { kind: "idle", status: "waiting", doing: "Not part of this run" };

  // A loop's round is how many times the one it sends back to has been at it
  const rounds: Record<string, number> = {};
  for (const e of plan.edges) {
    const round = (finished[e.from] ?? 0) + (working[e.from] ? 1 : 0);
    if (e.loop && round) rounds[edgeId(e)] = Math.min(e.loop.rounds, round);
  }
  return { live, rounds };
}

const running = (run: TeamRun) => run.members.some((m) => m.state === "working");

function since(ms: number) {
  const minutes = Math.max(0, Math.round((Date.now() - ms) / 60_000));
  return minutes < 60 ? `${minutes}m` : `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** The bar over the grid while a team's at work, or done and not put away yet. */
export function RunBar({
  run,
  watching,
  changing,
  onWatch,
  onChange,
  onStopped,
}: {
  run: TeamRun;
  watching: boolean;
  /** Changing the team as it works (see Steer.tsx), rather than following it */
  changing: boolean;
  onWatch: (watch: boolean) => void;
  onChange: (changing: boolean) => void;
  onStopped: () => void;
}) {
  const { live } = liveOf(run);
  const counts = (["needs-you", "working", "waiting", "done"] as Status[])
    .map((st) => [st, Object.values(live).filter((l) => l.kind !== "idle" && l.status === st).length] as const)
    .filter(([, n]) => n)
    .map(([st, n]) => `${n} ${STATUS[st].toLowerCase()}`)
    .join(" · ");
  const active = running(run);
  return (
    <div className="plan-running">
      <span className={`plan-dot ${active ? "working" : "done"}`} />
      <b>
        {run.plan.name} {active ? "is at work" : "is done"}
      </b>
      <span>
        on {run.leadState?.title ?? "its chat"} · {since(run.started)} · {counts}
      </span>
      <span className="ask-spacer" />
      {watching && active && (
        <div className="segmented steer-mode">
          <button className={changing ? "" : "on"} onClick={() => onChange(false)}>
            Follow
          </button>
          <button className={changing ? "on" : ""} onClick={() => onChange(true)} title="Change the team while it works">
            Change
          </button>
        </div>
      )}
      <button className="ask-btn ghost" onClick={() => onWatch(!watching)}>
        {watching ? "Edit the Plan" : "Follow the Team"}
      </button>
      {active ? (
        <button className="ask-btn ghost" onClick={() => bridge.stopTeam(run.id).then(onStopped)}>
          Stop
        </button>
      ) : (
        <button className="ask-btn ghost" onClick={() => bridge.clearTeam(run.id).then(onStopped)} title="Put this run away">
          Clear
        </button>
      )}
    </div>
  );
}

/** While a team works: how the picked agent's getting on, what it did lately and said, and a message for it. */
export function LivePanel({ run, selected }: { run: TeamRun; selected: string | null }) {
  const plan: Plan = run.plan;
  const agent = plan.agents.find((a) => a.id === selected);
  const live = agent ? liveOf(run).live[agent.id] : undefined;
  const [text, setText] = useState("");
  const [sent, setSent] = useState("");
  const [problem, setProblem] = useState("");
  useEffect(() => {
    setText("");
    setSent("");
    setProblem("");
  }, [selected]);
  if (!agent || !live)
    return <div className="inspector empty">Pick an agent to see what it's doing, or to send the Lead or a manager a message.</div>;

  const session = live.kind === "lead" ? run.lead : live.kind === "manager" ? live.member?.session : undefined;
  const hears = (live.kind === "lead" && live.status !== "done") || (live.kind === "manager" && live.status === "working");
  const send = async () => {
    if (!text.trim()) return;
    try {
      if (live.kind === "lead") await bridge.sendToSession(run.lead, text.trim());
      else await bridge.messageManager(run.id, agent.id, text.trim());
      setSent(`Sent. ${agent.name} sees it after its current step.`);
      setText("");
      setProblem("");
    } catch (e) {
      setProblem(e instanceof Error ? e.message : String(e));
    }
  };
  const now = async () => {
    try {
      if (session) await bridge.stopStep(session);
      setSent(`Sent. ${agent.name} is reading it now.`);
    } catch (e) {
      setProblem(e instanceof Error ? e.message : String(e));
    }
  };
  const kind = {
    lead: "your chat",
    manager: "its own session",
    worker: `${live.member?.name}'s subagent`,
    "lead-worker": "the Lead's subagent",
    idle: "not running",
  }[live.kind];

  return (
    <div className="inspector">
      <div className="inspector-head">
        <Avatar icon={agent.icon} size={34} corner={<i className={`plan-dot ${live.status}`} />} />
        <div>
          <div className="inspector-title">{agent.name}</div>
          <div className="inspector-sub">
            {STATUS[live.status]} · {agent.model} · {kind}
          </div>
        </div>
      </div>
      {/* Done with something to say, what it said says it */}
      {!(live.status === "done" && live.said) && (
        <>
          <div className="inspector-label">{live.status === "working" ? "Now" : "Where it is"}</div>
          <div className="live-now">{live.doing}</div>
        </>
      )}
      {live.steps && live.steps.length > 0 && (
        <>
          <div className="inspector-label">Lately</div>
          <ol className="live-steps">
            {live.steps.map((step, i) => (
              <li key={i}>{step}</li>
            ))}
          </ol>
        </>
      )}
      {live.said && (
        <>
          <div className="inspector-label">{live.kind === "manager" ? "Its report" : "What it said"}</div>
          <div className="live-said">
            <Markdown text={live.said} />
          </div>
        </>
      )}
      {hears && !run.canMessage && (
        <div className="inspector-hint loop-hint">
          To message the Lead or a manager while it works, turn on Hands-free in Settings → Hooks. Claude Code then checks for messages after each
          step.
        </div>
      )}
      {hears && run.canMessage && (
        <>
          <div className="inspector-label">Message {agent.name}</div>
          <SpokenField className="inspector-field" rows={3} value={text} onChange={setText} placeholder={`Tell ${agent.name} something`} />
          {sent && <div className="pending-sent">{sent}</div>}
          {problem && <div className="ask-problem">{problem}</div>}
          <div className="live-actions">
            {sent && live.status === "working" && session && (
              <button className="ask-btn" onClick={now} title="Stop the command it's running, so it reads your message now">
                Send Now
              </button>
            )}
            <span className="ask-spacer" />
            <button className="ask-btn primary" disabled={!text.trim()} onClick={send}>
              Send
            </button>
          </div>
        </>
      )}
      {(live.kind === "worker" || live.kind === "lead-worker") && (
        <div className="inspector-hint loop-hint">
          It runs as a subagent, so it can't take a message. Tell {live.kind === "worker" ? live.member?.name : "the Lead"} instead.
        </div>
      )}
      {live.kind === "lead" && (
        <button className="ask-btn ghost ask-open live-open" onClick={() => bridge.openPending(run.lead)}>
          Open in Claude ↗
        </button>
      )}
    </div>
  );
}

/** What's happening in a run, in a few words: someone who needs the user, a loop that's gone round, or what the
 *  first one at work is doing. */
function happening(run: TeamRun, live: Record<string, AgentLive>, rounds: Record<string, number>) {
  const agents = run.plan.agents.filter((a) => live[a.id]?.kind !== "idle");
  const name = (id: string) => run.plan.agents.find((a) => a.id === id)?.name ?? id;
  const asks = agents.find((a) => live[a.id].status === "needs-you");
  if (asks) return `${asks.name} needs you`;
  const loop = run.plan.edges.find((e) => e.loop && (rounds[edgeId(e)] ?? 0) > 1 && live[e.from]?.status === "working");
  if (loop) return `${name(loop.from)}'s on round ${rounds[edgeId(loop)]} of ${loop.loop!.rounds} with ${name(loop.to)}`;
  const busy =
    agents.find((a) => live[a.id].kind === "worker" && live[a.id].status === "working") ?? agents.find((a) => live[a.id].status === "working");
  return busy ? `${busy.name}: ${live[busy.id].doing}` : "";
}

/** A team at work on its lead's row in the menu bar list: a segment for each agent, by how it's getting on, and a
 *  line on what's happening. A click follows the team in the planner. */
export function TeamBar({ run, onOpen }: { run: TeamRun; onOpen: () => void }) {
  const { live, rounds } = liveOf(run);
  const agents = run.plan.agents.filter((a) => live[a.id]?.kind !== "idle");
  const order: Status[] = ["needs-you", "working", "waiting", "stopped", "failed", "done"];
  const sorted = order.flatMap((st) => agents.filter((a) => live[a.id].status === st));
  const done = agents.filter((a) => live[a.id].status === "done").length;
  const now = happening(run, live, rounds);
  return (
    <button
      className="team-bar-box"
      title="Follow the team in the planner"
      onClick={(e) => {
        e.stopPropagation();
        onOpen();
      }}
    >
      <span className="team-bar">
        {sorted.map((a) => (
          <i key={a.id} className={live[a.id].status} />
        ))}
      </span>
      <span className="team-bar-note">
        {running(run) ? `${run.plan.name}: ${done} of ${agents.length} done` : `${run.plan.name} is done`}
        {running(run) && now && ` · ${now}`}
      </span>
    </button>
  );
}
