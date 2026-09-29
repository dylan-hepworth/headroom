// What Claude made in a turn, on its card and its row in the list (see made.rs): a picture's thumbnail, and a chip for
// each document, picture, file, and link. A chip opens Quick Look over whatever the user's doing, or the browser for a
// link, and a Markdown or text file reads right here in the panel.

import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { bridge, type Made } from "./bridge";
import { Markdown } from "./markdown";

const READABLE = /\.(md|markdown|txt)$/i;

/** Thumbnails already made, by path, so the list doesn't ask again each time it's shown. */
const pictures = new Map<string, Promise<string | null>>();
const picture = (path: string) => {
  if (!pictures.has(path))
    pictures.set(
      path,
      bridge.madePicture(path).catch(() => null),
    );
  return pictures.get(path)!;
};

function Glyph({ kind }: { kind: Made["kind"] }) {
  if (kind === "image")
    return (
      <svg width="12" height="12" viewBox="0 0 16 16" aria-hidden>
        <rect x="1.75" y="2.75" width="12.5" height="10.5" rx="2" fill="none" stroke="currentColor" strokeWidth="1.4" />
        <path d="M3.5 11.5l3-3.2 2.2 2.2 1.6-1.5 2.2 2.5" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
      </svg>
    );
  if (kind === "link")
    return (
      <svg width="12" height="12" viewBox="0 0 16 16" aria-hidden>
        <path
          d="M6.8 9.2a3 3 0 0 0 4.2 0l2.2-2.2a3 3 0 0 0-4.2-4.2l-.9.9M9.2 6.8a3 3 0 0 0-4.2 0L2.8 9a3 3 0 0 0 4.2 4.2l.9-.9"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.4"
          strokeLinecap="round"
        />
      </svg>
    );
  return (
    <svg width="12" height="12" viewBox="0 0 16 16" aria-hidden>
      <path
        d="M4 1.75h5.2L12.5 5v8.25a1 1 0 0 1-1 1h-7.5a1 1 0 0 1-1-1V2.75a1 1 0 0 1 1-1zM9 1.75V5.3h3.4"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** A picture's thumbnail, once it's been made small; nothing if it can't be. */
function Thumb({ item, onLook }: { item: Made; onLook: () => void }) {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => void picture(item.path).then(setSrc), [item.path]);
  if (!src) return null;
  return (
    <button className="made-thumb" title={`${item.name}: Quick Look`} onClick={onLook}>
      <img src={src} alt={item.name} />
    </button>
  );
}

/** What Claude made that turn, with the first picture as a thumbnail when `thumb` is set. */
export function MadeStrip({ made, thumb = false }: { made?: Made[]; thumb?: boolean }) {
  const strip = useRef<HTMLDivElement>(null);
  const [reading, setReading] = useState<Made | null>(null);
  const [problem, setProblem] = useState("");
  if (!made?.length) return null;
  const look = (item: Made) => {
    setProblem("");
    bridge.lookAt(item.path).catch((e) => setProblem(e instanceof Error ? e.message : String(e)));
  };
  const open = (item: Made) => (READABLE.test(item.path) ? setReading(item) : look(item));
  const first = thumb ? made.find((m) => m.kind === "image" && !/\.svg$/i.test(m.path)) : undefined;
  // The panel's own box, for the document to fill while it's read
  const panel = strip.current?.closest(".popover");
  return (
    <div className="made" ref={strip} onClick={(e) => e.stopPropagation()}>
      {first && <Thumb item={first} onLook={() => look(first)} />}
      <div className="made-chips">
        {made.map((item) => (
          <button key={item.path} className={`made-chip made-${item.kind}`} title={item.path} onClick={() => open(item)}>
            <Glyph kind={item.kind} />
            <span>{item.name}</span>
          </button>
        ))}
        {problem && <div className="ask-problem">{problem}</div>}
      </div>
      {reading && panel && createPortal(<Reader item={reading} onLook={() => look(reading)} onClose={() => setReading(null)} />, panel)}
    </div>
  );
}

/** A Markdown or text file Claude made, read in the panel, over whatever it was showing. ⎋ or Back puts it away. */
function Reader({ item, onLook, onClose }: { item: Made; onLook: () => void; onClose: () => void }) {
  const [text, setText] = useState<string | null>(null);
  const [problem, setProblem] = useState("");
  useEffect(() => {
    bridge.readMade(item.path).then(setText, (e) => setProblem(e instanceof Error ? e.message : String(e)));
  }, [item.path]);
  // Room to read, and ⎋ for the document before the panel
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const panel = box.current?.closest(".popover");
    panel?.classList.add("reading");
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      onClose();
    };
    window.addEventListener("keydown", key, true);
    return () => {
      panel?.classList.remove("reading");
      window.removeEventListener("keydown", key, true);
    };
  }, []);
  const where = item.path.replace(/^\/Users\/[^/]+/, "~").replace(/\/[^/]+$/, "");
  return (
    <div className="made-read" ref={box}>
      <div className="made-read-top">
        <button className="ask-btn ghost" onClick={onClose}>
          ‹ Back
        </button>
        <span className="ask-spacer" />
        <button className="ask-btn ghost" onClick={onLook}>
          Quick Look
        </button>
        <button className="ask-btn ghost" onClick={() => bridge.showMade(item.path)}>
          Show in Finder
        </button>
      </div>
      <div className="made-read-name">
        <Glyph kind="doc" /> {item.name} <span>{where}</span>
      </div>
      <div className="made-read-body">
        {problem ? (
          <div className="ask-problem">{problem}</div>
        ) : text === null ? null : /\.txt$/i.test(item.path) ? (
          <pre className="made-text">{text}</pre>
        ) : (
          <Markdown text={text} />
        )}
      </div>
    </div>
  );
}
