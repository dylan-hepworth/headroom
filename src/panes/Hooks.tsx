import { useEffect, useState } from "react";
import type { PaneProps } from "../App";
import { needsAnswer, yourTurn } from "../bridge";
import { Chat, Hook, Shield } from "../icons";
import { Popup, Row, Section, Switch } from "../ui";

const NEAR = [
  [70, "70%"],
  [75, "75%"],
  [80, "80%"],
  [85, "85%"],
  [90, "90%"],
  [95, "95%"],
] as const;
const HOLD = [
  [30, "30 seconds"],
  [60, "1 minute"],
  [120, "2 minutes"],
  [300, "5 minutes"],
  [600, "10 minutes"],
] as const;

const SEND = [
  ["message", "With my next message"],
  ["now", "Right away"],
] as const;

/** The user's own words for what Claude should do near a limit. Saved when they're done typing, not on each key. */
function MessageField({ settings, set, disabled }: { settings: PaneProps["settings"]; set: PaneProps["set"]; disabled: boolean }) {
  const [text, setText] = useState(settings.nearLimitMessage);
  useEffect(() => setText(settings.nearLimitMessage), [settings.nearLimitMessage]);
  const save = () => {
    if (text.trim() !== settings.nearLimitMessage) set("nearLimitMessage", text.trim());
  };
  return (
    <textarea
      className="field message-field"
      rows={2}
      value={text}
      placeholder={settings.nearLimitDefault}
      disabled={disabled}
      onChange={(e) => setText(e.target.value)}
      onBlur={save}
      onKeyDown={(e) => {
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          e.currentTarget.blur();
        }
      }}
    />
  );
}

/** The note Claude gets, as it would read with the current usage, or with an example once it's over the threshold. It
 *  matches what the hook writes (`near_limit_note` in hooks.rs). */
function noteExample(app: PaneProps["app"], settings: PaneProps["settings"]) {
  const at = settings.nearLimitAt;
  const advice = settings.nearLimitMessage.trim() || settings.nearLimitDefault;
  const five = app.limits.find((l) => l.key === "5h");
  const pct = five && five.pct >= at ? five.pct : Math.min(100, at + 6);
  const resets = five?.resets ?? "3:10 PM";
  const mins = five?.resetsAt ? Math.max(0, Math.floor((five.resetsAt - Date.now()) / 60_000)) : 29;
  const left = mins < 90 ? `in ${mins} minute${mins === 1 ? "" : "s"}` : `in ${Math.round(mins / 60)} hours`;
  return `Headroom: the user is at ${pct}% of their 5-hour Claude limit, which resets at ${resets} (${left}). ${advice}`;
}

export default function Hooks({ app, settings, set }: PaneProps) {
  const off = !settings.hooks;
  const [error, setError] = useState<string | null>(null);
  const waiting = app.sessions.filter((s) => needsAnswer(s) || yourTurn(s)).length;
  return (
    <>
      <Section note="Hooks are commands Claude Code runs when something happens in a session. Headroom adds its own to ~/.claude/settings.json and takes them out again when this is off. Your other hooks and settings are left alone, and what you type to Claude never reaches Headroom, apart from your recent messages and answers, shortened, kept with a question in hands-free mode until it's answered, and a message you send a chat from the list, kept until the chat takes it or ends.">
        <div className="group">
          <Row
            tile={["purple", <Hook />]}
            title="Hooks"
            detail={
              error ? (
                <span className="error-text">{error}</span>
              ) : settings.hooks ? (
                <>
                  <span className="dot green" />
                  {app.sessions.length === 0
                    ? "On. New Claude Code sessions will show up in Sessions."
                    : `On. ${app.sessions.length} ${app.sessions.length === 1 ? "session" : "sessions"}, ${waiting} waiting on you.`}
                </>
              ) : (
                "Off. Headroom only sees your usage."
              )
            }
          >
            <Switch checked={settings.hooks} onChange={(v) => set("hooks", v).then(setError)} label="Hooks" />
          </Row>
        </div>
      </Section>

      <Section
        title="Approvals"
        note="Only for requests Claude Code would have asked you about anyway. Anything your settings already allow or deny never reaches Headroom. Skipped when you're already looking at the session: its terminal is in front, or its chat is the one open in the Claude app."
      >
        <div className="group">
          <Row
            tile={["green", <Shield />]}
            title="Approve from the menu bar"
            detail="Permission requests and Claude's questions drop down from the menu bar icon, to answer right there."
            info="While Headroom holds a request, Claude Code's own prompt waits. If you don't answer in time, or Headroom quits, the prompt shows in the terminal as usual."
          >
            <Switch checked={settings.approvals} onChange={(v) => set("approvals", v)} label="Approve from the menu bar" disabled={off} />
          </Row>
          <Row inset title="If I don't answer, ask in the terminal after">
            <Popup
              value={settings.approvalTimeout}
              options={HOLD}
              onChange={(v) => set("approvalTimeout", v)}
              label="Ask in the terminal after"
              disabled={off || !settings.approvals}
            />
          </Row>
          <Row
            inset
            title="Open automatically when a session needs an answer"
            detail="When it's off, requests wait behind the yellow dot until you click the icon."
          >
            <Switch
              checked={settings.popoverAuto}
              onChange={(v) => set("popoverAuto", v)}
              label="Open automatically when a session needs an answer"
              disabled={off || !settings.approvals}
            />
          </Row>
          <Row
            inset
            title="Compact cards"
            detail="Each request in a few lines. Point at Claude's reply, an option, or a command to read all of it, press an option's number to pick it, or show the whole card."
          >
            <Switch
              checked={settings.compactCards}
              onChange={(v) => set("compactCards", v)}
              label="Compact cards"
              disabled={off || !settings.approvals}
            />
          </Row>
          <Row
            inset
            title="When Claude finishes a turn"
            detail={`Claude can ask what to do next, with a few choices to pick from right here, and “That’s all for now” to let it stop. Asking only when it needs a decision leaves it free to just tell you things, in full.`}
            info="Headroom adds a line to your messages saying when Claude should ask. Set to always, a turn that ends without the question is sent back to ask. Sessions that are already open hear about it with your next message, if they started while approvals were on (others, once they restart), and turning it off tells the ones that heard to stop. For this, Claude Code waits a moment for Headroom (about 20 milliseconds) on each message and at the end of each turn, while approvals are on."
          >
            <Popup
              value={settings.askWhen}
              options={
                [
                  ["decision", "Ask when it needs a decision"],
                  ["always", "Always ask what's next"],
                  ["never", "Don't ask"],
                ] as const
              }
              onChange={(v) => set("askWhen", v)}
              label="When Claude finishes a turn"
              disabled={off || !settings.approvals}
            />
          </Row>
          <Row
            inset
            title="Hands-free"
            detail="Each question comes with what Claude said that turn, and what's gone on lately. Click the menu bar icon to see every chat waiting on you or at work, reply to finished ones, and send a message to one mid-task. Right-click it for the menu."
            info="Headroom reads Claude's reply, and your recent messages and answers (the last few, shortened), from the session's transcript when the question comes in, and keeps them only until it's answered. Claude is asked to end with a short summary of what it did, which is the only extra it writes. So you can reply from the menu bar, a finished turn waits for a reply (up to the time above), and the chat looks like it's still working meanwhile. It lets go as soon as you open that chat yourself. A message you send a chat at work reaches it after its next step (or right away with Send Now, which stops the command it's running), since Claude Code waits for Headroom after each one (about 20 ms) while this is on."
          >
            <Switch checked={settings.handsFree} onChange={(v) => set("handsFree", v)} label="Hands-free" disabled={off || !settings.approvals} />
          </Row>
          <Row
            inset
            title="Show replies right away"
            detail="A finished turn's reply drops down from the menu bar, like a question, to read and answer. Off, it waits in the list."
          >
            <Switch
              checked={settings.showReplies}
              onChange={(v) => set("showReplies", v)}
              label="Show replies right away"
              disabled={off || !settings.approvals || !settings.handsFree}
            />
          </Row>
        </div>
      </Section>

      <Section
        title="Tell Claude About Your Limits"
        note={
          settings.nearLimit
            ? settings.nearLimitRightAway
              ? "To add the note, Claude Code waits a moment for Headroom (about 20 milliseconds) on each message, and after each step a session takes."
              : "To add the note, Claude Code waits a moment for Headroom (about 20 milliseconds) on each message."
            : undefined
        }
      >
        <div className="group">
          <Row
            tile={["clay", <Chat />]}
            title="Add a note when I'm near a limit"
            detail="Claude sees it with your message, and can wrap up or save its progress before you run out."
          >
            <Switch checked={settings.nearLimit} onChange={(v) => set("nearLimit", v)} label="Add a note when I'm near a limit" disabled={off} />
          </Row>
          <Row inset title="Once usage passes">
            <Popup
              value={settings.nearLimitAt}
              options={NEAR}
              onChange={(v) => set("nearLimitAt", v)}
              label="Once usage passes"
              disabled={off || !settings.nearLimit}
            />
          </Row>
          <Row
            inset
            title="Send it"
            detail={
              settings.nearLimitRightAway
                ? "Sessions that are working get it after their next step. Idle ones get it with your next message."
                : "With your next message to each session."
            }
          >
            <Popup
              value={settings.nearLimitRightAway ? "now" : "message"}
              options={SEND}
              onChange={(v) => set("nearLimitRightAway", v === "now")}
              label="Send it"
              disabled={off || !settings.nearLimit}
            />
          </Row>
          <div className="row inset stacked">
            <div className="detail">Ask Claude to:</div>
            <MessageField settings={settings} set={set} disabled={off || !settings.nearLimit} />
          </div>
          <div className="row inset stacked">
            <div className="detail">What Claude sees, at {settings.nearLimitAt}% or more:</div>
            <pre className="preview-note">{noteExample(app, settings)}</pre>
          </div>
        </div>
      </Section>
    </>
  );
}
