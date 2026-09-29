// Steering a team at work (see Running.tsx): the plan it's following, changed on the grid, and the changes sent to
// whoever they're for. A manager hears about its team's changes after its current step, the way a message from the
// list goes in: a worker it's been given hands off as a general-purpose subagent with the instructions and model it
// should have, and one whose model changed is handed its next work on that model. A new manager starts as a session of
// its own, and one taken off the team is stopped. The lead hears what changed, as a message to its chat.
//
// What a session that's already running can't take isn't sent: a manager's own model, and a worker that needs a tool
// its manager wasn't started with (a manager's refused anything else, rather than asked).

import { useState } from "react";
import { bridge, type TeamRun } from "./bridge";
import { changes, type Change } from "./planChanges";
import { deliver, describe, launch, list, teamOf, toolNames, type Agent, type Launch, type Plan } from "./Planner";

/** What changes to a team at work come to: who's told what, who starts or stops, and what can't be done. */
export type Steering = {
  /** For each manager at work whose team changed: what changed, and the message it's sent */
  managers: { agent: string; name: string; changed: string[]; text: string }[];
  /** For the lead's chat: what changed, and the message */
  lead: { changed: string[]; text: string } | null;
  start: Launch[];
  stop: { agent: string; name: string }[];
  /** Why some of it can't be sent */
  problems: string[];
};

const TOOL_WORDS: Record<string, string> = {
  "Read files": "read files",
  "Edit files": "edit files",
  "Run commands": "run commands",
  "Browse the web": "browse the web",
};

export function steering(run: TeamRun, draft: Plan): Steering {
  const before = run.plan;
  const all = changes(before, draft);
  const agent = (id: string): Agent | undefined => draft.agents.find((a) => a.id === id) ?? before.agents.find((a) => a.id === id);
  const working = run.members.filter((m) => m.state === "working");
  const member = (id: string) => run.members.find((m) => m.agent === id);
  // What each manager at work was started able to do
  const allowed = Object.fromEntries(launch(before, "").managers.map((m) => [m.agent, m.tools]));
  const problems: string[] = [];

  const start = launch(draft, "").managers.filter((m) => !member(m.agent));
  const stop = working.filter((m) => agent(m.agent) === undefined || draft.agents.find((a) => a.id === m.agent)?.role !== "Manager");

  // Whose each change is: a manager's (for its team, or itself), or the lead's
  const forManager = new Map<string, Change[]>();
  const forLead: Change[] = [];
  for (const c of all) {
    const a = c.agent ? agent(c.agent) : undefined;
    const owner = !a ? undefined : a.role === "Manager" ? a.id : (teamOf(draft, a.id)[0] ?? teamOf(before, a.id)[0]);
    if (!owner || a?.role === "Lead") {
      forLead.push(c);
      continue;
    }
    // A new manager starts with all of it, and one that's stopped needs telling nothing
    if (start.some((m) => m.agent === owner) || stop.some((m) => m.agent === owner)) continue;
    const m = member(owner);
    if (!m && draft.agents.find((a) => a.id === owner)?.role === "Manager") {
      problems.push(`${agent(owner)?.name} needs a worker of its own before it can start.`);
      continue;
    }
    if (!m || m.state !== "working") {
      problems.push(`${agent(owner)?.name ?? "That manager"} isn't at work any more, so its team can't be changed.`);
      continue;
    }
    forManager.set(owner, [...(forManager.get(owner) ?? []), c]);
  }

  const managers = [...forManager.entries()].map(([id, list]) => {
    const m = member(id)!;
    const byKey = Object.fromEntries(Object.entries(m.workers).map(([key, agentId]) => [agentId, key]));
    const lines: string[] = [];
    const affected = [...new Set(list.map((c) => c.agent!).filter((x) => x !== id))];
    for (const wid of affected) {
      const w = draft.agents.find((a) => a.id === wid);
      const old = before.agents.find((a) => a.id === wid);
      if (!w || teamOf(draft, wid)[0] !== id) {
        lines.push(`- ${old?.name ?? "A worker"}: taken off your team. Don't hand it any more work.`);
        continue;
      }
      const missing = w.tools.filter((t) => !toolNames([t]).every((n) => allowed[id]?.includes(n)));
      if (missing.length) {
        problems.push(`${w.name} needs to ${missing.map((t) => TOOL_WORDS[t]).join(" and ")}, which ${m.name} wasn't started able to do.`);
      }
      const how = byKey[wid] ? `the subagent "${byKey[wid]}"` : "a general-purpose subagent";
      const said = describe(draft, w);
      lines.push(
        `- ${w.name}: hand it work as ${how}, passing model "${w.model.toLowerCase()}"${byKey[wid] ? "" : ", and put its instructions in the prompt"}.` +
          `${said ? ` Its instructions: You're ${w.name}. ${said}` : ""}`,
      );
    }
    const self = draft.agents.find((a) => a.id === id);
    const was = before.agents.find((a) => a.id === id);
    if (self && was && self.model !== was.model) problems.push(`${self.name}'s own model can't change while it works.`);
    if (self && was && (self.brief !== was.brief || self.until !== was.until || self.commands.join() !== was.commands.join())) {
      lines.push(`- You: ${describe(draft, self) || "no standing instructions now"}`);
    }
    const text = [
      "While you work, the user changed your team in Headroom's planner:",
      ...list.map((c) => `- ${c.text}`),
      "",
      "From now on:",
      ...lines,
      "",
      "Fit these in where they belong in what you're doing.",
    ].join("\n");
    return { agent: id, name: m.name, changed: list.map((c) => c.text), text };
  });

  // The lead hears what changed that's its own, and who's joined or left
  const news = [
    ...forLead.map((c) => c.text),
    ...start.map((m) => `started ${m.name} as a session of its own, on this same work: wait for its report too`),
    ...stop.map((m) => `took ${m.name} off the team and stopped it`),
  ];
  const leadAgent = draft.agents.find((a) => a.role === "Lead");
  const wasLead = before.agents.find((a) => a.role === "Lead");
  if (leadAgent && wasLead && leadAgent.model !== wasLead.model) problems.push("The Lead's model is your chat's, which the planner can't change.");
  const lead = news.length
    ? {
        changed: news,
        text: ["While the team works, I changed its plan in Headroom's planner:", ...news.map((n) => `- I ${n}`)].join("\n"),
      }
    : null;

  if (managers.length && !run.canMessage) {
    problems.push("To send changes to a manager at work, turn on Hands-free in Settings → Hooks: that's how a message gets to it.");
  }
  return { managers, lead, start, stop: stop.map((m) => ({ agent: m.agent, name: m.name })), problems };
}

/** The panel while changing a team at work, when nothing's picked: what's changed, for whom, and sending it. */
export function SteerPanel({
  run,
  draft,
  onAgent,
  onSent,
  onDiscard,
}: {
  run: TeamRun;
  draft: Plan;
  onAgent: () => void;
  onSent: () => void;
  onDiscard: () => void;
}) {
  const s = steering(run, draft);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState("");
  const [problem, setProblem] = useState("");
  const nothing = !s.managers.length && !s.lead && !s.start.length && !s.stop.length;
  const send = async () => {
    setBusy(true);
    setProblem("");
    try {
      for (const m of s.stop) await bridge.stopMember(run.id, m.agent);
      for (const m of s.start) await bridge.addManager(run.id, m);
      for (const m of s.managers) await bridge.messageManager(run.id, m.agent, m.text);
      const how = s.lead ? await deliver(run.lead, s.lead.text) : null;
      await bridge.updateTeamPlan(run.id, draft);
      const told = [...s.managers.map((m) => m.name), ...(s.lead ? ["the Lead"] : [])];
      setDone(
        [
          told.length ? `Sent to ${list(told)}: they fit it in after their current step.` : "",
          how === "copied" ? "The Lead's part is copied, to paste into its chat." : "",
          s.start.length ? `Started ${list(s.start.map((m) => m.name))}.` : "",
          s.stop.length ? `Stopped ${list(s.stop.map((m) => m.name))}.` : "",
        ]
          .filter(Boolean)
          .join(" "),
      );
      onSent();
    } catch (e) {
      setProblem(`Couldn't send it all: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="inspector">
      <div className="inspector-title">Changes</div>
      <div className="inspector-sub">
        {nothing ? "Change the team on the grid, and what it comes to shows here." : "Not sent yet. Pick an agent to set it up."}
      </div>
      {done && <div className="pending-sent">{done}</div>}
      {s.managers.map((m) => (
        <div key={m.agent}>
          <div className="inspector-label">For {m.name}</div>
          <ul className="steer-changes">
            {m.changed.map((c, i) => (
              <li key={i}>{c}</li>
            ))}
          </ul>
        </div>
      ))}
      {s.start.length > 0 && (
        <>
          <div className="inspector-label">Starts</div>
          <ul className="steer-changes">
            {s.start.map((m) => (
              <li key={m.agent}>{m.name}, as a session of its own</li>
            ))}
          </ul>
        </>
      )}
      {s.stop.length > 0 && (
        <>
          <div className="inspector-label">Stops</div>
          <ul className="steer-changes">
            {s.stop.map((m) => (
              <li key={m.agent}>{m.name}, and whatever it's running</li>
            ))}
          </ul>
        </>
      )}
      {s.lead && (
        <>
          <div className="inspector-label">For the Lead</div>
          <ul className="steer-changes">
            {s.lead.changed.map((c, i) => (
              <li key={i}>{c}</li>
            ))}
          </ul>
        </>
      )}
      {s.problems.map((p, i) => (
        <div key={i} className="ask-problem">
          {p}
        </div>
      ))}
      {problem && <div className="ask-problem">{problem}</div>}
      {!nothing && (
        <div className="inspector-hint loop-hint">
          A manager fits changes in after its current step. A worker that's in the middle of something finishes it as it was.
        </div>
      )}
      <div className="live-actions">
        <button className="ask-btn ghost" onClick={onAgent}>
          + Agent
        </button>
        <span className="ask-spacer" />
        <button className="ask-btn ghost" disabled={nothing || busy} onClick={onDiscard}>
          Discard
        </button>
        <button className="ask-btn primary" disabled={nothing || busy || s.problems.length > 0} onClick={send}>
          Send Changes
        </button>
      </div>
    </div>
  );
}
