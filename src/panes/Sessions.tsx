import { useEffect, useState } from "react";
import type { PaneProps } from "../App";
import { Avatar, choosePicture, IconPicker, type ChatIcon } from "../Avatar";
import { bridge, needsAnswer, yourTurn, type Session } from "../bridge";
import { Hook, Terminal } from "../icons";
import { Section } from "../ui";

const STATE_TEXT: Record<Session["state"], string> = {
  permission: "Needs permission",
  question: "Has a question",
  waiting: "Waiting for you",
  working: "Working",
  limited: "Stopped at the limit",
  idle: "Idle",
};

/** How long ago, as short as it can be said: "now", "4m", "2h". */
function ago(ms: number, now: number) {
  const mins = Math.floor((now - ms) / 60_000);
  if (mins < 1) return "now";
  if (mins < 60) return `${mins}m`;
  return `${Math.floor(mins / 60)}h`;
}

/** The current time, updated every 30 seconds, so "4m" keeps up without a new report from the app. */
function useNow() {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(timer);
  }, []);
  return now;
}

function SessionRow({ s, now }: { s: Session; now: number }) {
  const held = s.held;
  // A request waiting in Headroom shows itself, since that's the one the buttons answer
  const ask = held ?? s.request;
  const [picking, setPicking] = useState(false);
  const pick = async (icon: ChatIcon | "picture" | null, wholeProject: boolean) => {
    setPicking(false);
    const picked = icon === "picture" ? await choosePicture() : icon;
    if (icon !== "picture" || picked) bridge.setIcon(s.id, picked, wholeProject);
  };
  return (
    <>
      <div className={`row session ${s.state}`}>
        {s.icon ? (
          <Avatar
            icon={s.icon}
            size={28}
            corner={<span className={`state-dot ${s.state}`} />}
            onClick={() => setPicking(!picking)}
            title="Change its icon"
          />
        ) : (
          <span className={`state-dot ${s.state}`} />
        )}
        <div className="label">
          <div className="title">
            {s.title ?? s.project}
            <span className="session-state">{[STATE_TEXT[s.state], ago(s.since, now), s.title && s.project].filter(Boolean).join(" · ")}</span>
          </div>
          {ask && (
            <div className="request">
              <span className="request-tool">{held?.kind === "question" ? "Question" : ask.tool}</span>
              {ask.detail && <code>{ask.detail}</code>}
            </div>
          )}
          {s.last && <div className="detail quote">{s.last}</div>}
          {s.activity && <div className="detail">{s.activity}</div>}
          {!ask && !s.last && !s.activity && <div className="detail">{s.path}</div>}
        </div>
        {held?.kind === "question" && (
          <div className="row-end">
            <button className="btn primary" onClick={() => bridge.showPopover()}>
              Answer…
            </button>
          </div>
        )}
        {held?.kind === "permission" && (
          <div className="row-end">
            <button className="btn" onClick={() => bridge.answer(held.id, "deny")}>
              Deny
            </button>
            {held.canSession && (
              <button className="btn" onClick={() => bridge.answer(held.id, "session")}>
                Allow for Session
              </button>
            )}
            <button className="btn primary" onClick={() => bridge.answer(held.id, "allow")}>
              Allow
            </button>
          </div>
        )}
      </div>
      {picking && s.icon && (
        <div className="pop-vars session-picker">
          <IconPicker
            icon={s.icon}
            project={s.project}
            onPick={pick}
            onPicture={(whole) => pick("picture", whole)}
            onReset={() => pick(null, false)}
          />
        </div>
      )}
    </>
  );
}

export default function Sessions({ app, settings, set }: PaneProps) {
  const now = useNow();
  if (!settings.hooks) {
    return (
      <div className="group">
        <div className="empty">
          <span className="setting-tile purple">
            <Hook />
          </span>
          <div className="title">Headroom isn't hearing from Claude Code</div>
          <div>Turn on Hooks to see which sessions are working and which are waiting on you.</div>
          <button className="btn primary" onClick={() => set("hooks", true)}>
            Turn On Hooks
          </button>
        </div>
      </div>
    );
  }
  if (app.sessions.length === 0) {
    return (
      <div className="group">
        <div className="empty">
          <span className="setting-tile orange">
            <Terminal />
          </span>
          <div className="title">No sessions yet</div>
          <div>Claude Code sessions show up here once they start. Any that were already running need a restart first.</div>
        </div>
      </div>
    );
  }

  const needYou = app.sessions.filter((s) => needsAnswer(s) || yourTurn(s));
  const rest = app.sessions.filter((s) => !needYou.includes(s));
  return (
    <>
      {needYou.length > 0 && (
        <Section
          title="Needs You"
          note={
            needYou.some((s) => s.held) ? "Your answer goes straight to Claude Code, the same as answering its prompt in the terminal." : undefined
          }
        >
          <div className="group">
            {needYou.map((s) => (
              <SessionRow key={s.id} s={s} now={now} />
            ))}
          </div>
        </Section>
      )}
      {rest.length > 0 && (
        <Section title={needYou.length > 0 ? "Other Sessions" : "Sessions"}>
          <div className="group">
            {rest.map((s) => (
              <SessionRow key={s.id} s={s} now={now} />
            ))}
          </div>
        </Section>
      )}
      <p className="section-note">Sessions that go quiet for 12 hours are dropped from the list.</p>
    </>
  );
}
