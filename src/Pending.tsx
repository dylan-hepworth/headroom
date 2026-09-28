// Hands-free: the conversations waiting on the user or at work, dropped down from the menu bar item when it's clicked.
// Each one can be opened where it runs, a waiting request answered on its card, a finished chat replied to right
// here, and one at work sent a message for after its current step.
//
// `npm run ui`, then add `?pending` to the address, to see it on made-up conversations.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Avatar, previewIcon, IconPicker, type ChatIcon } from "./Avatar";
import { MarkdownSnippet } from "./markdown";
import { OtherAnswer, outline, PREVIEW_PICTURE, useSize } from "./Popover";
import { Ring } from "./ui";
import "./popover.css";

export type Pending = {
  id: string;
  title: string;
  project: string;
  /** Asking a question, asking permission, working on something, or done and waiting for the user's next message. */
  state: "question" | "permission" | "working" | "done";
  /** When it started waiting, or for one at work, when it got the user's message, in milliseconds since 1970. */
  since: number;
  /** The question, what it wants to do, what it's doing, or the start of Claude's last reply. */
  what: string;
  /** The request Headroom's holding for it, to answer on its card. */
  heldId?: string | null;
  /** A finished chat whose turn Headroom is holding open, so a reply from here goes straight to it. */
  replyId?: string | null;
  /** Runs in the Claude app, rather than a terminal. */
  inChat?: boolean;
  /** A message sent to it from here is waiting for it to take. */
  queued?: boolean;
  /** At work on a command, which can be stopped so a waiting message goes in now. */
  canStop?: boolean;
  /** The chat's icon, to tell it apart at a glance. */
  icon?: ChatIcon;
};

const STATE: Record<Pending["state"], string> = {
  question: "Has a question",
  permission: "Wants permission",
  working: "Working",
  done: "Done",
};

/** "just now", "4m", "2h" */
function ago(since: number) {
  const minutes = Math.floor((Date.now() - since) / 60_000);
  if (minutes < 1) return "just now";
  return minutes < 60 ? `${minutes}m` : `${Math.floor(minutes / 60)}h`;
}

function OpenIcon() {
  return (
    <svg width="9" height="9" viewBox="0 0 10 10" aria-hidden>
      <path d="M3 1.5h5.5V7M8.5 1.5 1.5 8.5" />
    </svg>
  );
}

function Row({
  item,
  onOpen,
  onAnswer,
  onReply,
  onReplying,
  onStop,
  onSeen,
  onIcon,
}: {
  item: Pending;
  onOpen: () => void;
  onAnswer: () => void;
  onReply: (text: string, images: string[]) => Promise<unknown>;
  onReplying?: (replying: boolean) => void;
  onStop: () => Promise<unknown>;
  onSeen: () => void;
  /** A new icon for it (null to go back to the one it started with), for it or every chat in its project, or a picture
   *  to choose for it. */
  onIcon?: (icon: ChatIcon | "picture" | null, wholeProject: boolean) => void;
}) {
  const [picking, setPicking] = useState(false);
  // A working session is interjected rather than replied to: the message waits for its next step
  const working = item.state === "working";
  const [replying, setReplyingState] = useState(false);
  const setReplying = (now: boolean) => {
    setReplyingState(now);
    onReplying?.(now);
  };
  // A row that goes while a message is being written in it (the chat ended, say) lets the list go too
  const writing = useRef(false);
  writing.current = replying;
  useEffect(() => () => void (writing.current && onReplying?.(false)), []);
  const [text, setText] = useState("");
  const [images, setImages] = useState<string[]>([]);
  const [problem, setProblem] = useState("");
  // Saving images takes a moment, and a second ↩ in the meantime mustn't send the reply twice
  const sending = useRef(false);
  const waiting = item.state === "question" || item.state === "permission";
  // Stopping the command it's running, so the message waiting for it goes in now
  const [stopping, setStopping] = useState(false);
  const stop = () => {
    setStopping(true);
    setProblem("");
    onStop()
      .catch((e) => setProblem(e instanceof Error ? e.message : String(e)))
      .finally(() => setStopping(false));
  };
  const send = () => {
    if (sending.current || (!text.trim() && images.length === 0)) return;
    sending.current = true;
    setProblem("");
    onReply(text.trim(), images)
      .then(() => {
        if (!working) return;
        setReplying(false);
        setText("");
        setImages([]);
      })
      .catch((e) => setProblem(`Couldn't send it: ${e instanceof Error ? e.message : String(e)}`))
      .finally(() => (sending.current = false));
  };
  return (
    <li className={replying ? "pending-row replying" : "pending-row"}>
      <div className="pending-main" onClick={() => !replying && (item.heldId ? onAnswer() : onOpen())}>
        {item.icon ? (
          <Avatar
            icon={item.icon}
            corner={<i className={waiting ? "count-dot answer" : working ? "count-dot working" : "count-dot turn"} />}
            onClick={onIcon && (() => setPicking(!picking))}
            title="Change its icon"
          />
        ) : (
          <i className={waiting ? "count-dot answer" : working ? "count-dot working" : "count-dot turn"} />
        )}
        <div className="pending-text">
          <div className="pending-top">
            <span className="pending-title">{item.title}</span>
            <span className="pending-ago">{ago(item.since)}</span>
          </div>
          <div className="pending-sub">
            {item.project} · {STATE[item.state]}
          </div>
          {/* Claude's words, in Markdown, but not a command or what it's doing */}
          <div className="pending-what">{item.state === "done" || item.state === "question" ? <MarkdownSnippet text={item.what} /> : item.what}</div>
          {item.queued && (
            <div className="pending-sent">
              {working ? "Sent. Claude sees it after its current step." : "It finished first, so your message goes with your next one."}
            </div>
          )}
          {problem && !replying && <div className="ask-problem">{problem}</div>}
        </div>
      </div>
      {picking && item.icon && onIcon && (
        <IconPicker
          icon={item.icon}
          project={item.project}
          onPick={(icon, whole) => {
            onIcon(icon, whole);
            setPicking(false);
          }}
          onPicture={(whole) => {
            onIcon("picture", whole);
            setPicking(false);
          }}
          onReset={() => {
            onIcon(null, false);
            setPicking(false);
          }}
        />
      )}
      {replying ? (
        <div
          className="pending-reply"
          onKeyDown={(e) => {
            if (e.key !== "Escape") return;
            // Only the reply goes, not the list
            e.preventDefault();
            setReplying(false);
          }}
        >
          <OtherAnswer
            placeholder={working ? `Tell ${item.title} something` : `Reply to ${item.title}`}
            value={text}
            onChange={setText}
            images={images}
            onImages={setImages}
            onAddImages={(more) => setImages((now) => [...now, ...more])}
            onSend={send}
          />
          {problem && <div className="ask-problem">{problem}</div>}
          <div className="pending-actions">
            <span className="ask-spacer" />
            <button className="ask-btn ghost" onClick={() => setReplying(false)}>
              Cancel
            </button>
            <button className="ask-btn primary" disabled={!text.trim() && images.length === 0} onClick={send}>
              Send <kbd>↩</kbd>
            </button>
          </div>
        </div>
      ) : (
        <div className="pending-actions">
          {waiting && item.heldId ? (
            <button className="ask-btn" onClick={onAnswer}>
              {item.state === "question" ? "Answer" : "Review"}
            </button>
          ) : working ? (
            <>
              {item.queued && item.canStop && (
                <button
                  className="ask-btn primary"
                  disabled={stopping}
                  title="Stops the command Claude is running, the way Esc does, so it sees your message now"
                  onClick={stop}
                >
                  Send Now
                </button>
              )}
              <button className="ask-btn" onClick={() => setReplying(true)}>
                Message
              </button>
            </>
          ) : !waiting && item.replyId ? (
            <button className="ask-btn" onClick={() => setReplying(true)}>
              Reply
            </button>
          ) : null}
          <button className="ask-btn ghost ask-open" onClick={onOpen}>
            {item.inChat ? "Open in Claude" : "Open in Terminal"}
            <OpenIcon />
          </button>
          {item.state === "done" && (
            <>
              <span className="ask-spacer" />
              <button className="ask-btn ghost" onClick={onSeen} title="Take it off the list, as if you'd opened it">
                Mark as Seen
              </button>
            </>
          )}
        </div>
      )}
    </li>
  );
}

/** The list itself, in the popover's panel. */
export function PendingList({
  items,
  arrow = 60,
  style,
  onOpen,
  onAnswer,
  onReply,
  onReplying,
  onStop,
  onSeen,
  onIcon,
}: {
  items: Pending[];
  arrow?: number;
  style?: React.CSSProperties;
  onOpen: (item: Pending) => void;
  onAnswer: (item: Pending) => void;
  onReply: (item: Pending, text: string, images: string[]) => Promise<unknown>;
  /** A reply's being written, or isn't any more. */
  onReplying?: (replying: boolean) => void;
  onStop: (item: Pending) => Promise<unknown>;
  /** Finished chats the user's seen, to take off the list. */
  onSeen: (items: Pending[]) => void;
  onIcon?: (item: Pending, icon: ChatIcon | "picture" | null, wholeProject: boolean) => void;
}) {
  const { ref, width, height } = useSize();
  const path = width ? outline(width, height, width - arrow) : "";
  const sections: [string, Pending[]][] = [
    ["Needs you", items.filter((i) => i.state === "question" || i.state === "permission")],
    ["Working", items.filter((i) => i.state === "working")],
    ["Done", items.filter((i) => i.state === "done")],
  ];
  const row = (item: Pending) => (
    <Row
      key={item.id}
      item={item}
      onOpen={() => onOpen(item)}
      onAnswer={() => onAnswer(item)}
      onReply={(text, images) => onReply(item, text, images)}
      onReplying={onReplying}
      onStop={() => onStop(item)}
      onSeen={() => onSeen([item])}
      onIcon={onIcon && ((icon, whole) => onIcon(item, icon, whole))}
    />
  );
  return (
    <div className="popover-fade" style={{ ...style, "--arrow-right": `${arrow}px` } as React.CSSProperties}>
      <div className="popover-wrap" ref={ref}>
        <div className="popover-material" style={{ clipPath: path ? `path("${path}")` : undefined }}>
          <div className="popover-sheen" />
        </div>
        <svg className="popover-edge" width={width} height={height} aria-hidden>
          <path d={path} />
        </svg>
        <div className="popover pending">
          {/* One list, so a row that moves to another heading (done, or asking something) keeps what's being
              written in it */}
          <ul className="pending-list">
            {sections.flatMap(([heading, rows]) =>
              rows.length
                ? [
                    <li key={heading} className="pending-heading">
                      {heading}
                      {heading === "Done" && rows.length > 1 && (
                        <button className="pending-heading-action" onClick={() => onSeen(rows)}>
                          Mark All as Seen
                        </button>
                      )}
                    </li>,
                    ...rows.map(row),
                  ]
                : [],
            )}
          </ul>
          {items.length === 0 && <div className="pending-empty">No chats are waiting on you or at work.</div>}
        </div>
      </div>
    </div>
  );
}

/** The mockup: a menu bar with Headroom's item, which drops the list down when it's clicked, and puts it away on
 *  another click, a click anywhere else, or ⎋. */
export function PendingPreview() {
  const now = Date.now();
  // `?pending&working` shows only the chats at work
  const [items, setItems] = useState<Pending[]>(() =>
    (
      [
        {
          id: "a",
          title: "Ship the 2.0 release",
          project: "weather-app",
          state: "question",
          since: now - 2 * 60_000,
          what: "Publish to the App Store now, or wait for the last round of TestFlight feedback?",
          heldId: "q-a",
          inChat: true,
        },
        {
          id: "b",
          title: "Settings window polish",
          project: "headroom",
          state: "permission",
          since: now - 5 * 60_000,
          what: "npm run build && open src-tauri/target/release/bundle/macos/Headroom.app",
          heldId: "q-b",
        },
        {
          id: "w1",
          title: "Photo library cleanup",
          project: "photo-sorter",
          state: "working",
          since: now - 3 * 60_000,
          what: "Running python dedupe.py --dry-run",
          inChat: true,
        },
        {
          id: "w2",
          title: "Docs site",
          project: "docs",
          state: "working",
          since: now - 12 * 60_000,
          what: "Editing src/pages/install.md",
        },
        {
          id: "c",
          title: "Blog redesign",
          project: "blog",
          state: "done",
          since: now - 8 * 60_000,
          what: "Moved the home page and every post to the new layout. The archive still uses the old one, since its year headings need their own design.",
          replyId: "r-c",
          inChat: true,
        },
        {
          id: "d",
          title: "Group duplicates by capture time",
          project: "photo-sorter",
          state: "done",
          since: now - 21 * 60_000,
          what: "Deleted 214 duplicate exports and kept the originals. 12,194 photos left, and the groups are in duplicates.json.",
          inChat: true,
        },
      ] satisfies Pending[]
    ).filter((i) => !location.search.includes("working") || i.state === "working"),
  );
  const [note, setNote] = useState("");
  // `?icons` gives each chat its icon, one of them a picture
  const icons = location.search.includes("icons");
  useEffect(() => {
    if (icons) setItems((list) => list.map((x) => ({ ...x, icon: x.project === "photo-sorter" ? PREVIEW_PICTURE : previewIcon(x.title) })));
  }, []);

  // A click on the item opens and closes it, as does ⎋ and a click anywhere else
  const [open, setOpen] = useState(false);
  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => !(e.target as Element).closest(".popover-wrap, .stage-item") && setOpen(false);
    const key = (e: KeyboardEvent) => e.key === "Escape" && !e.defaultPrevented && setOpen(false);
    window.addEventListener("pointerdown", away);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("pointerdown", away);
      window.removeEventListener("keydown", key);
    };
  }, [open]);

  const item = useRef<HTMLSpanElement>(null);
  const [place, setPlace] = useState<{ right: number; arrow: number }>();
  useLayoutEffect(() => {
    const box = item.current?.getBoundingClientRect();
    if (!box) return;
    const middle = box.left + box.width / 2;
    const right = Math.max(8, window.innerWidth - (middle + 60));
    setPlace({ right, arrow: window.innerWidth - right - middle });
  }, []);

  const waiting = items.filter((i) => i.state === "question" || i.state === "permission").length;
  const done = items.filter((i) => i.state === "done").length;
  return (
    <div className="popover-stage">
      <div className="stage-menubar">
        <span className={open ? "stage-item open" : "stage-item"} ref={item} onClick={() => setOpen((now) => !now)}>
          <Ring pct={71} size={15} stroke={2} />
          71% · 60%
          {waiting > 0 && (
            <>
              <i className="count-dot answer" />
              {waiting}
            </>
          )}
          {done > 0 && (
            <>
              <i className="count-dot turn" />
              {done}
            </>
          )}
        </span>
        <span>Fri Sep 25 2:41 PM</span>
      </div>
      {open && (
        <PendingList
          items={items}
          style={place && { right: place.right }}
          arrow={place?.arrow}
          onOpen={(i) => setNote(`Opens “${i.title}” ${i.inChat ? "in Claude" : "in its terminal"}`)}
          onAnswer={(i) => setNote(`Shows “${i.title}” on its card, to answer`)}
          onIcon={
            icons
              ? (i, icon, whole) => {
                  const next = icon === "picture" ? PREVIEW_PICTURE : (icon ?? previewIcon(i.title));
                  setNote(icon === "picture" ? "Opens a file picker for a picture" : whole ? `Uses it for every chat in ${i.project}` : "");
                  setItems((list) => list.map((x) => (x.id === i.id || (whole && x.project === i.project) ? { ...x, icon: next } : x)));
                }
              : undefined
          }
          onSeen={(seen) => {
            setNote(seen.length > 1 ? "Marks every finished chat as seen" : `Marks “${seen[0].title}” as seen`);
            setItems((list) => list.filter((x) => !seen.some((s) => s.id === x.id)));
          }}
          onStop={async (i) => {
            setNote(`Stops what “${i.title}” is running, and it reads your message now`);
            setItems((list) => list.map((x) => (x.id === i.id ? { ...x, queued: false, canStop: false, what: "Reading your message" } : x)));
          }}
          onReply={async (i, text, images) => {
            const also = images.length ? ` and ${images.length === 1 ? "an image" : `${images.length} images`}` : "";
            if (i.state === "working") {
              setItems((list) => list.map((x) => (x.id === i.id ? { ...x, queued: true, canStop: true } : x)));
              return setNote(`Queues “${text}”${also} for “${i.title}”, for after its current step`);
            }
            setNote(`Sends “${text}”${also} to “${i.title}”, and Claude carries on`);
            setItems((list) => list.filter((x) => x.id !== i.id));
          }}
        />
      )}
      {note && <div className="pending-note">{note}</div>}
      {icons && (
        // Notifications as they'd come in, for the mockup: macOS puts the app's icon first, so a chat's emoji leads
        // its title, and its picture goes beside the text
        <div className="stage-notifications">
          {items
            .filter((i) => i.state !== "working")
            .slice(0, 3)
            .map((i) => (
              <div className="stage-notification" key={i.id}>
                <span className="stage-app-icon">
                  <Ring pct={71} size={20} stroke={3} />
                </span>
                <div>
                  <b>
                    {i.icon && "emoji" in i.icon ? `${i.icon.emoji} ` : ""}
                    {i.title}
                  </b>
                  <div>{i.state === "done" ? i.what : i.state === "question" ? `Has a question: ${i.what}` : `Wants to run ${i.what}`}</div>
                </div>
                {i.icon && "image" in i.icon && <img className="stage-notification-image" src={i.icon.image} alt="" />}
              </div>
            ))}
        </div>
      )}
    </div>
  );
}
