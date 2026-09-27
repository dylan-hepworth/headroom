// A mockup of the planner while a team runs: how each agent's getting on, the loops' rounds, and a message for one of
// them, on made-up progress. Running a team isn't built yet; `?plan` alone shows the planner itself.
//
// `npm run ui`, then add `?plan&running` to the address.

import { useState } from "react";
import { Avatar } from "./Avatar";
import myCompany from "./my-company.json";
import { Canvas, PlannerWindow, type Agent, type Plan } from "./Planner";
import "./plan.css";
import "./popover.css";

const COMPANY = myCompany as Plan;

/** How an agent's getting on while the plan runs. */
type Status = "working" | "needs-you" | "done" | "waiting";
type Live = { status: Status; doing: string; for?: string; steps?: string[] };
const STATUS: Record<Status, string> = { working: "Working", "needs-you": "Needs you", done: "Done", waiting: "Waiting" };

/** The team partway through, made up for the mockup. */
const LIVE: Record<string, Live> = {
  lead: { status: "waiting", doing: "Waiting on Development and R&D", for: "18m" },
  mkt: { status: "done", doing: "Launch post and three social posts ready", for: "12m" },
  copy: { status: "done", doing: "Wrote the launch post", for: "9m" },
  social: { status: "done", doing: "Scheduled 3 posts for Tuesday", for: "4m" },
  dev: { status: "working", doing: "Coder's on round 2 of review", for: "16m" },
  coder: {
    status: "working",
    doing: "Editing src/auth/session.ts",
    for: "14m",
    steps: ["Read the reviewer's notes", "Edited src/auth/session.ts", "Ran npm test", "Edited src/auth/session.ts"],
  },
  review: { status: "waiting", doing: "Round 1: asked for smaller functions", for: "6m" },
  tests: { status: "waiting", doing: "Starts after the Coder", for: "" },
  rnd: { status: "needs-you", doing: "Asks: include paid sources in the comparison?", for: "2m" },
  research: { status: "done", doing: "Compared 6 services", for: "11m" },
  sum: { status: "working", doing: "Writing the summary table", for: "3m" },
  brief: { status: "waiting", doing: "Starts when both are done", for: "" },
};

/** While the plan runs: how the picked agent is getting on, what it's done lately, and a message for it. */
function LivePanel({ n, live }: { n?: Agent; live?: Live }) {
  const [sent, setSent] = useState(false);
  if (!n || !live) return <div className="inspector empty">Pick an agent to see what it's doing, or to send it a message.</div>;
  return (
    <div className="inspector">
      <div className="inspector-head">
        <Avatar icon={n.icon} size={34} corner={<i className={`plan-dot ${live.status}`} />} />
        <div>
          <div className="inspector-title">{n.name}</div>
          <div className="inspector-sub">
            {STATUS[live.status]}
            {live.for && ` · ${live.for}`} · {n.model} · {n.role === "Manager" ? "its own session" : n.role === "Lead" ? "your chat" : "subagent"}
          </div>
        </div>
      </div>
      <div className="inspector-label">Now</div>
      <div className="live-now">{live.doing}</div>
      {live.steps && (
        <>
          <div className="inspector-label">Lately</div>
          <ol className="live-steps">
            {live.steps.map((step, i) => (
              <li key={i}>{step}</li>
            ))}
          </ol>
        </>
      )}
      {live.status !== "done" && (
        <>
          <div className="inspector-label">Message {n.name}</div>
          <textarea className="inspector-field" rows={3} defaultValue="Keep the session code in one file, it's easier to review." />
          {sent && <div className="pending-sent">Sent. {n.name} sees it after its current step.</div>}
          <div className="live-actions">
            {sent && live.status === "working" && <button className="ask-btn primary">Send Now</button>}
            <span className="ask-spacer" />
            <button className={sent ? "ask-btn" : "ask-btn primary"} onClick={() => setSent(true)}>
              Send
            </button>
          </div>
        </>
      )}
      {n.role !== "Worker" && <button className="ask-btn ghost ask-open live-open">Open in Claude ↗</button>}
    </div>
  );
}

/** The plan while it runs: the same grid, with how each agent's getting on, and what's asked of it. `?plan&running` */
function Running() {
  const [selected, setSelected] = useState<string | null>("coder");
  const counts = (Object.keys(STATUS) as Status[])
    .map((st) => [st, Object.values(LIVE).filter((l) => l.status === st).length] as const)
    .filter(([, n]) => n)
    .map(([st, n]) => `${n} ${STATUS[st].toLowerCase()}`)
    .join(" · ");
  return (
    <div className="plan-stage">
      <div className="running-row pop-vars">
        <Avatar icon={{ emoji: "🏢" }} size={26} corner={<i className="count-dot working" />} />
        <div>
          <b>Launch the sign-in redesign</b>
          <div>web-app · Working</div>
        </div>
        <span className="ask-spacer" />
        <button className="plan-pill">12 agents · 1 needs you</button>
      </div>
      <div className="plan-window pop-vars">
        <div className="plan-toolbar">
          <span className="template-menu">🏢 My company, on Launch the sign-in redesign</span>
          <span className="plan-sub">18m · {counts}</span>
          <span className="ask-spacer" />
          <button className="ask-btn ghost">Pause All</button>
          <button className="ask-btn ghost">Stop</button>
        </div>
        <div className="plan-body">
          <Canvas agents={COMPANY.agents} edges={COMPANY.edges} selected={selected} onSelect={setSelected} live={LIVE} />
          <LivePanel n={COMPANY.agents.find((n) => n.id === selected)} live={selected ? LIVE[selected] : undefined} />
        </div>
      </div>
    </div>
  );
}

export function PlanPreview() {
  return location.search.includes("running") ? <Running /> : <PlannerWindow />;
}
