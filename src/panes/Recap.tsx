import type { PaneProps } from "../App";
import { BrandMark, Calendar } from "../icons";
import { Popup, Row, Section, Switch } from "../ui";

const TIMES = [
  ["17:00", "5:00 PM"],
  ["18:00", "6:00 PM"],
  ["19:00", "7:00 PM"],
  ["21:00", "9:00 PM"],
] as const;

export default function Recap({ app, settings, set }: PaneProps) {
  const r = app.recap;
  return (
    <>
      <Section>
        <div className="group">
          <Row tile={["teal", <Calendar />]} title="Daily recap" detail="A notification at the end of the day with what you got through.">
            <Switch checked={settings.recap} onChange={(v) => set("recap", v)} label="Daily recap" />
          </Row>
          <Row inset title="Send it at">
            <Popup value={settings.recapAt} options={TIMES} onChange={(v) => set("recapAt", v)} label="Send it at" disabled={!settings.recap} />
          </Row>
        </div>
      </Section>

      <Section title="Today So Far" note="From Claude Code's transcripts on this Mac. Days without any Claude Code use are skipped.">
        {r ? (
          <div className="notification">
            <BrandMark size={34} />
            <div>
              <div className="notification-head">
                <b>Headroom</b>
                <span>now</span>
              </div>
              <div className="notification-title">{r.title}</div>
              <div className="notification-body">{r.body}</div>
            </div>
          </div>
        ) : (
          <div className="group">
            <div className="empty">
              <div>No Claude Code use yet today.</div>
            </div>
          </div>
        )}
      </Section>
    </>
  );
}
