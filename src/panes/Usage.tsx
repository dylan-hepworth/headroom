import type { PaneProps } from "../App";
import { Folder } from "../icons";
import { Meter, Section } from "../ui";

export default function Usage({ app }: PaneProps) {
  const biggest = Math.max(1, ...app.shares.map((s) => s.pct));
  return (
    <>
      <Section
        title="Limits"
        note="The tick shows how far through each window you are. Usage ahead of it is running faster than the window, and Headroom says when you'd hit the limit at that pace."
      >
        <div className="group">
          {app.limits.length === 0 && (
            <div className="empty">
              <div>{app.status}</div>
            </div>
          )}
          {app.limits.map((l) => (
            <div className="row limit" key={l.key}>
              <div className="label">
                <div className="limit-head">
                  <span className="title">{l.label}</span>
                  <span className="limit-pct">{l.pct}%</span>
                </div>
                <Meter pct={l.pct} marker={l.elapsed} />
                <div className="detail">
                  {l.resets ? `Resets ${l.resets}` : "No reset time"}
                  {l.pace ? (
                    <span className="pace hot">· At this pace, you'll hit it by {l.pace}</span>
                  ) : (
                    <span className="pace">· On pace to stay under</span>
                  )}
                </div>
              </div>
            </div>
          ))}
        </div>
      </Section>

      <Section
        title="Claude Code This Window"
        note="Each project's share of your Claude Code use on this Mac in the current 5-hour window. It's estimated from the token counts in Claude Code's transcripts, so it doesn't include claude.ai or other Macs."
      >
        <div className="group">
          {app.shares.length === 0 && (
            <div className="empty">
              <div>No Claude Code use in this window yet.</div>
            </div>
          )}
          {app.shares.map((s) => (
            <div className="row share" key={s.project}>
              <span className="setting-tile small clay">
                <Folder />
              </span>
              <span className="share-name">{s.project}</span>
              <Meter pct={(s.pct / biggest) * 100} />
              <span className="share-pct">{s.pct < 1 ? "<1" : Math.round(s.pct)}%</span>
            </div>
          ))}
        </div>
      </Section>
    </>
  );
}
