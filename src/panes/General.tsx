import { useState } from "react";
import type { PaneProps } from "../App";
import { bridge, type Limit, type LimitKey } from "../bridge";
import { Clock, Download, Info, Key, Mic, Power, Refresh } from "../icons";
import { Dial, PaceArrow, paceOf, Popup, Row, Section, Switch, useDialReading } from "../ui";
import { ShortcutField } from "../Voice";

const INTERVALS = [
  [30, "30 seconds"],
  [60, "1 minute"],
  [300, "5 minutes"],
  [600, "10 minutes"],
  [1800, "30 minutes"],
] as const;

const EVERY: Record<number, string> = {
  30: "every 30 seconds",
  60: "every minute",
  300: "every 5 minutes",
  600: "every 10 minutes",
  1800: "every 30 minutes",
};

/** The smaller limits beside the weekly one, in this order: the 5-hour limit, then each model's own weekly limit. */
const SMALL_ORDER: LimitKey[] = ["5h", "7d_opus", "7d_sonnet"];

/** Where the weekly limit stands against the time gone in the week: a short verdict, and how it's colored. */
function pace(weekly: Limit): { text: string; tone: "good" | "warn" | "bad" } | null {
  if (weekly.pct >= 95) return { text: "Nearly out", tone: "bad" };
  if (weekly.pace) return { text: `Full by ${weekly.pace} at this pace`, tone: "warn" };
  if (weekly.elapsed === undefined) return null;
  return paceOf(weekly) === "ahead" ? { text: "Ahead of the week", tone: "warn" } : { text: "On pace", tone: "good" };
}

/** One of the smaller limits: its own ring, what it is, and when it resets. */
function SmallLimit({ limit, index, arrows }: { limit: Limit; index: number; arrows: boolean }) {
  const reading = useDialReading(limit.pct, limit.key, 160 * (index + 1));
  const five = limit.key === "5h";
  const name = five ? "5-hour" : limit.label;
  return (
    <div className="hero-small">
      <div className="hero-small-ring">
        <Dial pct={reading.pct} from={reading.from} size={54} tone={five ? "five" : "model"} />
        {arrows && (
          <div className="hero-small-arrow" style={{ opacity: reading.label }}>
            <PaceArrow limit={limit} size={16} />
          </div>
        )}
      </div>
      <div className="hero-small-text">
        <div className="hero-small-top">
          <span className="hero-small-name">{name}</span>
          <span className="hero-small-pct" style={{ opacity: reading.label }}>
            {Math.round(reading.value)}%
          </span>
        </div>
        <div className="hero-small-sub">{limit.resets && `Resets ${limit.resets}`}</div>
      </div>
    </div>
  );
}

export default function General({ app, settings, set }: PaneProps) {
  const weekly = app.limits.find((l) => l.key === "7d");
  const small = [
    ...SMALL_ORDER.map((key) => app.limits.find((l) => l.key === key)).filter((l) => l !== undefined),
    ...app.limits.filter((l) => l.key !== "7d" && !SMALL_ORDER.includes(l.key)),
  ];
  const reading = useDialReading(weekly?.pct, "7d");
  const verdict = weekly && pace(weekly);

  return (
    <>
      <div className="hero">
        <div className="hero-top">
          <div className="hero-ring">
            <Dial pct={reading.pct} from={reading.from} size={144} />
            <div className="hero-center" style={{ opacity: reading.label }}>
              <span className="hero-pct">{weekly ? `${Math.round(reading.value)}%` : ""}</span>
              <span className="hero-pct-caption">
                {settings.paceArrows && weekly && <PaceArrow limit={weekly} size={13} />}
                used
              </span>
            </div>
          </div>
          <div className="hero-text">
            <div className="hero-title">Weekly limit</div>
            <div className="hero-sub">
              {weekly
                ? [weekly.resets && `Resets ${weekly.resets}`, weekly.elapsed !== undefined && `${weekly.elapsed}% of the week gone`]
                    .filter(Boolean)
                    .join(" · ")
                : app.status}
            </div>
            {verdict && (
              <div className={`hero-chip ${verdict.tone}`}>
                <i />
                {verdict.text}
              </div>
            )}
          </div>
        </div>
        {small.length > 0 && (
          <div className="hero-smalls">
            {small.map((l, i) => (
              <SmallLimit key={l.key} limit={l} index={i} arrows={settings.paceArrows} />
            ))}
          </div>
        )}
        <div className="hero-footer">
          <div className="hero-note">Headroom checks {EVERY[settings.interval] ?? "regularly"} and shows this in the menu bar.</div>
          <button className="btn" onClick={() => bridge.refresh()}>
            Refresh Now
          </button>
          <button className="btn primary" onClick={() => bridge.openUsagePage()}>
            Open Usage Page
          </button>
        </div>
      </div>

      <SignIn app={app} />

      <Section title="General">
        <div className="group">
          <Row tile={["gray", <Power />]} title="Open at login" detail="Waits in the menu bar.">
            <Switch checked={settings.launchAtLogin} onChange={(v) => set("launchAtLogin", v)} label="Open at login" />
          </Row>
          <Row
            tile={["green", <Clock />]}
            title="Check usage every"
            info="With a token, each check sends a one-token request to Claude Haiku, which uses a sliver of your limit. Five minutes or longer keeps that negligible."
          >
            <Popup value={settings.interval} options={INTERVALS} onChange={(v) => set("interval", v)} label="Check usage every" />
          </Row>
          <Row
            tile={["red", <Mic />]}
            title="Talk to a chat"
            detail="From anywhere: the shortcut drops a panel from the menu bar that listens, and sends what you say to the chat you last followed."
          >
            <ShortcutField keys={settings.talkShortcut} onChange={(keys) => set("talkShortcut", keys)} />
          </Row>
        </div>
      </Section>

      <Section
        title="About"
        note="Not affiliated with Anthropic. Provided as is, under the Apache 2.0 license, with no warranty: you use it at your own risk."
      >
        <div className="group">
          <Row tile={["gray", <Info />]} title="Version">
            <span className="value">{app.version}</span>
          </Row>
          <Row
            tile={["blue", <Download />]}
            title="Updates"
            detail={app.update ? `Version ${app.update} is ready to install.` : "Checks GitHub for a newer release."}
          >
            {app.update ? (
              <button className="btn primary" onClick={() => bridge.installUpdate()}>
                Install and Restart
              </button>
            ) : (
              <button className="btn" onClick={() => bridge.checkUpdates()}>
                Check Now
              </button>
            )}
          </Row>
          <Row
            tile={["green", <Refresh />]}
            title="Check for updates automatically"
            detail="At launch and once a day. An update only installs when you choose."
          >
            <Switch checked={settings.autoUpdate} onChange={(v) => set("autoUpdate", v)} label="Check for updates automatically" />
          </Row>
        </div>
      </Section>
    </>
  );
}

/** How Headroom reads usage: a saved token, or Claude Code's login. The token is typed (well, pasted) here and goes
 *  straight to the keychain; the window never gets it back. */
function SignIn({ app }: { app: PaneProps["app"] }) {
  const [editing, setEditing] = useState(false);
  const [token, setToken] = useState("");
  const [message, setMessage] = useState("");
  const hasToken = app.signIn === "token";

  const save = async () => {
    try {
      await bridge.saveToken(token);
      setToken("");
      setEditing(false);
      setMessage("");
    } catch (e) {
      setMessage(String(e));
    }
  };
  const remove = async () => {
    try {
      await bridge.clearToken();
      setEditing(false);
      setMessage("");
    } catch (e) {
      setMessage(String(e));
    }
  };

  return (
    <Section
      title="Sign In"
      note="A token from claude setup-token lasts about a year. Without one, Headroom borrows Claude Code's login, which expires every few hours."
    >
      <div className="group">
        <Row
          tile={["yellow", <Key />]}
          title={hasToken ? "Claude token" : "Claude Code login"}
          detail={
            <>
              <span className={app.ok ? "dot green" : "dot red"} />
              {app.ok ? (hasToken ? "Saved in your keychain." : "Borrowed from Claude Code.") : app.status}
            </>
          }
        >
          <button className="btn" onClick={() => setEditing((e) => !e)}>
            {editing ? "Cancel" : hasToken ? "Change…" : "Set Token…"}
          </button>
        </Row>
        {editing && (
          <div className="token-editor">
            <p>
              Run <code>claude setup-token</code> in Terminal, approve it in your browser, and paste the token it prints.
            </p>
            <input
              className="field token-field"
              type="password"
              placeholder="sk-ant-oat01-…"
              autoFocus
              spellCheck={false}
              value={token}
              onChange={(e) => setToken(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") save();
                if (e.key === "Escape") setEditing(false);
              }}
            />
            {message && <p className="token-error">{message}</p>}
            <div className="token-actions">
              {hasToken && (
                <button className="link danger" onClick={remove}>
                  Remove Token
                </button>
              )}
              <button className="btn primary" onClick={save} disabled={!token.trim()}>
                Save
              </button>
            </div>
          </div>
        )}
      </div>
    </Section>
  );
}
