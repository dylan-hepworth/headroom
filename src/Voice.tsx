// Talking instead of typing: a mic button for any box Headroom has you write in, and what it shows while it listens.
// Click it and talk, and it stops by itself after a pause (or click it again); or hold it down while you talk, and let
// go to stop.

import { useEffect, useRef, useState } from "react";
import { bridge } from "./bridge";
import "./popover.css";

/** What's being heard while the mic's on: how loud it's been lately (0 to 1, the newest last), and the words heard so
 *  far that may still change. */
export type Listening = { levels: number[]; partial: string };

/** A clicked mic stops by itself after this long without speech. */
export const PAUSE_MS = 1800;
/** Held down longer than this, the mic's held to talk, and letting go stops it. */
const HOLD_MS = 350;

/** The last few moments' loudness, as bars. */
export function Waveform({ levels }: { levels: number[] }) {
  return (
    <span className="waveform" aria-hidden>
      {levels.slice(-14).map((l, i) => (
        <i key={i} style={{ height: `${Math.max(12, Math.min(100, l * 100))}%` }} />
      ))}
    </span>
  );
}

/** The mic. A click turns it on, until a pause or another click; holding it down talks until it's let go. */
export function MicButton({ listening, onStart, onStop }: { listening: boolean; onStart: () => void; onStop: () => void }) {
  const press = useRef({ at: 0, wasOn: false });
  return (
    <button
      type="button"
      className={listening ? "mic on" : "mic"}
      title={listening ? "Stop" : "Talk: click, and pause when you're done, or hold it while you talk"}
      aria-label={listening ? "Stop listening" : "Talk"}
      aria-pressed={listening}
      onPointerDown={() => {
        press.current = { at: Date.now(), wasOn: listening };
        if (!listening) onStart();
      }}
      onPointerUp={() => {
        const held = Date.now() - press.current.at > HOLD_MS;
        if (held || press.current.wasOn) onStop();
      }}
    >
      <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden>
        <rect x="5.25" y="1.75" width="5.5" height="8.5" rx="2.75" />
        <path d="M3 7.5a5 5 0 0 0 10 0M8 12.5v2" />
      </svg>
    </button>
  );
}

/** What the app says while listening (see speech.rs): how loud, the words so far, what was said, or what went wrong. */
export type Heard = { kind: "level"; level: number } | { kind: "words" | "done" | "failed"; text: string };

/** Loud enough to be talking, and quiet enough to have stopped, on the app's 0 to 1 scale. */
const TALKING = 0.12;
const QUIET = 0.05;

/** Which box is listening now: there's one mic, and one box at a time has it. */
let owner: symbol | null = null;

/** Listening for a box: start and stop the mic, what's heard meanwhile, and what was said, handed to `onText` at the
 *  end. It stops by itself after a pause once something's been said. */
export function useVoice(onText: (text: string) => void) {
  const me = useRef(Symbol("mic")).current;
  const [listening, setListening] = useState<Listening | null>(null);
  const [problem, setProblem] = useState("");
  const said = useRef({ heard: false, talked: false, quietSince: 0, stopping: false });
  const done = useRef(onText);
  done.current = onText;
  useEffect(() => {
    const stop = bridge.onVoice((heard) => {
      if (owner !== me) return;
      if (heard.kind === "level") {
        const pause = said.current;
        pause.heard = true;
        if (heard.level > TALKING) {
          pause.talked = true;
          pause.quietSince = 0;
        } else if (pause.talked && heard.level < QUIET) {
          pause.quietSince ||= Date.now();
          if (Date.now() - pause.quietSince > PAUSE_MS && !pause.stopping) {
            pause.stopping = true;
            bridge.listenStop();
          }
        }
        setListening((now) => now && { ...now, levels: [...now.levels.slice(-30), heard.level] });
      } else if (heard.kind === "words") {
        setListening((now) => now && { ...now, partial: heard.text });
      } else {
        owner = null;
        setListening(null);
        if (heard.kind === "done" && heard.text.trim()) done.current(heard.text.trim());
        // Stopped before anything was said is no problem, but a mic that never started is
        if (heard.kind === "failed" && (said.current.talked || !said.current.heard)) setProblem(heard.text);
      }
    });
    return () => {
      stop.then((f) => f());
      if (owner === me) {
        owner = null;
        bridge.listenCancel();
      }
    };
  }, []);
  return {
    listening,
    problem,
    start: () => {
      owner = me;
      said.current = { heard: false, talked: false, quietSince: 0, stopping: false };
      setProblem("");
      setListening({ levels: [], partial: "" });
      bridge.listenStart();
    },
    stop: () => owner === me && bridge.listenStop(),
  };
}

/** A box to write in, or to say what goes in it: what's said goes after what's typed. */
export function SpokenField({
  value,
  onChange,
  rows = 3,
  placeholder,
  className = "",
}: {
  value: string;
  onChange: (value: string) => void;
  rows?: number;
  placeholder?: string;
  className?: string;
}) {
  const typed = useRef(value);
  typed.current = value;
  const voice = useVoice((said) => onChange(typed.current.trim() ? `${typed.current.trimEnd()} ${said}` : said));
  return (
    <div className={voice.listening ? "spoken-field on" : "spoken-field"}>
      <textarea className={className} rows={rows} value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />
      {voice.listening?.partial && <div className="ask-heard">{voice.listening.partial}</div>}
      <span className="spoken-mic">
        {voice.listening && <Waveform levels={voice.listening.levels} />}
        <MicButton listening={!!voice.listening} onStart={voice.start} onStop={voice.stop} />
      </span>
      {voice.problem && <div className="ask-problem">{voice.problem}</div>}
    </div>
  );
}

/** A shortcut as the app reads it ("Control+Alt+Space") the way macOS shows one: ⌃⌥Space. */
export function shortcutLabel(keys: string | null | undefined) {
  if (!keys) return "";
  const marks: Record<string, string> = { control: "⌃", ctrl: "⌃", alt: "⌥", option: "⌥", shift: "⇧", command: "⌘", cmd: "⌘", super: "⌘" };
  const arrows: Record<string, string> = { arrowup: "↑", arrowdown: "↓", arrowleft: "←", arrowright: "→", up: "↑", down: "↓", left: "←", right: "→" };
  return keys
    .split("+")
    .map((part) => {
      const p = part.toLowerCase();
      if (marks[p]) return marks[p];
      if (arrows[p]) return arrows[p];
      return part.replace(/^Key/, "").replace(/^Digit/, "");
    })
    .join("");
}

/** A shortcut as the app reads it, from a key pressed with at least one of ⌃, ⌥, or ⌘ (⇧ alone would type), or none
 *  for a key on its own or a modifier still being held. */
export function shortcutFrom(e: KeyboardEvent | React.KeyboardEvent) {
  if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) return null;
  if (!e.ctrlKey && !e.altKey && !e.metaKey) return null;
  const mods = [e.ctrlKey && "Control", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Command"].filter(Boolean);
  return [...mods, e.code].join("+");
}

/** Recording a shortcut: click, then press it. ⎋, or a click anywhere else, leaves it as it was. */
export function ShortcutField({ keys, onChange }: { keys: string | null; onChange: (keys: string | null) => Promise<string | null> }) {
  const [recording, setRecording] = useState(false);
  const [problem, setProblem] = useState("");
  const button = useRef<HTMLButtonElement>(null);
  const set = (next: string | null) => onChange(next).then((failed) => setProblem(failed ? failed.replace(/^Error: /, "") : ""));
  const latest = useRef(set);
  latest.current = set;
  // Listened for on the window: WebKit doesn't focus a button that's clicked, so the button itself wouldn't hear them
  useEffect(() => {
    if (!recording) return;
    const key = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") return setRecording(false);
      const next = shortcutFrom(e);
      if (!next) return;
      setRecording(false);
      latest.current(next);
    };
    const away = (e: PointerEvent) => !button.current?.contains(e.target as Node) && setRecording(false);
    window.addEventListener("keydown", key, true);
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("blur", () => setRecording(false), { once: true });
    return () => {
      window.removeEventListener("keydown", key, true);
      window.removeEventListener("pointerdown", away, true);
    };
  }, [recording]);
  return (
    <span className="shortcut-field">
      <button ref={button} className={recording ? "btn shortcut recording" : "btn shortcut"} onClick={() => setRecording(true)}>
        {recording ? "Press the keys…" : keys ? shortcutLabel(keys) : "Off"}
      </button>
      {keys && !recording && (
        <button className="btn" onClick={() => set(null)}>
          Turn Off
        </button>
      )}
      {problem && <span className="shortcut-problem">{problem}</span>}
    </span>
  );
}
