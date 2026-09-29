// Planning a team with Claude: the planner's plan shared with a chat, which changes it by editing its file while the
// user changes it here. Headroom watches the file, so each side sees the other's changes as they land, listed in the
// panel by who made them. A shared plan saves as it's changed, for Claude to see; Undo takes back the last change,
// whoever made it.
//
// When both change it at once, Claude's version wins, and the panel says it replaced the user's edit, which Undo
// brings back. A change Claude's file has that can't be read isn't taken, and the panel says so until one can.

import { useState } from "react";
import { Avatar, type ChatIcon } from "./Avatar";
import { bridge } from "./bridge";
import { deliver, useChats, type Plan } from "./Planner";
import { SpokenField } from "./Voice";

/** A plan shared with a chat. */
export type Share = { plan: string; chat: string; title: string; icon?: ChatIcon };
/** A change, as the panel lists it: who made it, what it was, and when. */
export type Said = { who: "claude" | "you"; text: string; at: number };

/** The share there was when the planner last closed, if any, so it carries on. */
export function savedShare(): Share | null {
  try {
    return JSON.parse(localStorage.getItem("planner-share") ?? "null");
  } catch {
    return null;
  }
}
export function keepShare(share: Share | null) {
  try {
    if (share) localStorage.setItem("planner-share", JSON.stringify(share));
    else localStorage.removeItem("planner-share");
  } catch {
    // Not kept past this window, which is fine
  }
}

/** What Claude's told when the plan's shared: where the file is, what's in it, and how to change it. */
export function shareMessage(plan: Plan, path: string) {
  return [
    `Let's plan a team of agents together in Headroom's planner. The plan's called "${plan.name}", and it's this file:`,
    "",
    path,
    "",
    "Read it, and change the team by editing that file. Headroom shows me each change as you make it, and my changes in the planner go into the same file, so read it again before each edit.",
    "",
    "It's JSON: `name`, `agents`, and `edges`.",
    '- Each agent has `id`, `name`, `icon` ({"emoji": "…"}), `model` ("Opus", "Sonnet", or "Haiku"), `role` ("Lead", "Manager", or "Worker"), `x` and `y` (its place on a grid, in multiples of 20, about 180 apart across and 130 down), `brief` (its standing instructions), `commands` (slash commands and skills, like "/review"), `tools` (any of "Read files", "Edit files", "Run commands", "Browse the web"), and, if it should keep going until something holds, `until`.',
    '- Each edge is `from` and `to`, agent IDs: whoever hands work to whoever does it. A loop sends the work back until something holds: `"loop": {"until": "approved", "rounds": 3}`.',
    "- There's one Lead: my chat. Managers run their own teams in sessions of their own, with their workers as subagents.",
    "- Keep each agent's `id` as it is. Give a new one a short `id` of its own.",
    "",
    "Tell me what you think the team should look like for what we're working on, or wait until I ask.",
  ].join("\n");
}

/** Picking the chat to plan with, which is sent the plan's file and how to change it. */
export function ShareSheet({ plan, onShared, onClose }: { plan: Plan; onShared: (share: Share) => void; onClose: () => void }) {
  const chats = useChats();
  const [chosen, setChosen] = useState<string | null>(null);
  const [problem, setProblem] = useState("");
  const [busy, setBusy] = useState(false);
  const chat = chats.find((c) => c.id === chosen);
  const share = async () => {
    if (!chat || busy) return;
    setBusy(true);
    setProblem("");
    try {
      const path = await bridge.planPath(plan.id);
      if (!path) throw new Error("Save the plan first");
      await deliver(chat.id, shareMessage(plan, path));
      onShared({ plan: plan.id, chat: chat.id, title: chat.title, icon: chat.icon });
    } catch (e) {
      setProblem(`Couldn't share it: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="sheet-backdrop" onPointerDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="sheet">
        <div className="inspector-title">Plan {plan.name} with Claude</div>
        <div className="inspector-hint">
          The chat's sent where the plan's file is and how to change it. Its changes show here as they land, yours go into the same file, and the plan
          saves as it's changed.
        </div>
        <div className="inspector-label">Chat</div>
        <div className="sheet-chats">
          {chats.length === 0 && <div className="inspector-hint">No chats are open. Start one in Claude or a terminal first.</div>}
          {chats.map((c) => (
            <button key={c.id} className={chosen === c.id ? "sheet-chat on" : "sheet-chat"} onClick={() => setChosen(c.id)}>
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
        {problem && <div className="ask-problem">{problem}</div>}
        <div className="plan-foot">
          <span className="ask-spacer" />
          <button className="ask-btn ghost" onClick={onClose}>
            Cancel
          </button>
          <button className="ask-btn primary" disabled={!chat || busy} onClick={share}>
            Plan Together
          </button>
        </div>
      </div>
    </div>
  );
}

function ago(at: number) {
  const s = Math.round((Date.now() - at) / 1000);
  return s < 45 ? "just now" : s < 3600 ? `${Math.round(s / 60)}m` : `${Math.round(s / 3600)}h`;
}

/** The panel while planning with Claude, when nothing's picked: who's changed what, a word for Claude, and Undo. */
export function TogetherPanel({
  share,
  said,
  problem,
  canUndo,
  onUndo,
}: {
  share: Share;
  said: Said[];
  problem: string;
  canUndo: boolean;
  onUndo: () => void;
}) {
  const [text, setText] = useState("");
  const [sent, setSent] = useState("");
  const tell = async () => {
    if (!text.trim()) return;
    try {
      const how = await deliver(share.chat, text.trim());
      setSent(how === "sent" ? "Sent." : `Copied: paste it into ${share.title}.`);
      setText("");
    } catch (e) {
      setSent(`Couldn't send it: ${e instanceof Error ? e.message : String(e)}`);
    }
  };
  return (
    <div className="inspector together">
      <div className="inspector-title">With Claude</div>
      <div className="inspector-sub">Each of you sees the other's changes as they land. Pick an agent to set it up.</div>
      {problem && <div className="ask-problem">{problem}</div>}
      <ol className="together-feed">
        {said.length === 0 && <li className="inspector-hint">No changes yet.</li>}
        {said.map((s, i) => (
          <li key={i}>
            <span className={`together-who ${s.who}`}>{s.who === "claude" ? "Claude" : "You"}</span> {s.text}
            <span className="together-when">{ago(s.at)}</span>
          </li>
        ))}
      </ol>
      <div className="live-actions">
        <button className="ask-btn ghost" disabled={!canUndo} onClick={onUndo} title="⌘Z">
          Undo
        </button>
      </div>
      <div className="inspector-label">Tell Claude</div>
      <SpokenField className="inspector-field" rows={2} value={text} onChange={setText} placeholder="Split the API work between two agents" />
      {sent && <div className="pending-sent">{sent}</div>}
      <div className="live-actions">
        <span className="ask-spacer" />
        <button className="ask-btn primary" disabled={!text.trim()} onClick={tell}>
          Send
        </button>
      </div>
    </div>
  );
}
