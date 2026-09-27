import { useEffect, useRef, useState, type CSSProperties, type RefObject } from "react";
import type { PaneProps } from "../App";
import { bridge, needsAnswer, yourTurn, type Wallpaper } from "../bridge";
import { Hourglass, MenuBar as MenuBarIcon, Pace, Sparkle } from "../icons";
import { paceOf, Popup, Ring, Row, Section, Switch } from "../ui";

const MODES = [
  ["both", "5-hour and weekly"],
  ["5h", "5-hour only"],
  ["7d", "Weekly only"],
  ["5h_reset", "5-hour and time until reset"],
  ["ring", "Ring only"],
  ["rings", "Two rings, 5-hour and weekly"],
] as const;

/** A model's own weekly limit, like Fable's, if Claude reports one for the plan. */
function modelLimit(app: PaneProps["app"]) {
  return app.limits.find((l) => !["5h", "7d", "7d_opus", "7d_sonnet"].includes(l.key));
}

function title(app: PaneProps["app"], mode: PaneProps["settings"]["title"], arrows: boolean) {
  const pct = (key: string) => {
    const l = app.limits.find((l) => l.key === key);
    const pace = l && arrows ? paceOf(l) : null;
    return l ? `${l.pct}%${pace === "ahead" ? "↑" : pace === "behind" ? "↓" : ""}` : "–";
  };
  if (mode === "5h") return pct("5h");
  if (mode === "7d") return pct("7d");
  if (mode === "5h_reset") return `${pct("5h")} · 2h 14m`;
  return `${pct("5h")} · ${pct("7d")}`;
}

/** The menu bar's title with its pace arrows in their colors. */
function arrowsIn(text: string) {
  return text.split(/([↑↓])/).map((part, i) =>
    part === "↑" ? (
      <span key={i} className="title-arrow ahead">
        ↑
      </span>
    ) : part === "↓" ? (
      <span key={i} className="title-arrow behind">
        ↓
      </span>
    ) : (
      part
    ),
  );
}

/** The wallpaper behind the preview, sized and placed so the preview shows the top right corner of the screen at its
 *  real size: macOS scales the wallpaper to fill the screen, keeping its shape and centering it, and so do we. Also
 *  how tall the real menu bar is. Until the wallpaper arrives (or in a browser), the preview keeps its gradient. */
function useWallpaper(box: RefObject<HTMLDivElement | null>) {
  const [wall, setWall] = useState<Wallpaper | null>(null);
  const [image, setImage] = useState<{ width: number; height: number } | null>(null);
  const [boxWidth, setBoxWidth] = useState(0);

  useEffect(() => {
    bridge.wallpaper().then((w) => {
      setWall(w);
      if (!w?.url) return;
      const img = new Image();
      img.onload = () => setImage({ width: img.naturalWidth, height: img.naturalHeight });
      img.src = w.url;
    });
  }, []);
  useEffect(() => {
    if (!box.current) return;
    const watch = new ResizeObserver(([entry]) => setBoxWidth(entry.contentRect.width));
    watch.observe(box.current);
    return () => watch.disconnect();
  }, [box]);

  const menuBar = wall?.menuBar || 24;
  if (!wall?.url || !image || !boxWidth) return { menuBar, style: undefined };
  const scale = Math.max(wall.width / image.width, wall.height / image.height);
  const width = image.width * scale;
  const height = image.height * scale;
  // Where the screen's top right corner lands on the scaled wallpaper, minus the width of the preview
  const left = (width - wall.width) / 2 + wall.width - boxWidth;
  const top = (height - wall.height) / 2;
  const style: CSSProperties = {
    backgroundImage: `url(${wall.url})`,
    backgroundSize: `${width}px ${height}px`,
    backgroundPosition: `${-left}px ${-top}px`,
  };
  return { menuBar, style };
}

export default function MenuBar({ app, settings, set }: PaneProps) {
  const preview = useRef<HTMLDivElement>(null);
  const wallpaper = useWallpaper(preview);
  const answer = app.sessions.filter(needsAnswer).length;
  const turn = app.sessions.filter(yourTurn).length;
  const highest = Math.max(0, ...app.limits.map((l) => l.pct));
  const pct = (key: string) => app.limits.find((l) => l.key === key)?.pct ?? 0;
  const pace = (key: string) => {
    const l = app.limits.find((l) => l.key === key);
    return settings.paceArrows && l ? paceOf(l) : null;
  };
  const model = modelLimit(app);
  const modes = model ? [...MODES, ["rings_model", `Three rings, with ${model.label.replace(/ weekly$/, "")}`] as const] : MODES;
  const ringsOnly = settings.title === "ring" || settings.title === "rings" || settings.title === "rings_model";
  return (
    <>
      <div className="menubar-preview" aria-label="Preview" ref={preview} style={wallpaper.style}>
        <div className="menubar-strip" style={{ height: wallpaper.menuBar }}>
          <span className="menubar-item">
            {settings.title === "rings" || settings.title === "rings_model" ? (
              <>
                <Ring pct={pct("5h")} size={15} stroke={2} fiveHour pace={pace("5h")} />
                <Ring pct={pct("7d")} size={15} stroke={2} pace={pace("7d")} />
                {settings.title === "rings_model" && model && (
                  <Ring pct={model.pct} size={15} stroke={2} model pace={settings.paceArrows ? paceOf(model) : null} />
                )}
              </>
            ) : settings.ring || ringsOnly ? (
              <Ring pct={pct("7d")} critical={highest >= 95} size={15} stroke={2} pace={pace("7d")} />
            ) : (
              <span>✻</span>
            )}
            {!ringsOnly && <span>{arrowsIn(app.menuTitle || title(app, settings.title, settings.paceArrows))}</span>}
            {settings.hooks && settings.waitingCount && answer > 0 && (
              <span className="menubar-count">
                <i className="count-dot answer" />
                {answer}
              </span>
            )}
            {settings.hooks && settings.waitingCount && turn > 0 && (
              <span className="menubar-count">
                <i className="count-dot turn" />
                {turn}
              </span>
            )}
          </span>
          <span className="menubar-clock">Fri Sep 25 2:41 PM</span>
        </div>
      </div>

      <Section>
        <div className="group">
          <Row tile={["blue", <MenuBarIcon />]} title="Shows">
            <Popup value={settings.title} options={modes} onChange={(v) => set("title", v)} label="Menu bar shows" />
          </Row>
          <Row
            tile={["clay", <Sparkle />]}
            title="Usage ring"
            detail={
              ringsOnly
                ? "Always on with the ring options above."
                : settings.ring
                  ? "Fills with your weekly limit, and turns red when either limit passes 95%."
                  : "Off. A ✻ shows instead, and turns 🟠 at 80% and 🔴 at 95%."
            }
          >
            <Switch checked={settings.ring || ringsOnly} onChange={(v) => set("ring", v)} label="Usage ring" disabled={ringsOnly} />
          </Row>
          <Row
            tile={["orange", <Pace />]}
            title="Pace arrows"
            detail="Up in orange when a limit is being used faster than its window is going by, down in green when slower. In the menu bar, and in the rings at the top of Settings."
          >
            <Switch checked={settings.paceArrows} onChange={(v) => set("paceArrows", v)} label="Pace arrows" />
          </Row>
          <Row
            tile={["orange", <Hourglass />]}
            title="Sessions waiting on you"
            detail={
              settings.hooks
                ? "A yellow dot for sessions that need an answer, and blue for ones that are done and waiting for you."
                : "Needs Hooks turned on."
            }
          >
            <Switch
              checked={settings.waitingCount}
              onChange={(v) => set("waitingCount", v)}
              label="Sessions waiting on you"
              disabled={!settings.hooks}
            />
          </Row>
        </div>
      </Section>
    </>
  );
}
