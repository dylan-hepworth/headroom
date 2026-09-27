import type { PaneProps } from "../App";
import { bridge } from "../bridge";
import { Bell, Chat, Check, Gauge, Layers, Refresh } from "../icons";
import { Popup, Row, Section, Switch } from "../ui";

const LEVELS = [
  ["off", "Off"],
  [50, "50%"],
  [60, "60%"],
  [70, "70%"],
  [80, "80%"],
  [90, "90%"],
  [95, "95%"],
] as const;
const CONTEXT_LEVELS = [
  ["off", "Off"],
  [50, "50%"],
  [60, "60%"],
  [70, "70%"],
  [80, "80%"],
  [90, "90%"],
] as const;
const RUNS = [
  [0, "any length"],
  [60, "1 minute"],
  [300, "5 minutes"],
  [900, "15 minutes"],
] as const;

export default function Alerts({ settings, set }: PaneProps) {
  return (
    <>
      {settings.paused && (
        <Section>
          <div className="group">
            <Row
              tile={["gray", <Bell />]}
              title={settings.paused.until ? `Paused until ${settings.paused.until}` : "Paused"}
              detail="No alerts go out, and requests go to each session's own prompt instead of the menu bar. Pause again from the menu bar's menu."
            >
              <button className="btn" onClick={() => set("paused", null)}>
                Resume
              </button>
            </Row>
          </div>
        </Section>
      )}
      <Section title="Usage">
        <div className="group">
          <Row tile={["clay", <Gauge />]} title="5-hour limit" detail="Once per window, when usage passes this.">
            <Popup value={settings.alert5h} options={LEVELS} onChange={(v) => set("alert5h", v)} label="5-hour limit alert" />
          </Row>
          <Row tile={["clay", <Gauge />]} title="Weekly limit" detail="Once per week, when usage passes this.">
            <Popup value={settings.alert7d} options={LEVELS} onChange={(v) => set("alert7d", v)} label="Weekly limit alert" />
          </Row>
          <Row
            tile={["green", <Refresh />]}
            title="When a limit resets"
            detail="Only if a Claude Code session stopped because it hit the limit, so you know you can pick it back up."
          >
            <Switch checked={settings.resumeAlert} onChange={(v) => set("resumeAlert", v)} label="When a limit resets" disabled={!settings.hooks} />
          </Row>
        </div>
      </Section>

      <Section title="Sessions" note={settings.hooks ? undefined : "These need Hooks turned on, so Headroom hears from Claude Code."}>
        <div className="group">
          <Row
            tile={["yellow", <Chat />]}
            title="A session needs an answer"
            detail="It's asking permission, or asking you a question. Not when the menu bar panel opens for it, or it's the chat you have open."
          >
            <Switch
              checked={settings.notifyWaiting}
              onChange={(v) => set("notifyWaiting", v)}
              label="A session needs an answer"
              disabled={!settings.hooks}
            />
          </Row>
          <Row
            tile={["blue", <Check />]}
            title="A session finishes"
            detail="It's done and waiting for your next message. Shows the start of Claude's reply."
          >
            <Switch checked={settings.notifyDone} onChange={(v) => set("notifyDone", v)} label="A session finishes" disabled={!settings.hooks} />
          </Row>
          <Row
            inset
            title="Only when I'm not looking at it"
            detail="Skipped while its terminal is in front, or its chat is the one open in the Claude app."
          >
            <Switch
              checked={settings.doneOnlyAway}
              onChange={(v) => set("doneOnlyAway", v)}
              label="Only when I'm not looking at it"
              disabled={!settings.hooks || !settings.notifyDone}
            />
          </Row>
          <Row inset title="Only after runs of at least">
            <Popup
              value={settings.doneMinRun}
              options={RUNS}
              onChange={(v) => set("doneMinRun", v)}
              label="Only after runs of at least"
              disabled={!settings.hooks || !settings.notifyDone}
            />
          </Row>
        </div>
      </Section>

      <Section title="Conversations">
        <div className="group">
          <Row
            tile={["purple", <Layers />]}
            title="Context is filling up"
            detail="When a conversation's context passes this share of its window, once until it's compacted. Claude Code compacts on its own at about 97% of a 1M window, or 83% of a 200k one."
          >
            <Popup value={settings.contextAlert} options={CONTEXT_LEVELS} onChange={(v) => set("contextAlert", v)} label="Context alert" />
          </Row>
        </div>
      </Section>

      <Section note="macOS asks whether Headroom can send notifications. If you missed it, Send Test Alert asks again.">
        <div className="group">
          <Row
            tile={["gray", <Bell />]}
            title="Keep alerts on screen"
            detail={settings.persistentAlerts ? "They stay until you close them." : "Headroom clears each one after a few seconds."}
          >
            <Switch checked={settings.persistentAlerts} onChange={(v) => set("persistentAlerts", v)} label="Keep alerts on screen" />
          </Row>
          {settings.persistentAlerts && (
            <Row
              inset
              title="macOS has to allow it too"
              detail="Set Headroom's alert style to Persistent in System Settings, if it isn't already. Apps can't change that themselves."
            >
              <button className="btn" onClick={() => bridge.openNotificationSettings()}>
                Open Settings…
              </button>
            </Row>
          )}
          <Row tile={["red", <Bell />]} title="Test alert" detail="Sends one now, so you can see how they look.">
            <button className="btn" onClick={() => bridge.testAlert()}>
              Send Test Alert
            </button>
          </Row>
        </div>
      </Section>
    </>
  );
}
