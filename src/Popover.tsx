// The panel that drops down from Headroom's menu bar icon when a Claude Code session needs an answer: a permission
// request to allow or deny, or a question to pick an answer for. It's its own small window, drawn to look like a
// macOS popover, and follows the system's light and dark appearance.
//
// Several requests stack like notifications: the one on top is the one to answer, and the rest peek out beneath it.

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { Avatar, choosePicture, previewIcon, type ChatIcon } from "./Avatar";
import { MicButton, useVoice, Waveform, type Listening } from "./Voice";
import { bridge, type Made } from "./bridge";
import { MadeStrip } from "./Made";
import { Markdown, MarkdownSnippet } from "./markdown";
import { PendingList, type Pending } from "./Pending";
import { TalkPanel } from "./Talk";
import { Ring } from "./ui";
import "./popover.css";

export type Ask =
  | {
      kind: "permission";
      id: string;
      title: string;
      project: string;
      tool: string;
      detail: string;
      canSession: boolean;
      /** When the request goes back to the terminal, in milliseconds since 1970, and how long it was held for. */
      until: number;
      hold: number;
      /** Does the session run in the Claude app? Then it goes back to its chat, not a terminal. */
      inChat?: boolean;
      /** The chat's icon, to tell it apart at a glance. */
      icon?: ChatIcon;
    }
  | {
      kind: "question";
      id: string;
      title: string;
      project: string;
      /** Claude can ask a few at once. The card steps through them and sends the answers together. */
      questions: Question[];
      until: number;
      hold: number;
      inChat?: boolean;
      icon?: ChatIcon;
      /** Hands-free: what Claude said this turn, what the user last said to it, and what's gone on lately, to read
       *  before answering. */
      said?: string;
      prompt?: string;
      recent?: Recent[];
      /** What Claude's made this turn, to look at before answering */
      made?: Made[];
    }
  | {
      /** Hands-free: a finished turn that didn't need a question, held open a while for a reply */
      kind: "reply";
      id: string;
      session?: string;
      title: string;
      project: string;
      until: number;
      hold: number;
      inChat?: boolean;
      icon?: ChatIcon;
      said?: string;
      prompt?: string;
      recent?: Recent[];
      made?: Made[];
    };

/** A card that's been answered, as it was, so it can show its receipt (or dissolve) even after the request is gone
 *  from the list. `failed` is set if Headroom couldn't pass the answer on. */
type Answered = { ask: Ask; choice: string; failed?: boolean };

/** Something that happened lately in a session: the user said something or answered a question, Claude wrote
 *  something, or it worked ("Ran 3 commands, edited 2 files"). */
type Recent = { kind: "said" | "answered" | "wrote" | "did"; text: string; about?: string };

type Question = {
  question: string;
  options: { label: string; description?: string }[];
  /** Can more than one option be picked? */
  multiple?: boolean;
};

/** What's picked so far for a question: the options by index, the user's own answer if "Other…" is on, and any
 *  images added to it, as data URLs. */
type Picks = { picked: number[]; other: string | null; images?: string[] };
const NO_PICKS: Picks = { picked: [], other: null };

/** Where a question card is: the question showing, and what's picked for each so far. */
type Progress = { step: number; picks: Picks[] };
const START: Progress = { step: 0, picks: [] };

/** The answers a question's picks add up to, in the options' order, with the user's own words last. */
function answersFor(question: Question, picks: Picks) {
  const { picked, other } = picks;
  return [
    ...[...picked].sort((a, b) => a - b).map((i) => question.options[i].label),
    ...(otherOn(question, picks) && other?.trim() ? [other.trim()] : []),
  ];
}

/** Is the user's own answer part of it? A single answer is either an option or your own words, never both. */
function otherOn(question: Question, { picked, other }: Picks) {
  return other !== null && (question.multiple || picked.length === 0);
}

/** The images that go with the answer to a question. */
function imagesFor(question: Question, picks: Picks) {
  return otherOn(question, picks) ? (picks.images ?? []) : [];
}

/** Has a question got an answer: an option, the user's own words, or an image? */
function hasAnswer(question: Question, picks: Picks) {
  return answersFor(question, picks).length > 0 || imagesFor(question, picks).length > 0;
}

/** What ↩ does on a question card, once the question showing has an answer: on to the next question, or from the
 *  last, send. Nothing until then. */
function advance(ask: Extract<Ask, { kind: "question" }>, progress: Progress): "send" | { progress: Progress } | null {
  const answered = ask.questions.map((q, i) => hasAnswer(q, progress.picks[i] ?? NO_PICKS));
  if (!answered[progress.step]) return null;
  if (progress.step < ask.questions.length - 1) return { progress: { ...progress, step: progress.step + 1 } };
  return answered.every(Boolean) ? "send" : null;
}

/** The answers to send, one for each question. A held question can only send words back, so each image is saved
 *  where Claude can read it (`save`), and the answer says where. */
async function answersToSend(ask: Extract<Ask, { kind: "question" }>, progress: Progress, save: (image: string) => Promise<string>) {
  return Promise.all(
    ask.questions.map(async (q, i) => {
      const picks = progress.picks[i] ?? NO_PICKS;
      const paths = await Promise.all(imagesFor(q, picks).map(save));
      return [...answersFor(q, picks), ...paths.map((path) => `see the image at ${path}`)].join(", ");
    }),
  );
}

/** What a tool call does, in a few words, and the glyph for its badge. */
function describe(tool: string): { verb: string; glyph: ReactNode } {
  const glyph = (d: ReactNode) => (
    <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      {d}
    </svg>
  );
  switch (tool) {
    case "Bash":
      return { verb: "run a command", glyph: glyph(<path d="M3 4.5L6.5 8 3 11.5M8.5 11.5H13" />) };
    case "Edit":
    case "MultiEdit":
    case "Write":
    case "NotebookEdit":
      return { verb: "edit a file", glyph: glyph(<path d="M10.5 2.8l2.7 2.7-7.6 7.6H2.9v-2.7z" />) };
    case "Read":
      return { verb: "read a file", glyph: glyph(<path d="M4 2.5h5.5L12.5 5.5v8H4zM9.5 2.5v3h3" />) };
    case "WebFetch":
    case "WebSearch":
      return {
        verb: tool === "WebFetch" ? "open a web page" : "search the web",
        glyph: glyph(
          <>
            <circle cx="8" cy="8" r="5.6" />
            <path d="M2.4 8h11.2M8 2.4c1.8 1.6 2.6 3.4 2.6 5.6S9.8 12 8 13.6C6.2 12 5.4 10.2 5.4 8S6.2 4 8 2.4z" />
          </>,
        ),
      };
    default:
      return { verb: `use ${tool}`, glyph: glyph(<path d="M8 2.5v11M2.5 8h11" />) };
  }
}

const REPLY_GLYPH = (
  <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
    <path d="M6.5 4 3 7.5 6.5 11M3.5 7.5h6a3.5 3.5 0 0 1 3.5 3.5v1" />
  </svg>
);

const QUESTION_GLYPH = (
  <svg width="15" height="15" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
    <path d="M5.8 6a2.3 2.3 0 1 1 3.4 2c-.8.5-1.2 1-1.2 1.8" />
    <circle cx="8" cy="12.3" r="0.4" fill="currentColor" />
  </svg>
);

/** Sent on the page's window when the panel comes back into view. */
const SHOWN_EVENT = "popover-shown";

/** The time now, ticking every quarter second so countdowns move smoothly. While the panel is put away, the page's
 *  timers slow down or stop, so it catches up the moment the panel is back rather than on the next tick. The
 *  countdowns themselves never stop: they count to a fixed time. */
function useNow() {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const update = () => setNow(Date.now());
    const timer = setInterval(update, 250);
    const events = ["visibilitychange", "focus", SHOWN_EVENT];
    events.forEach((e) => window.addEventListener(e, update));
    return () => {
      clearInterval(timer);
      events.forEach((e) => window.removeEventListener(e, update));
    };
  }, []);
  return now;
}

/** While the card is paused, how much time it had left when it was paused; its countdowns show that until it goes on. */
const PausedLeft = createContext<number | null>(null);

/** How long the card has left before it goes to the terminal, in milliseconds, held still while it's paused. */
function useLeft(until: number) {
  const now = useNow();
  const paused = useContext(PausedLeft);
  return paused ?? Math.max(0, until - now);
}

/** The countdown's colors, from plenty of time left to none: the app's peach, then yellow, orange, and red. */
const HEAT: [number, [number, number, number]][] = [
  [1, [240, 170, 138]],
  [0.6, [240, 170, 138]],
  [0.42, [242, 190, 40]],
  [0.25, [238, 128, 44]],
  [0.12, [229, 72, 77]],
  [0, [229, 72, 77]],
];

/** The countdown's color with `left` (0 to 1) of the time remaining, blended between the stops above. */
function heat(left: number) {
  for (let i = 1; i < HEAT.length; i++) {
    const [at, color] = HEAT[i];
    const [above, from] = HEAT[i - 1];
    if (left >= at) {
      const t = (left - at) / (above - at || 1);
      const [r, g, b] = color.map((c, k) => Math.round(c + (from[k] - c) * t));
      return `rgb(${r}, ${g}, ${b})`;
    }
  }
  return "rgb(229, 72, 77)";
}

/** The part of the countdown where pointing at the card pauses it. */
const RED_ZONE = 0.2;

/** "1:42" */
function clock(ms: number) {
  const seconds = Math.ceil(Math.max(0, ms) / 1000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

/** The tool's badge. It pulses gently once the request is about to go to the terminal. */
function Badge({ glyph, urgent }: { glyph: ReactNode; urgent: boolean }) {
  return <div className={urgent ? "ask-badge urgent" : "ask-badge"}>{glyph}</div>;
}

/** A thin bar along the bottom of the card that runs down as the hold does, warming from peach through yellow and
 *  orange to red as the request gets close to going to the terminal. */
function Countdown({ until, hold }: { until: number; hold: number }) {
  const paused = useContext(PausedLeft) !== null;
  const left = Math.min(1, useLeft(until) / hold);
  return (
    <div className={paused ? "ask-countdown paused" : "ask-countdown"} aria-hidden>
      <div style={{ width: `${left * 100}%`, background: heat(left) }} />
    </div>
  );
}

/** Hands the request back and brings up its conversation, to answer it there: its chat in the Claude app, or the app
 *  it runs in. */
function OpenButton({ ask, onOpen }: { ask: Ask; onOpen: () => void }) {
  return (
    <button className="ask-btn ghost ask-open" onClick={onOpen} title="Answer it in the conversation instead">
      {ask.inChat ? "Open in Claude" : "Open in Terminal"}
      <svg width="9" height="9" viewBox="0 0 10 10" aria-hidden>
        <path d="M3 1.5h5.5V7M8.5 1.5 1.5 8.5" />
      </svg>
    </button>
  );
}

/** Did the user send the request back, to answer it in the session itself? */
const handedBack = (choice: string) => choice === "terminal" || choice === "chat";

function Header({ ask, glyph }: { ask: Ask; glyph: ReactNode }) {
  const paused = useContext(PausedLeft) !== null;
  const left = useLeft(ask.until);
  return (
    <div className="ask-head">
      {/* The chat's icon, with the kind of request on its corner, or without one, just that */}
      {ask.icon ? (
        <Avatar icon={ask.icon} size={34} corner={<Badge glyph={glyph} urgent={!paused && left < RED_ZONE * ask.hold} />} />
      ) : (
        <Badge glyph={glyph} urgent={!paused && left < RED_ZONE * ask.hold} />
      )}
      <div className="ask-head-text">
        <div className="ask-title" title={ask.title}>
          {ask.title}
        </div>
        <div className="ask-sub">
          <span className="ask-project">
            <svg width="11" height="11" viewBox="0 0 16 16" fill="currentColor" aria-hidden>
              <path d="M1.5 4.2c0-.8.6-1.4 1.4-1.4h3.3l1.6 1.6h5.3c.8 0 1.4.6 1.4 1.4v6.3c0 .8-.6 1.4-1.4 1.4H2.9c-.8 0-1.4-.6-1.4-1.4z" />
            </svg>
            <span className="ask-project-name">{ask.project}</span>
          </span>
          <span className={left < RED_ZONE * ask.hold ? "ask-left urgent" : "ask-left"}>
            {paused
              ? `paused at ${clock(left)}`
              : ask.kind === "reply"
                ? `waits ${clock(left)}`
                : `${ask.inChat ? "chat" : "terminal"} in ${clock(left)}`}
          </span>
        </div>
      </div>
    </div>
  );
}

/** A command, shown like a line in a terminal: the prompt, the program in bold, the rest after it. Long ones fade out
 *  after two lines and open up while pointed at. */
function Command({ text, file }: { text: string; file: boolean }) {
  const body = useRef<HTMLDivElement>(null);
  const [tall, setTall] = useState(false);
  useLayoutEffect(() => {
    const el = body.current;
    if (el) setTall(el.scrollHeight > el.clientHeight + 2);
  }, [text]);
  const [program, ...rest] = text.split(" ");
  return (
    <div className={tall ? "ask-command tall" : "ask-command"} tabIndex={tall ? 0 : undefined}>
      <span className="ask-prompt">{file ? "↳" : "$"}</span>
      <div className="ask-command-body" ref={body}>
        {file ? (
          <code>{text}</code>
        ) : (
          <code>
            <b>{program}</b> {rest.join(" ")}
          </code>
        )}
      </div>
    </div>
  );
}

function PermissionCard({ ask, answer }: { ask: Extract<Ask, { kind: "permission" }>; answer: (choice: string) => void }) {
  const { verb, glyph } = describe(ask.tool);
  return (
    <>
      <Header ask={ask} glyph={glyph} />
      <div className="ask-question">
        Wants to <b>{verb}</b>
      </div>
      <Command text={ask.detail} file={ask.tool !== "Bash"} />
      <div className="ask-buttons">
        <button className="ask-btn ghost" onClick={() => answer("deny")}>
          Deny <kbd>esc</kbd>
        </button>
        <span className="ask-spacer" />
        {ask.canSession && (
          <button className="ask-btn" onClick={() => answer("session")} title="Don't ask again for this until the session ends">
            Allow for Session <kbd>⌘↩</kbd>
          </button>
        )}
        <button className="ask-btn primary" onClick={() => answer("allow")}>
          Allow <kbd>↩</kbd>
        </button>
      </div>
      {/* Its own line: beside Allow for Session and the key hints, there's no room */}
      <div className="ask-buttons below">
        <OpenButton ask={ask} onOpen={() => answer("chat")} />
      </div>
    </>
  );
}

function QuestionCard({
  ask,
  progress,
  setProgress,
  addImages,
  problem,
  next,
  answer,
}: {
  ask: Extract<Ask, { kind: "question" }>;
  progress: Progress;
  setProgress: (progress: Progress) => void;
  /** Add images to a question's own answer, whatever else has changed on the card while they were read. */
  addImages: (step: number, images: string[]) => void;
  /** Why the answer couldn't be sent, if it couldn't. */
  problem?: string;
  /** On to the next question, or send. */
  next: () => void;
  answer: (choice: string) => void;
}) {
  const { step } = progress;
  const question = ask.questions[step];
  const picks = progress.picks[step] ?? NO_PICKS;
  const setPicks = (p: Picks) => setProgress({ ...progress, picks: Object.assign([...progress.picks], { [step]: p }) });
  const { picked, other } = picks;
  const toggle = (i: number) => {
    if (!question.multiple) return setPicks({ picked: [i], other: null });
    setPicks({ ...picks, picked: picked.includes(i) ? picked.filter((p) => p !== i) : [...picked, i] });
  };
  const pickOther = () => {
    if (question.multiple) setPicks({ ...picks, other: other === null ? "" : null });
    else setPicks({ picked: [], other: other ?? "" });
  };
  const otherShown = otherOn(question, picks);
  const answered = hasAnswer(question, picks);
  const last = step === ask.questions.length - 1;
  const role = question.multiple ? "checkbox" : "radio";

  return (
    <>
      <Header ask={ask} glyph={QUESTION_GLYPH} />
      {(!!ask.said || !!ask.prompt || (ask.recent?.length ?? 0) > 0 || (ask.made?.length ?? 0) > 0) && (
        <Conversation said={ask.said} prompt={ask.prompt} recent={ask.recent} made={ask.made} />
      )}
      {ask.questions.length > 1 && (
        <div className="ask-step">
          Question {step + 1} of {ask.questions.length}
        </div>
      )}
      <div className="ask-question strong">{question.question}</div>
      {question.multiple && <div className="ask-hint">Pick as many as apply</div>}
      <div className={`ask-options ${role}`} role={question.multiple ? "group" : "radiogroup"} key={step}>
        {question.options.map((option, i) => (
          <button
            key={option.label}
            className={picked.includes(i) ? "ask-option picked" : "ask-option"}
            role={role}
            aria-checked={picked.includes(i)}
            onClick={() => toggle(i)}
          >
            <span className="ask-mark" />
            <span className="ask-option-text">
              <span className="ask-option-label">{option.label}</span>
              {option.description && <span className="ask-option-description">{option.description}</span>}
            </span>
          </button>
        ))}
        <button className={otherShown ? "ask-option picked" : "ask-option"} role={role} aria-checked={otherShown} onClick={pickOther}>
          <span className="ask-mark" />
          <span className="ask-option-text">
            <span className="ask-option-label">Other…</span>
          </span>
        </button>
        {otherShown && (
          <OtherAnswer
            value={other ?? ""}
            onChange={(other) => setPicks({ ...picks, other })}
            images={picks.images ?? []}
            onImages={(images) => setPicks({ ...picks, images })}
            onAddImages={(images) => addImages(step, images)}
            onSend={next}
          />
        )}
      </div>
      {problem && <div className="ask-problem">{problem}</div>}
      <div className="ask-buttons">
        <OpenButton ask={ask} onOpen={() => answer("chat")} />
        <span className="ask-spacer" />
        {step > 0 && (
          <button className="ask-btn ghost" onClick={() => setProgress({ ...progress, step: step - 1 })}>
            Back
          </button>
        )}
        <button className="ask-btn primary" disabled={!answered} onClick={next}>
          {last ? "Send" : "Next"} <kbd>↩</kbd>
        </button>
      </div>
    </>
  );
}

/** Hands-free: a finished turn that didn't need a question, held open a while for a reply. What Claude said, all of it,
 *  and a box to answer in; what's written goes straight to the chat. */
function ReplyCard({
  ask,
  answer,
  attach,
}: {
  ask: Extract<Ask, { kind: "reply" }>;
  answer: (choice: string) => void;
  attach: (image: string) => Promise<string>;
}) {
  const [text, setText] = useState("");
  const [images, setImages] = useState<string[]>([]);
  const [problem, setProblem] = useState("");
  const [sending, setSending] = useState(false);
  const empty = !text.trim() && images.length === 0;
  const send = async () => {
    if (sending || empty) return;
    setSending(true);
    setProblem("");
    try {
      // All a session can take from here is words, so the images go where Claude can read them
      const paths = await Promise.all(images.map(attach));
      answer(`reply:${[text.trim(), ...paths.map((path) => `see the image at ${path}`)].filter(Boolean).join(", ")}`);
    } catch (e) {
      setProblem(`Couldn't send it: ${e instanceof Error ? e.message : String(e)}`);
      setSending(false);
    }
  };
  return (
    <>
      <Header ask={ask} glyph={REPLY_GLYPH} />
      <Conversation said={ask.said} prompt={ask.prompt} recent={ask.recent} made={ask.made} />
      <OtherAnswer
        placeholder={`Reply to ${ask.title}`}
        value={text}
        onChange={setText}
        images={images}
        onImages={setImages}
        onAddImages={(more) => setImages((now) => [...now, ...more])}
        onSend={send}
      />
      {problem && <div className="ask-problem">{problem}</div>}
      <div className="ask-buttons">
        <OpenButton ask={ask} onOpen={() => answer("chat")} />
        <span className="ask-spacer" />
        <button
          className="ask-btn ghost"
          title="Nothing's sent, and it comes off the list"
          onClick={() => {
            if (ask.session) bridge.markSeen([ask.session]);
            answer("terminal");
          }}
        >
          That's All for Now
        </button>
        <button className="ask-btn primary" disabled={sending || empty} onClick={send}>
          Send <kbd>↩</kbd>
        </button>
      </div>
    </>
  );
}

/** Hands-free: what Claude said this turn, to read before answering, and a button to see what's gone on lately
 *  instead, with what the user last said. Long ones scroll. */
function Conversation({ said, prompt, recent = [], made }: { said?: string; prompt?: string; recent?: Recent[]; made?: Made[] }) {
  // The user's last message goes first when it's from before what's listed
  const lately: Recent[] = recent.some((r) => r.kind === "said") || !prompt ? recent : [{ kind: "said", text: prompt }, ...recent];
  const [showRecent, setShowRecent] = useState(!said);
  // The latest is at the bottom, so that's where the list opens
  const body = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (showRecent && body.current) body.current.scrollTop = body.current.scrollHeight;
  }, [showRecent]);
  return (
    <div className="ask-said">
      <div className="ask-said-top">
        <span>{showRecent ? "Recently" : "Claude"}</span>
        {said && lately.length > 0 && (
          <button className="ask-said-toggle" onClick={() => setShowRecent(!showRecent)}>
            {showRecent ? "Claude's Reply" : "Recent"}
          </button>
        )}
      </div>
      <div className="ask-said-body" ref={body}>
        {showRecent ? (
          <ol className="ask-recent">
            {lately.map((r, i) => (
              <li key={i} className={`ask-recent-${r.kind}`}>
                {r.kind !== "did" && (
                  <span className="ask-recent-who">{r.kind === "wrote" ? "Claude" : r.kind === "answered" ? "You answered" : "You"}</span>
                )}
                {r.about && <span className="ask-recent-about">{r.about}</span>}
                <span className="ask-recent-text">{r.kind === "did" ? r.text : <MarkdownSnippet text={r.text} />}</span>
              </li>
            ))}
          </ol>
        ) : (
          <Markdown text={said ?? ""} />
        )}
      </div>
      <MadeStrip made={made} thumb />
    </div>
  );
}

/*****************
 * C O M P A C T
 ****************/

/** A compact card's header, on one line: the badge, the chat, its project, and the time left, with buttons to show the
 *  whole card, and when it's the only one, to put the panel away (with more, that's up in the pager). */
function CompactHeader({ ask, glyph, onExpand, onClose }: { ask: Ask; glyph: ReactNode; onExpand: () => void; onClose?: () => void }) {
  const paused = useContext(PausedLeft) !== null;
  const left = useLeft(ask.until);
  const urgent = !paused && left < RED_ZONE * ask.hold;
  return (
    <div className="compact-top">
      {ask.icon ? (
        <Avatar icon={ask.icon} size={20} />
      ) : (
        <span className={ask.kind === "question" ? "compact-glyph" : "compact-glyph permission"}>{glyph}</span>
      )}
      <span className="compact-title" title={ask.title}>
        {ask.title}
      </span>
      <span className="compact-project">{ask.project}</span>
      <span className={urgent ? "compact-left urgent" : "compact-left"}>{paused ? `paused at ${clock(left)}` : clock(left)}</span>
      <button className="compact-icon" title="Show the whole card" aria-label="Show the whole card" onClick={onExpand}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path d="M6 1.5h2.5V4M4 8.5H1.5V6M8.5 1.5 5.5 4.5M1.5 8.5l3-3" />
        </svg>
      </button>
      {onClose && (
        <button className="compact-icon" title="Close. The requests keep waiting." aria-label="Close" onClick={onClose}>
          <svg width="9" height="9" viewBox="0 0 10 10" aria-hidden>
            <path d="M2 2l6 6M8 2 2 8" />
          </svg>
        </button>
      )}
    </div>
  );
}

/** What Claude said, on one line, and all of it while pointed at. It waits a moment first, so passing over it on the
 *  way to an option doesn't make the card jump. */
function CompactSaid({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const point = (inside: boolean) => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setOpen(inside), inside ? 250 : 150);
  };
  return (
    <div className={open ? "compact-said open" : "compact-said"} onPointerEnter={() => point(true)} onPointerLeave={() => point(false)}>
      <span className="compact-who">Claude</span>
      <MarkdownSnippet text={text} />
    </div>
  );
}

function CompactPermission({
  ask,
  answer,
  onExpand,
  onClose,
}: {
  ask: Extract<Ask, { kind: "permission" }>;
  answer: (choice: string) => void;
  onExpand: () => void;
  onClose?: () => void;
}) {
  const { glyph } = describe(ask.tool);
  const file = ask.tool !== "Bash";
  const [program, ...rest] = ask.detail.split(" ");
  return (
    <>
      <CompactHeader ask={ask} glyph={glyph} onExpand={onExpand} onClose={onClose} />
      {/* One line, and all of it while pointed at */}
      <div className="compact-command">
        <span className="ask-prompt">{file ? "↳" : "$"}</span>{" "}
        {file ? (
          ask.detail
        ) : (
          <>
            <b>{program}</b> {rest.join(" ")}
          </>
        )}
      </div>
      <div className="compact-foot">
        <OpenButton ask={ask} onOpen={() => answer("chat")} />
        <span className="ask-spacer" />
        <button className="ask-btn" onClick={() => answer("deny")}>
          Deny
        </button>
        {ask.canSession && (
          <button className="ask-btn" onClick={() => answer("session")} title="Don't ask again for this until the session ends">
            For Session
          </button>
        )}
        <button className="ask-btn primary" onClick={() => answer("allow")}>
          Allow
        </button>
      </div>
    </>
  );
}

/** A question as a short list: each option on a line of its own, with its number key, its detail cut off at the edge
 *  until it's pointed at. Picking one answers it, unless more than one can be picked, or it's the user's own answer. */
function CompactQuestion({
  ask,
  progress,
  setProgress,
  addImages,
  problem,
  next,
  answer,
  onExpand,
  onClose,
}: {
  ask: Extract<Ask, { kind: "question" }>;
  progress: Progress;
  setProgress: (progress: Progress) => void;
  addImages: (step: number, images: string[]) => void;
  problem?: string;
  /** On to the next question, or send, from where the card is now or from `progress`, when that's just changed. */
  next: (progress?: Progress) => void;
  answer: (choice: string) => void;
  onExpand: () => void;
  onClose?: () => void;
}) {
  const { step } = progress;
  const question = ask.questions[step];
  const picks = progress.picks[step] ?? NO_PICKS;
  const withPicks = (p: Picks) => ({ ...progress, picks: Object.assign([...progress.picks], { [step]: p }) });
  const { picked, other } = picks;
  const pick = (i: number) => {
    if (question.multiple) {
      return setProgress(withPicks({ ...picks, picked: picked.includes(i) ? picked.filter((p) => p !== i) : [...picked, i] }));
    }
    const now = withPicks({ picked: [i], other: null });
    setProgress(now);
    next(now);
  };
  const pickOther = () => {
    if (question.multiple) setProgress(withPicks({ ...picks, other: other === null ? "" : null }));
    else setProgress(withPicks({ picked: [], other: other ?? "" }));
  };
  const otherShown = otherOn(question, picks);
  // One click answers, so there's only a button to press when it takes more than that
  const needsButton = question.multiple || otherShown;
  const last = step === ask.questions.length - 1;
  return (
    <>
      <CompactHeader ask={ask} glyph={QUESTION_GLYPH} onExpand={onExpand} onClose={onClose} />
      {ask.said && <CompactSaid text={ask.said} />}
      <div className="compact-question">
        {ask.questions.length > 1 && (
          <span className="compact-step">
            {step + 1} of {ask.questions.length}
          </span>
        )}
        {question.question}
      </div>
      <div className="compact-options" role={question.multiple ? "group" : "radiogroup"} key={step}>
        {question.options.map((option, i) => (
          <button
            key={option.label}
            className={picked.includes(i) ? "compact-option picked" : "compact-option"}
            role={question.multiple ? "checkbox" : "radio"}
            aria-checked={picked.includes(i)}
            onClick={() => pick(i)}
          >
            <kbd>{i + 1}</kbd>
            <span className="compact-option-text">
              <span className="compact-option-label">{option.label}</span>{" "}
              {option.description && <span className="compact-option-detail">{option.description}</span>}
            </span>
          </button>
        ))}
        <button
          className={otherShown ? "compact-option other picked" : "compact-option other"}
          role={question.multiple ? "checkbox" : "radio"}
          aria-checked={otherShown}
          onClick={pickOther}
        >
          <kbd>{question.options.length + 1}</kbd>
          <span className="compact-option-text">
            <span className="compact-option-label">Other…</span>
          </span>
        </button>
      </div>
      {otherShown && (
        <OtherAnswer
          value={other ?? ""}
          onChange={(other) => setProgress(withPicks({ ...picks, other }))}
          images={picks.images ?? []}
          onImages={(images) => setProgress(withPicks({ ...picks, images }))}
          onAddImages={(images) => addImages(step, images)}
          onSend={() => next()}
        />
      )}
      {problem && <div className="ask-problem">{problem}</div>}
      <div className="compact-foot">
        <OpenButton ask={ask} onOpen={() => answer("chat")} />
        <span className="ask-spacer" />
        {step > 0 && (
          <button className="ask-btn ghost" onClick={() => setProgress({ ...progress, step: step - 1 })}>
            Back
          </button>
        )}
        {needsButton && (
          <button className="ask-btn primary" disabled={!hasAnswer(question, picks)} onClick={() => next()}>
            {last ? "Send" : "Next"}
          </button>
        )}
      </div>
    </>
  );
}

/** The kinds of image Claude Code can open. Any other kind is turned into a PNG. */
const READABLE = ["image/png", "image/jpeg", "image/gif", "image/webp"];
const MAX_IMAGE = 20 * 1024 * 1024;

/** An image file as a data URL, in a kind Claude Code can open. */
async function readImage(file: File): Promise<string> {
  const url = READABLE.includes(file.type)
    ? await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result));
        reader.onerror = () => reject(new Error("Couldn't read that image"));
        reader.readAsDataURL(file);
      })
    : await toPng(file);
  // Base64 takes four characters for every three bytes
  if ((url.length - url.indexOf(",")) * 0.75 > MAX_IMAGE) throw new Error("That image is over 20 MB");
  return url;
}

/** Draw an image the page can show, but Claude Code can't open (HEIC, TIFF, and so on), into a PNG. */
async function toPng(file: File): Promise<string> {
  try {
    const bitmap = await createImageBitmap(file);
    const canvas = document.createElement("canvas");
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    canvas.getContext("2d")?.drawImage(bitmap, 0, 0);
    return canvas.toDataURL("image/png");
  } catch {
    throw new Error(`Headroom can't attach that kind of image (${file.type.replace("image/", "")})`);
  }
}

/** The answer under "Other…": one line to start, and another each time the text wraps. ↩ sends it rather than
 *  starting a new line. Images can go with it: pasted, dropped on it, or picked with Add Image. */
export function OtherAnswer({
  value,
  onChange,
  images,
  onImages,
  onAddImages,
  onSend,
  placeholder = "Your answer",
  talk = true,
  listening: shown,
}: {
  placeholder?: string;
  value: string;
  onChange: (value: string) => void;
  images: string[];
  onImages: (images: string[]) => void;
  onAddImages: (images: string[]) => void;
  onSend: () => void;
  /** Can it be said instead of typed (see Voice.tsx)? */
  talk?: boolean;
  /** For the previews: what the mic's hearing, as if it were on. */
  listening?: Listening | null;
}) {
  // What's said goes after what's typed
  const typed = useRef(value);
  typed.current = value;
  const voice = useVoice((said) => onChange(typed.current.trim() ? `${typed.current.trimEnd()} ${said}` : said));
  const listening = shown ?? voice.listening;
  const field = useRef<HTMLTextAreaElement>(null);
  const [dropping, setDropping] = useState(false);
  const [problem, setProblem] = useState("");
  const add = (files: FileList | File[] | null | undefined) => {
    const list = [...(files ?? [])].filter((f) => f.type.startsWith("image/"));
    if (list.length === 0) return false;
    setProblem("");
    Promise.allSettled(list.map(readImage)).then((results) => {
      const read = results.flatMap((r) => (r.status === "fulfilled" ? [r.value] : []));
      const failed = results.find((r): r is PromiseRejectedResult => r.status === "rejected");
      if (read.length) onAddImages(read);
      if (failed) setProblem(String(failed.reason instanceof Error ? failed.reason.message : failed.reason));
    });
    return true;
  };
  useLayoutEffect(() => {
    const el = field.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, [value]);
  return (
    <div
      className={dropping ? "ask-other-box dropping" : "ask-other-box"}
      onDragOver={(e) => {
        if (![...e.dataTransfer.items].some((i) => i.type.startsWith("image/"))) return;
        e.preventDefault();
        setDropping(true);
      }}
      onDragLeave={() => setDropping(false)}
      onDrop={(e) => {
        setDropping(false);
        if (add(e.dataTransfer.files)) e.preventDefault();
      }}
    >
      <textarea
        ref={field}
        className="ask-other"
        rows={1}
        autoFocus
        placeholder={placeholder}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        // An image on the clipboard goes with the answer; anything else pastes as usual
        onPaste={(e) => add(e.clipboardData.files) && e.preventDefault()}
        onKeyDown={(e) => {
          // Not the ↩ that finishes a word in an input method (Japanese, Chinese), only one that sends
          if (e.key !== "Enter" || e.nativeEvent.isComposing || e.keyCode === 229) return;
          e.preventDefault();
          onSend();
        }}
      />
      {listening?.partial && <div className="ask-heard">{listening.partial}</div>}
      <div className="ask-images">
        {images.map((src, i) => (
          <span className="ask-image" key={i}>
            <img src={src} alt="" />
            <button aria-label="Remove image" onClick={() => onImages(images.filter((_, j) => j !== i))}>
              ×
            </button>
          </span>
        ))}
        {problem && <span className="ask-image-problem">{problem}</span>}
        <label className="ask-add-image" title="Or paste one, or drop it here">
          <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden>
            <rect x="1.75" y="2.75" width="12.5" height="10.5" rx="2" />
            <circle cx="5.6" cy="6.4" r="1.2" />
            <path d="M2.2 12l3.6-3.5 2.6 2.4 2.2-2 3.2 3" />
          </svg>
          Add Image
          <input
            type="file"
            accept="image/*"
            multiple
            hidden
            onChange={(e) => {
              add(e.target.files);
              e.target.value = "";
            }}
          />
        </label>
        {voice.problem && <span className="ask-image-problem">{voice.problem}</span>}
        {talk && (
          <span className={listening ? "ask-talk on" : "ask-talk"}>
            {listening && (
              <>
                <Waveform levels={listening.levels} />
                <span className="ask-talk-hint">Listening…</span>
              </>
            )}
            <MicButton listening={!!listening} onStart={voice.start} onStop={voice.stop} />
          </span>
        )}
      </div>
    </div>
  );
}

/** How an answer reads on the card for the moment before it goes: "Allowed", "Denied", and so on. */
function receipt(choice: string, failed: boolean | undefined, place: string): { text: string; good: boolean } {
  if (failed) return { text: `Too late: it's waiting in the ${place}`, good: false };
  if (choice === "allow") return { text: "Allowed", good: true };
  if (choice === "session") return { text: "Allowed for this session", good: true };
  if (choice === "deny") return { text: "Denied", good: false };
  if (choice === "terminal") return { text: "Sent to the terminal", good: true };
  if (choice.startsWith("reply:")) return { text: "Sent", good: true };
  return { text: "Answer sent", good: true };
}

function Receipt({ choice, failed, place }: { choice: string; failed?: boolean; place: string }) {
  const { text, good } = receipt(choice, failed, place);
  return (
    <div className={good ? "ask-receipt good" : "ask-receipt bad"}>
      <svg width="34" height="34" viewBox="0 0 34 34" aria-hidden>
        <circle cx="17" cy="17" r="15" />
        <path d={good ? "M11 17.5l4.3 4.3 8-8.8" : "M12 12l10 10M22 12l-10 10"} />
      </svg>
      <span>{text}</span>
    </div>
  );
}

/** How tall the arrow on top is, half the width of its base (curves included), and the panel's corner radius. */
const ARROW_HEIGHT = 11;
const ARROW_HALF_WIDTH = 20;
const RADIUS = 14;

/** The outline of the whole panel as one path: a rounded rectangle with the arrow rising out of its top edge. The
 *  arrow's sides curve out into the edge, the way macOS draws a popover, so the edge line never breaks. `arrow` is the
 *  arrow's point, from the left. */
export function outline(width: number, height: number, arrow: number) {
  const a = ARROW_HEIGHT;
  const r = RADIUS;
  const x = Math.min(Math.max(arrow, r + ARROW_HALF_WIDTH), width - r - ARROW_HALF_WIDTH);
  const w = ARROW_HALF_WIDTH;
  return [
    `M ${r} ${a}`,
    `L ${x - w} ${a}`,
    `C ${x - w * 0.55} ${a} ${x - w * 0.3} ${a * 0.32} ${x - w * 0.15} ${a * 0.14}`,
    `C ${x - w * 0.07} ${0.04 * a} ${x + w * 0.07} ${0.04 * a} ${x + w * 0.15} ${a * 0.14}`,
    `C ${x + w * 0.3} ${a * 0.32} ${x + w * 0.55} ${a} ${x + w} ${a}`,
    `L ${width - r} ${a}`,
    `A ${r} ${r} 0 0 1 ${width} ${a + r}`,
    `L ${width} ${height - r}`,
    `A ${r} ${r} 0 0 1 ${width - r} ${height}`,
    `L ${r} ${height}`,
    `A ${r} ${r} 0 0 1 0 ${height - r}`,
    `L 0 ${a + r}`,
    `A ${r} ${r} 0 0 1 ${r} ${a}`,
    "Z",
  ].join(" ");
}

/** The panel's size as it's drawn, so the outline can follow it. */
export function useSize() {
  // A callback ref rather than an object one: the panel isn't there until there's a request to show, and this has to
  // start measuring whenever it appears
  const [el, ref] = useState<HTMLDivElement | null>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useLayoutEffect(() => {
    if (!el) return;
    // Its layout size, not its size on screen: the opening animation scales it, and the outline has to fit it as it'll be
    const measure = () => setSize({ width: el.offsetWidth, height: el.offsetHeight });
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, [el]);
  return { ref, ...size };
}

/** How long after the panel opens, or a new card comes to the top, before a click or key can answer it. A request can
 *  arrive just as the user clicks or presses ↩ for something else, and that shouldn't answer a request they haven't
 *  seen. */
const SETTLE_MS = 450;

/** How long an answered card shows its receipt, or dissolves, before the next one comes up. */
const RECEIPT_MS = 850;
const DISSOLVE_MS = 760;

export function Popover({
  asks,
  onAnswer,
  onClose,
  onExtend,
  onAttach = async () => {
    throw new Error("Images can't be attached here");
  },
  focus = null,
  onFocused,
  openedAt = 0,
  open = true,
  pointer = null,
  arrow = 60,
  style,
  compact = false,
}: {
  asks: Ask[];
  /** Pass the answer on. It fails if the request has gone back to the terminal in the meantime. */
  onAnswer: (id: string, choice: string) => Promise<unknown>;
  /** Put the panel away. Nothing is answered: the requests keep waiting, and the menu bar item brings it back. */
  onClose: () => void;
  /** Give a request more time, while it's paused. */
  onExtend: (id: string, ms: number) => void;
  /** Save an image added to an answer, and say where it went. */
  onAttach?: (image: string) => Promise<string>;
  /** A request to bring to the top, picked from the list of what's waiting, and to call once it's there. */
  focus?: string | null;
  onFocused?: () => void;
  /** When the panel was last opened, so a click that lands as it appears doesn't count. */
  openedAt?: number;
  /** Is the panel showing? Put away, it can't be pointed at, so nothing stays paused. */
  open?: boolean;
  /** Where the pointer is over the page, when the page can't tell by itself (see PopoverWindow). */
  pointer?: [number, number] | null;
  /** Where the arrow's point is, in pixels from the panel's right edge. */
  arrow?: number;
  /** Where the panel sits on the page. Only the browser preview needs this: in the app, the window is moved instead. */
  style?: CSSProperties;
  /** Show each request in a few lines (see CompactQuestion), unless the whole card is asked for. */
  compact?: boolean;
}) {
  const { ref, width, height } = useSize();
  const path = width ? outline(width, height, width - arrow) : "";

  // Answered cards are left out until the list catches up, so one can't come back for a moment after its receipt
  const [gone, setGone] = useState<ReadonlySet<string>>(new Set());
  useEffect(() => {
    if ([...gone].some((id) => !asks.some((a) => a.id === id))) setGone(new Set([...gone].filter((id) => asks.some((a) => a.id === id))));
  }, [asks, gone]);
  const list = asks.filter((a) => !gone.has(a.id));

  // The card on top is followed by its ID, not its place in the list: requests ahead of it can go (answered in the
  // terminal, or run out) while it's being read, and it has to stay the one on top. If it goes itself, the one that
  // took its place comes up.
  const [topId, setTopId] = useState<string | null>(null);
  // Brought up once: after that, the stack goes round as usual
  useEffect(() => {
    if (!focus || !asks.some((a) => a.id === focus)) return;
    setTopId(focus);
    onFocused?.();
  }, [focus, asks]);
  const lastPosition = useRef(0);
  const found = list.findIndex((a) => a.id === topId);
  const position = found >= 0 ? found : Math.max(0, Math.min(lastPosition.current, list.length - 1));
  const top: Ask | undefined = list[position];
  useLayoutEffect(() => {
    lastPosition.current = position;
    if (top && top.id !== topId) setTopId(top.id);
  });

  // The card being answered shows its receipt for a moment, then goes, and the next one comes up. A request that runs
  // out of time, or that's handed to the terminal, dissolves instead. It's kept as it was, apart from the list.
  const [answered, setAnswered] = useState<Answered | null>(null);

  // A card on top that leaves the list without an answer from here (answered in the terminal, out of time a moment
  // before this page noticed, or its session ended) dissolves on its way out, rather than vanishing
  const lastTop = useRef<Ask | undefined>(undefined);
  const vanished = !answered && lastTop.current && !list.some((a) => a.id === lastTop.current?.id) ? lastTop.current : undefined;
  if (vanished) setAnswered({ ask: vanished, choice: "terminal" });
  useLayoutEffect(() => {
    if (!answered) lastTop.current = top;
  });
  const shown = answered?.ask ?? top;

  // How the card on top came to be there: rising from the stack, or swiping back in. It stays with that card for as
  // long as it's on top, so its animation plays once and never restarts.
  const nextArrival = useRef<"rise" | "slide-in" | null>(null);
  const [arrival, setArrival] = useState<{ id: string; how: string } | null>(null);
  const settledAt = useRef(0);
  useLayoutEffect(() => {
    if (shown && nextArrival.current) setArrival({ id: shown.id, how: nextArrival.current });
    nextArrival.current = null;
    settledAt.current = Date.now() + SETTLE_MS;
  }, [shown?.id]);
  const settled = () => Date.now() >= Math.max(settledAt.current, openedAt + SETTLE_MS);

  // Where each question card is, and what's picked on it, kept here so it survives flipping away and back
  const [progress, setProgress] = useState<Record<string, Progress>>({});
  // Why a card's answer couldn't be sent, by its ID, and the cards being sent now: saving images takes a moment, and
  // a second ↩ in the meantime mustn't send it twice
  const [problems, setProblems] = useState<Record<string, string>>({});
  const sending = useRef(new Set<string>());
  const nextQuestion = (ask: Extract<Ask, { kind: "question" }>, current = progress[ask.id] ?? START) => {
    const next = advance(ask, current);
    if (next === "send") {
      if (!top || top.id !== ask.id || answered || !settled() || sending.current.has(ask.id)) return;
      sending.current.add(ask.id);
      setProblems(({ [ask.id]: _, ...rest }) => rest);
      answersToSend(ask, current, onAttach)
        .then(
          (answers) => finish(ask, `answers:${JSON.stringify(answers)}`),
          // Not sent: the card stays, to try again or hand back
          (e) => setProblems((all) => ({ ...all, [ask.id]: `Couldn't send it: ${e instanceof Error ? e.message : String(e)}` })),
        )
        .finally(() => sending.current.delete(ask.id));
    } else if (next) setProgress((all) => ({ ...all, [ask.id]: next.progress }));
  };
  const addImages = (id: string, step: number, images: string[]) =>
    setProgress((all) => {
      const current = all[id] ?? START;
      const picks = [...current.picks];
      const was = picks[step] ?? NO_PICKS;
      picks[step] = { ...was, images: [...(was.images ?? []), ...images] };
      return { ...all, [id]: { ...current, picks } };
    });

  const finish = useCallback(
    (ask: Ask, choice: string) => {
      setAnswered({ ask, choice });
      // Sent now, not after the receipt: a request close to running out would miss its chance otherwise
      onAnswer(ask.id, choice).catch(() => {
        if (!handedBack(choice)) setAnswered((a) => (a && a.ask.id === ask.id ? { ...a, failed: true } : a));
      });
    },
    [onAnswer],
  );

  // Once its receipt or dissolve has played, the answered card goes and the next one comes up
  const answeredId = answered?.ask.id;
  const answeredChoice = answered?.choice;
  useEffect(() => {
    if (!answeredId) return;
    const timer = setTimeout(
      () => {
        setGone((g) => new Set(g).add(answeredId));
        // Its picks, images and all, aren't needed any more
        setProgress(({ [answeredId]: _, ...rest }) => rest);
        setAnswered(null);
        lastTop.current = undefined;
        nextArrival.current = "rise";
      },
      answeredChoice && handedBack(answeredChoice) ? DISSOLVE_MS : RECEIPT_MS,
    );
    return () => clearTimeout(timer);
  }, [answeredId, answeredChoice]);

  // Compact cards the user asked to see whole
  const [whole, setWhole] = useState<ReadonlySet<string>>(new Set());
  const isCompact = (ask: Ask) => compact && !whole.has(ask.id);

  // An answer from the user: a button, or a key
  const answer = useCallback(
    (choice: string) => {
      if (!top || answered || !settled()) return;
      finish(top, choice);
    },
    [top, answered, finish, openedAt],
  );

  // Pointing at a card that's in the red pauses its countdown, so it can't run out while it's being read. Pointing at
  // it earlier doesn't, or a card could sit paused forever. While it's paused, its hook is given more time every
  // second, so it holds on however long the pause lasts (up to the most the hook allows), even if the pointer never
  // moves off.
  const now = useNow();
  const [hovering, setHovering] = useState(false);
  const topCard = useRef<HTMLDivElement>(null);
  const box = pointer && topCard.current?.getBoundingClientRect();
  const pointedAt = !!box && pointer[0] >= box.left && pointer[0] < box.right && pointer[1] >= box.top && pointer[1] < box.bottom;
  const pointing = hovering || pointedAt;
  const [pause, setPause] = useState<{ id: string; left: number } | null>(null);
  const inRed = !!top && top.until - now <= RED_ZONE * top.hold;
  useEffect(() => {
    if (pointing && inRed && top && !pause && !answered && top.until > Date.now()) setPause({ id: top.id, left: top.until - Date.now() });
  }, [pointing, inRed, top, pause, answered]);
  // Moving off the card lets it go on
  useEffect(() => {
    if (!pointing) setPause(null);
  }, [pointing]);
  // A pause belongs to its card: it ends when another card comes up, or this one is answered
  const paused = pause && top && pause.id === top.id && !answered ? pause : null;
  useEffect(() => {
    if (pause && !paused) setPause(null);
  }, [pause, paused]);
  // Put away, the panel can't be pointed at, and it never hears the pointer leave
  useEffect(() => {
    if (open) return;
    setHovering(false);
    setPause(null);
  }, [open]);
  const close = () => {
    setHovering(false);
    setPause(null);
    onClose();
  };
  useEffect(() => {
    if (!paused) return;
    onExtend(paused.id, 1000);
    const tick = setInterval(() => onExtend(paused.id, 1000), 1000);
    return () => clearInterval(tick);
  }, [paused?.id, onExtend]);

  // When the one on top runs out of time, it goes to the terminal
  useEffect(() => {
    if (top && !answered && !paused && now >= top.until) finish(top, "terminal");
  }, [now, top, answered, paused, finish]);

  // Moving through the stack: the card on top swipes away to the left and the next one rises from behind, or the
  // previous one swipes back in over it. The card that's leaving stays on screen just long enough to animate out.
  const [leaving, setLeaving] = useState<{ ask: Ask; toward: "left" | "right" | "back"; dx: number } | null>(null);
  const [drag, setDrag] = useState<{ from: number; dx: number } | null>(null);
  const stack = useRef<HTMLDivElement>(null);
  // A little shake when there's nothing further to flip to
  const bump = () =>
    stack.current
      ?.querySelector(".ask-card:not(.out-left, .out-right, .out-back)")
      ?.animate([{ transform: "none" }, { transform: "translateX(-5px)" }, { transform: "translateX(5px)" }, { transform: "none" }], {
        duration: 300,
        easing: "ease-in-out",
      });
  const moveTo = (to: number, toward: "left" | "right" | "back", dx: number) => {
    if (!top) return;
    setLeaving({ ask: top, toward, dx });
    nextArrival.current = toward === "back" ? "slide-in" : "rise";
    setTopId(list[to].id);
    setTimeout(() => setLeaving(null), 420);
  };
  const go = (step: 1 | -1) => {
    const to = position + step;
    if (!top || answered || to < 0 || to >= list.length) return bump();
    moveTo(to, step > 0 ? "left" : "back", 0);
  };
  // Swiping, either way, sends the card on top to the back of the deck and brings up the next one, round and round.
  // The card flies off in the direction it was swiped.
  const swipeAway = (toward: "left" | "right", dx = 0) => {
    if (!top || answered || list.length < 2) return bump();
    moveTo((position + 1) % list.length, toward, dx);
  };

  // A two-finger swipe on the trackpad flips through the cards, one per swipe
  const swipe = useRef({ total: 0, locked: false });
  const onWheel = (e: React.WheelEvent) => {
    // Not while scrolling sideways through Claude's reply (a wide table, say)
    if ((e.target as HTMLElement).closest(".ask-said-body")) return;
    if (list.length < 2 || Math.abs(e.deltaX) < Math.abs(e.deltaY) || swipe.current.locked) return;
    swipe.current.total += e.deltaX;
    if (Math.abs(swipe.current.total) > 60) {
      swipeAway(swipe.current.total > 0 ? "left" : "right");
      swipe.current = { total: 0, locked: true };
      setTimeout(() => (swipe.current.locked = false), 500);
    }
  };

  // Dragging the card with the pointer: it follows along and tilts, and flies off if let go far enough out
  const onPointerDown = (e: React.PointerEvent) => {
    if (
      list.length < 2 ||
      answered ||
      (e.target as HTMLElement).closest("button, input, textarea, label, .ask-command, .ask-said-body, .ask-other-box")
    )
      return;
    e.currentTarget.setPointerCapture(e.pointerId);
    setDrag({ from: e.clientX, dx: 0 });
  };
  const onPointerMove = (e: React.PointerEvent) => drag && setDrag({ ...drag, dx: e.clientX - drag.from });
  const onPointerUp = () => {
    if (!drag) return;
    if (drag.dx < -90) swipeAway("left", drag.dx);
    else if (drag.dx > 90) swipeAway("right", drag.dx);
    setDrag(null);
  };

  // The keys: ↩ allows, ⌘↩ allows for the session, and esc denies; on a question, ↩ sends what's picked. ← and → move
  // through the stack. A focused button or text field keeps the keys it uses for itself, so ↩ on a focused Deny
  // doesn't also allow. The handler is kept in a ref so it always sees the current card.
  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e: KeyboardEvent) => {
    if (e.isComposing || e.repeat || !document.hasFocus()) return;
    const target = e.target instanceof Element ? e.target : null;
    const typing = !!target?.closest("input, textarea");
    const onButton = !!target?.closest("button");
    if (e.key === "Escape" && e.shiftKey) return onClose();
    if (typing) return;
    if (e.key === "ArrowRight") return go(1);
    if (e.key === "ArrowLeft") return go(-1);
    if (!top || onButton) return;
    // A reply's written in its box, which has the keys
    if (top.kind === "reply") return;
    if (top.kind === "question") {
      if (e.key === "Enter") nextQuestion(top);
      // A compact card's options have number keys
      else if (/^[1-9]$/.test(e.key) && !e.metaKey && !e.ctrlKey && isCompact(top)) {
        topCard.current?.querySelectorAll<HTMLButtonElement>(".compact-option")[Number(e.key) - 1]?.click();
      }
      return;
    }
    if (e.key === "Enter" && e.metaKey && top.canSession) answer("session");
    else if (e.key === "Enter" && !e.metaKey) answer("allow");
    else if (e.key === "Escape") answer("deny");
  };
  useEffect(() => {
    const listener = (e: KeyboardEvent) => keys.current(e);
    window.addEventListener("keydown", listener);
    return () => window.removeEventListener("keydown", listener);
  }, []);

  // The key hints only mean something while the panel has the keyboard. Opened by itself, it doesn't until clicked.
  const [focused, setFocused] = useState(() => document.hasFocus());
  useEffect(() => {
    const update = () => setFocused(document.hasFocus());
    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    return () => {
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", update);
    };
  }, []);

  // The stack follows the height of the card on top, so the panel grows and shrinks smoothly between cards of
  // different sizes rather than jumping
  const [cardHeight, setCardHeight] = useState(0);
  useLayoutEffect(() => {
    const el = topCard.current;
    if (!el) return;
    const measure = () => setCardHeight(el.offsetHeight);
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, [shown?.id, !!answered]);

  if (!shown) return null;
  // The deck goes round, so every other card is behind the one on top
  const count = answered && !list.some((a) => a.id === answered.ask.id) ? list.length + 1 : list.length;
  const behind = Math.min(count - 1, 2);
  const card = (ask: Ask, done: Answered | null) =>
    done && !handedBack(done.choice) ? (
      <Receipt choice={done.choice} failed={done.failed} place={ask.inChat ? "chat" : "terminal"} />
    ) : (
      <>
        {ask.kind === "reply" ? (
          <ReplyCard ask={ask} answer={answer} attach={onAttach} />
        ) : isCompact(ask) ? (
          ask.kind === "permission" ? (
            <CompactPermission
              ask={ask}
              answer={answer}
              onExpand={() => setWhole((w) => new Set(w).add(ask.id))}
              onClose={list.length <= 1 ? close : undefined}
            />
          ) : (
            <CompactQuestion
              ask={ask}
              progress={progress[ask.id] ?? START}
              setProgress={(p) => setProgress((all) => ({ ...all, [ask.id]: p }))}
              addImages={(step, images) => addImages(ask.id, step, images)}
              problem={problems[ask.id]}
              next={(p) => nextQuestion(ask, p)}
              answer={answer}
              onExpand={() => setWhole((w) => new Set(w).add(ask.id))}
              onClose={list.length <= 1 ? close : undefined}
            />
          )
        ) : ask.kind === "permission" ? (
          <PermissionCard ask={ask} answer={answer} />
        ) : (
          <QuestionCard
            ask={ask}
            progress={progress[ask.id] ?? START}
            setProgress={(p) => setProgress((all) => ({ ...all, [ask.id]: p }))}
            addImages={(step, images) => addImages(ask.id, step, images)}
            problem={problems[ask.id]}
            next={() => nextQuestion(ask)}
            answer={answer}
          />
        )}
        <Countdown until={ask.until} hold={ask.hold} />
      </>
    );
  const dissolving = !!answered && handedBack(answered.choice);
  // Answering the last one puts the whole panel away along with its card
  const closing = answered && count === 1 ? (dissolving ? "closing dissolved" : "closing") : "";
  // While dragging, the card that would come next shows underneath, growing into place as the drag goes further
  const peek = drag && list.length > 1 ? list[(position + 1) % list.length] : undefined;
  const arriving = arrival && arrival.id === shown.id ? arrival.how : "";

  return (
    // The panel comes and goes as a whole, shadow and all: faded by the element around it, so the window can't draw the
    // shadow on its own schedule and leave it behind
    <div className={["popover-fade", closing].filter(Boolean).join(" ")} style={{ ...style, "--arrow-right": `${arrow}px` } as CSSProperties}>
      <div className={["popover-wrap", !focused && "unfocused"].filter(Boolean).join(" ")} ref={ref}>
        <div className="popover-material" style={{ clipPath: path ? `path("${path}")` : undefined }}>
          <div className="popover-sheen" />
        </div>
        <svg className="popover-edge" width={width} height={height} aria-hidden>
          <path d={path} />
        </svg>
        {dissolving && <Dissolve />}
        <div className="popover" role="dialog" aria-label="Claude Code needs an answer">
          {list.length > 1 && (
            <div className="popover-top">
              <span className="popover-pager">
                <button aria-label="Previous" disabled={position === 0} onClick={() => go(-1)}>
                  ‹
                </button>
                <span>
                  {position + 1} of {list.length} need an answer
                </span>
                <button aria-label="Next" disabled={position >= list.length - 1} onClick={() => go(1)}>
                  ›
                </button>
              </span>
              <span className="popover-spacer" />
              <CloseButton onClose={close} />
            </div>
          )}
          <div
            ref={stack}
            className={list.length > 1 ? "ask-stack draggable" : "ask-stack"}
            style={
              cardHeight
                ? ({ gridTemplateRows: `${cardHeight}px`, "--card-h": `${cardHeight}px`, paddingBottom: behind * 4 } as CSSProperties)
                : undefined
            }
            onWheel={onWheel}
          >
            <div
              key={shown.id}
              ref={topCard}
              className={[
                "ask-card",
                isCompact(shown) && "compact",
                arriving,
                drag && "dragging",
                answered && !dissolving && "leaving",
                dissolving && "dissolving",
              ]
                .filter(Boolean)
                .join(" ")}
              style={drag ? ({ "--dx": `${drag.dx}px`, "--tilt": `${drag.dx / 30}deg` } as CSSProperties) : undefined}
              onMouseEnter={() => setHovering(true)}
              onMouseLeave={() => {
                setHovering(false);
                setPause(null);
              }}
              onPointerDown={onPointerDown}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
              onPointerCancel={onPointerUp}
            >
              <PausedLeft.Provider value={paused ? paused.left : null}>{card(shown, answered)}</PausedLeft.Provider>
              {list.length <= 1 && !answered && !isCompact(shown) && <CloseButton onClose={close} floating />}
            </div>
            {drag && peek && (
              <div
                key={`peek-${peek.id}`}
                className="ask-card peek"
                inert
                style={{ "--progress": Math.min(1, Math.abs(drag.dx) / 140) } as CSSProperties}
                aria-hidden
              >
                {card(peek, null)}
              </div>
            )}
            {leaving && (
              <div
                key={`leaving-${leaving.ask.id}`}
                className={`ask-card out-${leaving.toward}`}
                inert
                style={{ "--dx": `${leaving.dx}px`, "--tilt": `${leaving.dx / 30}deg` } as CSSProperties}
                aria-hidden
              >
                {card(leaving.ask, null)}
              </div>
            )}
            {Array.from({ length: behind }, (_, i) => (
              <div key={i} className={`ask-behind n${i + 1}`} />
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

/** Putting the panel away. Nothing is answered: the requests keep waiting, and the menu bar item brings it back. */
function CloseButton({ onClose, floating }: { onClose: () => void; floating?: boolean }) {
  return (
    <button
      className={floating ? "popover-close floating" : "popover-close"}
      aria-label="Close"
      title="Close. The requests keep waiting."
      onClick={onClose}
    >
      <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden>
        <path d="M1 1l6 6M7 1L1 7" />
      </svg>
    </button>
  );
}

/** The filter a card dissolves through on its way to the terminal: noise that pushes its pixels further and further
 *  apart as it fades. The spread is driven from here, since the filter only exists while it's in use. */
function Dissolve() {
  const map = useRef<SVGFEDisplacementMapElement>(null);
  useEffect(() => {
    const start = performance.now();
    let frame = requestAnimationFrame(function step(now) {
      const t = Math.min(1, (now - start) / 700);
      map.current?.setAttribute("scale", String(90 * t * t));
      if (t < 1) frame = requestAnimationFrame(step);
    });
    return () => cancelAnimationFrame(frame);
  }, []);
  return (
    <svg width="0" height="0" style={{ position: "absolute" }} aria-hidden>
      <filter id="dissolve" x="-20%" y="-20%" width="140%" height="140%">
        <feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="2" seed="7" result="noise" />
        <feDisplacementMap ref={map} in="SourceGraphic" in2="noise" scale="0" xChannelSelector="R" yChannelSelector="G" />
      </filter>
    </svg>
  );
}

/** The popover in its own window in the app, on the requests Headroom is holding. The window is the panel plus room
 *  around it for the shadow (see POPOVER_ROOM in main.rs); it's sized to fit the panel as it changes. */
export function PopoverWindow() {
  const [asks, setAsks] = useState<Ask[]>([]);
  // Hands-free: the list of what's waiting, while it's dropped down from the menu bar item, and a request picked from
  // it to show on its card
  const [pending, setPending] = useState<Pending[] | null>(null);
  const [focus, setFocus] = useState<string | null>(null);
  const [compact, setCompact] = useState(false);
  const [talking, setTalking] = useState(false);
  // A message being written in the list keeps it open when the window loses focus
  const writing = useRef(false);
  const [arrow, setArrow] = useState(60);
  const [openedAt, setOpenedAt] = useState(0);
  const [open, setOpen] = useState(false);
  // A panel that opened by itself belongs to an app that isn't active, and the page doesn't hear the pointer move
  // over it, so while it's open it asks where the pointer is
  const [pointer, setPointer] = useState<[number, number] | null>(null);
  useEffect(() => {
    if (!open) return setPointer(null);
    const timer = setInterval(() => bridge.popoverPointer().then(setPointer), 250);
    return () => clearInterval(timer);
  }, [open]);
  useEffect(() => {
    const load = () => {
      bridge.asks().then(setAsks);
      bridge.load().then((state) => setCompact(state.settings.compactCards));
      setPending((list) => {
        if (list) bridge.pendingSessions().then((rows) => setPending((now) => (now ? rows : now)));
        return list;
      });
    };
    load();
    bridge.popoverArrow().then(setArrow);
    const stops = [
      bridge.onChange(load),
      // Opened as the list of what's waiting, for talking to a chat, or with the requests
      bridge.onPopover("popover-open", (mode) => {
        setOpenedAt(Date.now());
        setOpen(true);
        window.dispatchEvent(new Event(SHOWN_EVENT));
        setTalking(mode === "talk");
        if (mode === "list") bridge.pendingSessions().then(setPending);
        else setPending(null);
        load();
      }),
      // The list stays as it is while the window fades, rather than the cards showing through it
      bridge.onPopover("popover-hide", () => setOpen(false)),
      bridge.onPopover("popover-arrow", (a) => setArrow(Number(a))),
    ];
    const visibility = () => document.visibilityState === "hidden" && setOpen(false);
    document.addEventListener("visibilitychange", visibility);
    return () => {
      stops.forEach((stop) => stop.then((f) => f()));
      document.removeEventListener("visibilitychange", visibility);
    };
  }, []);

  // A message waiting for a chat at work can be sent now while the chat runs a command (see Pending.tsx). Claude Code
  // says when a step ends but not when one starts, so while one's waiting, the list checks every second.
  const watching = !!pending?.some((p) => p.state === "working" && p.queued);
  useEffect(() => {
    if (!watching) return;
    const timer = setInterval(() => bridge.pendingSessions().then((rows) => setPending((now) => (now ? rows : now))), 1000);
    return () => clearInterval(timer);
  }, [watching]);

  // The list opens on a click, like a menu, and goes like one: going to another app, or ⎋. Going to another of
  // Headroom's windows, like the planner, it stays, and goes once Headroom isn't the app in front any more.
  const open_ = !!pending;
  useEffect(() => {
    if (!open_) return;
    let watch: number | undefined;
    const check = () =>
      bridge.headroomInFront().then((ours) => {
        if (writing.current || document.hasFocus()) return;
        // Quick Look, opened from the list, has the front, and the list stays down for it until it's put away
        if (!ours) bridge.looking().then((looking) => (looking ? (watch ??= window.setInterval(check, 500)) : bridge.closePopover()));
        else if (watch === undefined) watch = window.setInterval(check, 500);
      });
    // The app only knows which one's in front a moment after the focus moves
    const blur = () => window.setTimeout(check, 120);
    const focus = () => {
      window.clearInterval(watch);
      watch = undefined;
    };
    const key = (e: KeyboardEvent) => e.key === "Escape" && !e.defaultPrevented && bridge.closePopover();
    window.addEventListener("blur", blur);
    window.addEventListener("focus", focus);
    window.addEventListener("keydown", key);
    return () => {
      window.clearInterval(watch);
      window.removeEventListener("blur", blur);
      window.removeEventListener("focus", focus);
      window.removeEventListener("keydown", key);
    };
  }, [open_]);

  // Keep the window the height of the panel and the room below it for its shadow
  const box = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (!box.current) return;
    const el = box.current;
    const measure = () => bridge.popoverResize(el.offsetHeight);
    measure();
    const watch = new ResizeObserver(measure);
    watch.observe(el);
    return () => watch.disconnect();
  }, []);

  return (
    <div className="popover-window" ref={box}>
      {pending && !talking && (
        <PendingList
          items={pending}
          arrow={arrow}
          onReplying={(replying) => (writing.current = replying)}
          onIcon={async (item, icon, wholeProject) => {
            let picked: ChatIcon | null = null;
            if (icon === "picture") {
              // Choosing a file takes the window's focus, which would otherwise put the list away
              writing.current = true;
              picked = await choosePicture().finally(() => (writing.current = false));
              if (!picked) return;
            } else picked = icon;
            await bridge.setIcon(item.id, picked, wholeProject);
            const rows = await bridge.pendingSessions();
            setPending((now) => (now ? rows : now));
          }}
          onSeen={async (items) => {
            await bridge.markSeen(items.map((item) => item.id));
            const rows = await bridge.pendingSessions();
            if (rows.length) setPending((now) => (now ? rows : now));
            else bridge.closePopover();
          }}
          onStop={async (item) => {
            await bridge.stopStep(item.id);
            const rows = await bridge.pendingSessions();
            setPending((now) => (now ? rows : now));
          }}
          onOpen={(item) => bridge.openPending(item.id)}
          onAnswer={(item) => {
            setFocus(item.heldId ?? null);
            setPending(null);
            bridge.popoverCards();
          }}
          onReply={async (item, text, images) => {
            // All a session can take from here is words, so the images go where Claude can read them
            const paths = await Promise.all(images.map(bridge.saveAttachment));
            const message = [text, ...paths.map((path) => `see the image at ${path}`)].filter(Boolean).join(", ");
            await bridge.sendToSession(item.id, message);
            writing.current = false;
            const rows = await bridge.pendingSessions();
            if (rows.length) setPending((now) => (now ? rows : now));
            else bridge.closePopover();
          }}
        />
      )}
      {talking && (
        <TalkPanel
          arrow={arrow}
          onClose={() => {
            setTalking(false);
            bridge.closePopover();
          }}
        />
      )}
      {/* Kept while the list shows, so a half-answered card is just as it was */}
      <div hidden={!!pending || talking}>
        <Popover
          asks={asks}
          openedAt={openedAt}
          compact={compact}
          open={open && !pending}
          pointer={pointer}
          arrow={arrow}
          focus={focus}
          onFocused={() => setFocus(null)}
          onAnswer={bridge.answer}
          onClose={() => bridge.closePopover()}
          onExtend={bridge.extend}
          onAttach={bridge.saveAttachment}
        />
      </div>
    </div>
  );
}

/** The popover in a plain browser, under a stand-in menu bar, on made-up requests. Open with `?popover`. */
/** A picture for a chat's icon, in the previews: a little landscape. */
export const PREVIEW_PICTURE: ChatIcon = {
  image:
    "data:image/svg+xml," +
    encodeURIComponent(
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><defs><linearGradient id="s" x2="0" y2="1"><stop offset="0" stop-color="#f7b58a"/><stop offset="1" stop-color="#e98b6d"/></linearGradient></defs><rect width="40" height="40" fill="url(#s)"/><circle cx="29" cy="12" r="5" fill="#fff4d6"/><path d="M0 30l11-11 9 9 6-5 14 12v5H0z" fill="#3f6b4f"/><path d="M0 34l14-8 10 6 16-5v13H0z" fill="#2f5540"/></svg>',
    ),
};

export function PopoverPreview() {
  const now = Date.now();
  const [asks, setAsks] = useState<Ask[]>([
    // `?reply`: a finished turn that just told the user something, held open for a reply
    ...(location.search.includes("reply")
      ? [
          {
            kind: "reply" as const,
            id: "r",
            session: "c",
            title: "Blog redesign",
            project: "blog",
            said: "The archive keeps its old layout because its year headings are built by `archive.njk`, which the new theme never replaced. Three things differ from the other pages:\n\n| | Home and posts | Archive |\n| --- | --- | --- |\n| Template | `base.njk` | `archive.njk` |\n| Headings | Post titles | Years, sticky |\n| Width | 680 px | 960 px |\n\nMoving it over is about an hour: a new partial for the year headings, and the width from the theme. Nothing else depends on the old template.",
            prompt: "Why does the archive still look like the old site?",
            recent: [
              { kind: "said" as const, text: "Why does the archive still look like the old site?" },
              { kind: "did" as const, text: "Read 4 files, searched the code" },
            ],
            made: [{ path: "/Users/me/Code/blog/notes/archive.md", kind: "doc" as const, name: "archive.md" }],
            until: now + 102_000,
            hold: 120_000,
            inChat: true,
          },
        ]
      : []),
    {
      kind: "permission",
      id: "a",
      title: "Settings window polish",
      project: "headroom",
      tool: "Bash",
      detail:
        "npm run build && osascript -e 'quit app \"Headroom\"' && open src-tauri/target/release/bundle/macos/Headroom.app && sleep 2 && pgrep -fl headroom",
      canSession: true,
      until: now + 102_000,
      hold: 120_000,
    },
    {
      kind: "question",
      id: "b",
      title: "Group duplicates by capture time",
      project: "photo-sorter",
      said: "I grouped the library by capture time and found **214 pairs** that look like duplicates.\n\n- 180 pairs differ only in size, from exports\n- 29 were taken within a second of each other\n- 5 are the same photo with different edits\n\nNothing's been deleted yet. The groups are in `duplicates.json` if you want to look first.",
      prompt: "Find the duplicate photos in my library and tell me what you'd keep",
      recent: [
        { kind: "said", text: "Find the duplicate photos in my library and tell me what you'd keep" },
        { kind: "did", text: "Ran 6 commands, read 3 files" },
        { kind: "wrote", text: "The library has 12,408 photos. Some exports have the same capture time as their originals." },
        { kind: "answered", about: "Should edited copies count as duplicates?", text: "Only if the edit is just a crop" },
        { kind: "did", text: "Ran 2 commands, edited a file" },
      ],
      made: [
        { path: "/Users/me/Code/photo-sorter/pairs/IMG_2041.png", kind: "image", name: "IMG_2041.png" },
        { path: "/Users/me/Code/photo-sorter/duplicates-report.md", kind: "doc", name: "duplicates-report.md" },
        { path: "https://support.apple.com/guide/photos/pht6d60d10f/mac", kind: "link", name: "support.apple.com/…/mac" },
      ],
      questions: [
        {
          question: "Which photos should be kept when two are duplicates?",
          options: [
            { label: "The larger file", description: "Usually the original, before any export" },
            { label: "The older one", description: "By the date the photo was taken" },
            { label: "Ask each time" },
          ],
        },
      ],
      until: now + 21_000,
      hold: 120_000,
      inChat: true,
    },
    {
      kind: "question",
      id: "c",
      title: "Blog redesign",
      project: "blog",
      questions: [
        {
          question: "Which pages should get the new layout first?",
          multiple: true,
          options: [
            { label: "Home", description: "The post list and the intro" },
            { label: "Posts", description: "Every article page" },
            { label: "About" },
            { label: "Archive", description: "Posts by year" },
          ],
        },
        {
          question: "Should the old layout stay one click away?",
          options: [{ label: "Yes, for a month", description: "A link at the bottom of each new page" }, { label: "No, switch outright" }],
        },
      ],
      until: now + 95_000,
      hold: 120_000,
    },
  ]);

  // Hang the panel under the menu bar item, with its arrow pointing at the item's middle, the way the app will
  const item = useRef<HTMLSpanElement>(null);
  const [place, setPlace] = useState<{ right: number; arrow: number }>();
  useLayoutEffect(() => {
    const hang = () => {
      const box = item.current?.getBoundingClientRect();
      if (!box) return;
      const middle = box.left + box.width / 2;
      const right = Math.max(8, window.innerWidth - (middle + 60));
      setPlace({ right, arrow: window.innerWidth - right - middle });
    };
    hang();
    window.addEventListener("resize", hang);
    return () => window.removeEventListener("resize", hang);
  }, []);

  const extend = useCallback((id: string, ms: number) => setAsks((list) => list.map((a) => (a.id === id ? { ...a, until: a.until + ms } : a))), []);
  const [open, setOpen] = useState(true);
  return (
    <div className="popover-stage">
      <div className="stage-menubar">
        <span className={open ? "stage-item open" : "stage-item"} ref={item} onClick={() => setOpen((o) => !o)}>
          <Ring pct={71} size={15} stroke={2} />
          71% · 60%
          {asks.length > 0 && (
            <>
              <i className="count-dot answer" />
              {asks.length}
            </>
          )}
        </span>
        <span>Fri Sep 25 2:41 PM</span>
      </div>
      {open && location.search.includes("talk") ? (
        <TalkPanel arrow={place?.arrow} style={place && { right: place.right }} onClose={() => setOpen(false)} />
      ) : (
        open && (
          <Popover
            compact={location.search.includes("compact")}
            asks={location.search.includes("icons") ? asks.map((a, i) => ({ ...a, icon: i === 1 ? PREVIEW_PICTURE : previewIcon(a.title) })) : asks}
            style={place && { right: place.right }}
            arrow={place?.arrow}
            onAnswer={async (id) => setAsks((list) => list.filter((a) => a.id !== id))}
            onClose={() => setOpen(false)}
            onExtend={extend}
            onAttach={bridge.saveAttachment}
          />
        )
      )}
    </div>
  );
}
