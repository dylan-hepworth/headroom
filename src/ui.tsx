// The building blocks every pane is made of: sections, rows, switches, pop-up menus, the rings and meters, and the
// search field.

import type { Limit } from "./bridge";
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { Search } from "./icons";

export type TileColor = "gray" | "blue" | "orange" | "purple" | "teal" | "green" | "red" | "yellow" | "clay";

export function Switch({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return <button className="switch" role="switch" aria-checked={checked} aria-label={label} disabled={disabled} onClick={() => onChange(!checked)} />;
}

/** A native pop-up menu, drawn like a macOS pop-up button. */
export function Popup<T extends string | number>({
  value,
  options,
  onChange,
  label,
  disabled,
}: {
  value: T;
  options: readonly (readonly [T, string])[];
  onChange: (v: T) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <select
      className="popup"
      value={String(value)}
      aria-label={label}
      disabled={disabled}
      onChange={(e) => {
        const picked = options.find(([v]) => String(v) === e.target.value);
        if (picked) onChange(picked[0]);
      }}
    >
      {options.map(([v, text]) => (
        <option key={String(v)} value={String(v)}>
          {text}
        </option>
      ))}
    </select>
  );
}

export function Section({ title, note, children }: { title?: string; note?: ReactNode; children: ReactNode }) {
  return (
    <section>
      {title && <h3 className="section-title">{title}</h3>}
      {children}
      {note && <p className="section-note">{note}</p>}
    </section>
  );
}

export function Row({
  tile,
  inset,
  title,
  detail,
  info,
  children,
}: {
  /** The colored square at the start of the row, like System Settings has. */
  tile?: [TileColor, ReactNode];
  /** A setting that belongs to the row above: no tile, lined up with its text. */
  inset?: boolean;
  title: ReactNode;
  detail?: ReactNode;
  /** The longer explanation, behind an ⓘ beside the title. */
  info?: string;
  children?: ReactNode;
}) {
  return (
    <div className={tile ? "row with-tile" : inset ? "row inset" : "row"}>
      {tile && <span className={`setting-tile ${tile[0]}`}>{tile[1]}</span>}
      <div className="label">
        <div className="title">
          {title}
          {info && <Info text={info} />}
        </div>
        {detail && <div className="detail">{detail}</div>}
      </div>
      {children ? <div className="row-end">{children}</div> : null}
    </div>
  );
}

/** An ⓘ that shows a setting's full explanation while pointed at or focused. The card is placed on the window, since
 *  groups clip what spills out. */
function Info({ text }: { text: string }) {
  const [at, setAt] = useState<{ x: number; y: number; below: boolean } | null>(null);
  const show = (el: HTMLElement) => {
    const r = el.getBoundingClientRect();
    const x = Math.min(Math.max(r.left + r.width / 2, 140), window.innerWidth - 140);
    const below = r.top < 120;
    setAt({ x, y: below ? r.bottom + 7 : r.top - 7, below });
  };
  return (
    <span
      className="info"
      tabIndex={0}
      role="note"
      aria-label={text}
      onMouseEnter={(e) => show(e.currentTarget)}
      onMouseLeave={() => setAt(null)}
      onFocus={(e) => show(e.currentTarget)}
      onBlur={() => setAt(null)}
    >
      <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden>
        <circle cx="8" cy="8" r="6.6" fill="none" stroke="currentColor" strokeWidth="1.3" />
        <circle cx="8" cy="4.9" r="0.95" fill="currentColor" />
        <path d="M8 7.2v4.4" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
      </svg>
      {at && (
        <span className="info-card" aria-hidden style={{ left: at.x, top: at.y, transform: `translate(-50%, ${at.below ? "0" : "-100%"})` }}>
          {text}
        </span>
      )}
    </span>
  );
}

/** Whether a limit is being used faster or slower than its window is going by: "ahead" once usage is more than 5 points
 *  past the share of the window that's gone, "behind" once it's more than 5 short. Close to even, neither. The app works
 *  it out, so these always match the menu bar's. */
export function paceOf(limit: Limit): "ahead" | "behind" | null {
  return limit.paceArrow ?? null;
}

/** The arrow in a ring for its pace: up in orange when a limit is ahead of its window, down in green when it's behind. */
export function PaceArrow({ limit, size }: { limit: Limit; size: number }) {
  const pace = paceOf(limit);
  if (!pace) return null;
  const tip = `${limit.pct}% used, ${limit.elapsed}% of the ${limit.key === "5h" ? "window" : "week"} gone`;
  return (
    <svg
      className={`pace-arrow ${pace}`}
      width={size}
      height={size}
      viewBox="0 0 12 12"
      aria-label={pace === "ahead" ? `Ahead: ${tip}` : `Behind: ${tip}`}
    >
      <title>{pace === "ahead" ? `Ahead of pace: ${tip}` : `Behind pace: ${tip}`}</title>
      <path d={pace === "ahead" ? "M6 10V2.5M2.8 5.7L6 2.5l3.2 3.2" : "M6 2v7.5M2.8 6.3L6 9.5l3.2-3.2"} />
    </svg>
  );
}

/** The usage ring, as the menu bar draws it: fills clockwise from the top, and turns red at 95%. */
export function Ring({
  pct,
  size = 18,
  stroke,
  fiveHour,
  model,
  critical,
  pace,
}: {
  pct: number;
  size?: number;
  stroke?: number;
  fiveHour?: boolean;
  /** A model's own weekly limit, in violet. */
  model?: boolean;
  /** Red whatever it's filled to, because another limit is nearly used up. */
  critical?: boolean;
  /** Its pace arrow inside, as the menu bar draws it. */
  pace?: "ahead" | "behind" | null;
}) {
  const w = stroke ?? Math.max(2, size * 0.12);
  const r = (size - w) / 2;
  const c = 2 * Math.PI * r;
  const filled = Math.min(100, Math.max(0, pct)) / 100;
  return (
    <svg
      className={["ring", fiveHour && "five-hour", model && "model", (critical || pct >= 95) && "critical"].filter(Boolean).join(" ")}
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      aria-hidden
    >
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--track)" strokeWidth={w} />
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        stroke="currentColor"
        strokeWidth={w}
        strokeLinecap="round"
        strokeDasharray={`${c * filled} ${c}`}
        transform={`rotate(-90 ${size / 2} ${size / 2})`}
      />
      {pace && (
        <path
          className={`pace-arrow ${pace}`}
          transform={`translate(${size / 2} ${size / 2}) scale(${size / 30})${pace === "behind" ? " rotate(180)" : ""}`}
          d="M0 7V-5M-4.5 -0.5L0 -5l4.5 4.5"
          style={{ strokeWidth: 3 }}
        />
      )}
    </svg>
  );
}

/** A number that eases from 0 up to `target` when it first shows, and from wherever it is to the new target when it
 *  changes. It jumps straight there when the Mac is set to reduce motion. */
export function useCountUp(target: number, ms = 1200) {
  const [value, setValue] = useState(0);
  const from = useRef(0);
  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      from.current = target;
      setValue(target);
      return;
    }
    const start = performance.now();
    const begin = from.current;
    let frame = requestAnimationFrame(function step(now) {
      const t = Math.min(1, (now - start) / ms);
      const eased = 1 - Math.pow(1 - t, 3);
      from.current = begin + (target - begin) * eased;
      setValue(from.current);
      if (t < 1) frame = requestAnimationFrame(step);
    });
    return () => cancelAnimationFrame(frame);
  }, [target, ms]);
  return value;
}

/** Where the band sits in the app icon: from just past the bottom, clockwise round through the left and the top, to
 *  about half past one. In degrees clockwise from 12 o'clock. */
const ICON_BAND = { from: 172, sweep: 230 };
/** How long the ring stays as the icon draws it before it swings round to the reading, and how long the swing takes. */
const INTRO_HOLD = 450;
const INTRO_MS = 1100;

/** The rings whose opening has played in this window, by name. Each plays once each time Settings opens, not on every
 *  visit to the pane. */
const introPlayed = new Set<string>();

/** What a ring shows for `target` (a percentage, or undefined until there's a reading). The first time, it's the band
 *  as the app icon draws it, with no number; once there's a reading, the band swings round to start at the top and
 *  fill to it while the number fades in and counts up. After that it eases from one reading to the next. `name` tells
 *  the rings apart, and `delay` holds this one's swing back a little, so a few rings go one after another. `from` is
 *  where the band starts (see Dial), and `label` is how visible the number is, 0 to 1. */
export function useDialReading(target: number | undefined, name: string, delay = 0) {
  const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const [t, setT] = useState(() => (introPlayed.has(name) || reduce ? 1 : 0));
  const mounted = useRef(performance.now());
  const ready = target !== undefined;
  useEffect(() => {
    if (t >= 1 || !ready) return;
    introPlayed.add(name);
    const begin = Math.max(performance.now(), mounted.current + INTRO_HOLD + delay);
    let frame = requestAnimationFrame(function step(now) {
      const p = Math.min(1, Math.max(0, (now - begin) / INTRO_MS));
      setT(p);
      if (p < 1) frame = requestAnimationFrame(step);
    });
    return () => cancelAnimationFrame(frame);
    // Started once, when the first reading arrives
  }, [ready]);
  const counted = useCountUp(target ?? 0);
  if (t >= 1) return { from: 0, pct: counted, value: counted, label: 1 };
  const eased = t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
  const goal = target ?? 0;
  return {
    from: ICON_BAND.from + (360 - ICON_BAND.from) * eased,
    pct: (ICON_BAND.sweep + (goal * 3.6 - ICON_BAND.sweep) * eased) / 3.6,
    value: goal * eased,
    label: Math.min(1, t * 2.5),
  };
}

/** The big ring at the top of the Headroom pane, drawn to match the app icon: a soft, matte band in a glassy track,
 *  lit from the top left, with a faint glow and shadow under it. The band starts at `from` degrees clockwise from 12
 *  o'clock, which is the top except while it opens (see useDialReading).
 *
 *  The light comes from one direction, so the shading does too: each effect is a copy of the band (or track) shifted
 *  toward the bottom right and masked to the original's shape, so only the edge it uncovers shows. On the band that
 *  leaves it lighter along its top left and shaded along its bottom right, wherever it is on the ring. The track gets
 *  a shadow along its top left and a bright rim along its bottom right, so it looks sunk into the card. */
export function Dial({
  pct,
  size = 88,
  from = 0,
  tone,
}: {
  pct: number;
  size?: number;
  from?: number;
  /** The 5-hour limit's teal, as in the menu bar, or the violet of a model's own limit. The weekly limit is the brand's
   *  own color. */
  tone?: "five" | "model";
}) {
  const id = useId();
  const w = size * 0.085;
  const r = size / 2 - w * 1.4;
  const c = 2 * Math.PI * r;
  const filled = Math.min(100, Math.max(0, pct)) / 100;
  const mid = size / 2;
  const turn = `rotate(${from - 90} ${mid} ${mid})`;
  const ring = { cx: mid, cy: mid, r, fill: "none", strokeWidth: w };
  const arc = { ...ring, strokeLinecap: "round" as const, strokeDasharray: `${c * filled} ${c}` };
  const url = (name: string) => `url(#${id}-${name})`;
  // Toward the bottom right, away from the light, by `k` band widths
  const away = (k: number) => `translate(${w * k * 0.6} ${w * k})`;
  return (
    <svg
      className={["dial", tone, pct >= 95 && "critical"].filter(Boolean).join(" ")}
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      aria-hidden
    >
      <defs>
        <mask id={`${id}-track`}>
          <circle {...ring} stroke="white" />
        </mask>
        <mask id={`${id}-band`}>
          <circle {...arc} stroke="white" transform={turn} />
        </mask>
        <filter id={`${id}-soft`} x="-20%" y="-20%" width="140%" height="140%">
          <feGaussianBlur stdDeviation={w * 0.2} />
        </filter>
        <filter id={`${id}-blur`} x="-30%" y="-30%" width="160%" height="160%">
          <feGaussianBlur stdDeviation={w * 0.5} />
        </filter>
      </defs>

      <g mask={url("track")}>
        <circle {...ring} className="glass-shadow" />
        <circle {...ring} className="glass-rim" transform={away(0.22)} filter={url("soft")} />
        <circle {...ring} className="glass" strokeWidth={w * 0.62} filter={url("soft")} />
      </g>

      {filled > 0 && (
        <>
          <circle {...arc} className="band-glow" transform={turn} filter={url("blur")} />
          <circle {...arc} className="band-drop" transform={`${away(0.45)} ${turn}`} filter={url("blur")} />
          <g mask={url("band")}>
            <circle {...arc} className="band-light" transform={turn} />
            <circle {...arc} className="band-body" transform={`${away(0.3)} ${turn}`} filter={url("soft")} />
            <circle {...arc} className="band-shade" transform={`${away(0.85)} ${turn}`} filter={url("soft")} />
          </g>
        </>
      )}
    </svg>
  );
}

/** A thin horizontal meter, for usage by project and the limits. */
export function Meter({ pct, marker }: { pct: number; marker?: number }) {
  return (
    <div className={pct >= 95 ? "meter critical" : "meter"}>
      <div style={{ width: `${Math.min(100, pct)}%` }} />
      {marker !== undefined && <span className="meter-marker" style={{ left: `${Math.min(100, marker)}%` }} />}
    </div>
  );
}

/** A search field as the sidebar has it: the magnifying glass, and a button to clear it once something is typed. */
export function SearchField({
  value,
  onChange,
  placeholder,
  inputRef,
  onKeyDown,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder: string;
  inputRef?: React.RefObject<HTMLInputElement | null>;
  onKeyDown?: (e: React.KeyboardEvent) => void;
}) {
  const own = useRef<HTMLInputElement>(null);
  const field = inputRef ?? own;
  return (
    <label className="search-field">
      <Search />
      <input
        ref={field}
        value={value}
        placeholder={placeholder}
        aria-label={placeholder}
        spellCheck={false}
        autoCorrect="off"
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={onKeyDown}
      />
      {value && (
        <button
          className="search-clear"
          aria-label="Clear"
          title="Clear"
          onClick={() => {
            onChange("");
            field.current?.focus();
          }}
        >
          <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden>
            <path d="M4.5 4.5l7 7M11.5 4.5l-7 7" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
          </svg>
        </button>
      )}
    </label>
  );
}
