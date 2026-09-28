// Talking to a chat from anywhere: the shortcut (⌃⌥Space, unless it's changed in Settings) brings this down from the
// menu bar item, listening, to the chat the user last followed. What's said goes to that chat the way a message or
// reply from the list does. ↩ (or the shortcut again, once it's stopped listening) sends it, ⇥ picks another chat,
// and ⎋ lets it go.

import { useEffect, useRef, useState } from "react";
import { Avatar, type ChatIcon } from "./Avatar";
import { bridge } from "./bridge";
import { outline, useSize } from "./Popover";
import { shortcutLabel, useVoice, Waveform } from "./Voice";
import "./popover.css";

/** A chat to talk to, and how what's said gets there: after the step it's on, as the reply to its finished turn, or
 *  copied, to paste in there. */
type Chat = { id: string; title: string; project: string; icon?: ChatIcon; takes: "working" | "reply" | null };

export function TalkPanel({ arrow = 60, style, onClose }: { arrow?: number; style?: React.CSSProperties; onClose: () => void }) {
  const { ref, width, height } = useSize();
  const path = width ? outline(width, height, width - arrow) : "";
  const [chats, setChats] = useState<Chat[]>([]);
  const [at, setAt] = useState(0);
  const [text, setText] = useState("");
  const [status, setStatus] = useState("");
  const [shortcut, setShortcut] = useState("");
  const voice = useVoice((said) => setText((now) => (now.trim() ? `${now.trimEnd()} ${said}` : said)));

  // The chats, the one last followed first, then the ones that can take it now
  useEffect(() => {
    Promise.all([bridge.pendingSessions(), bridge.load(), bridge.lastChat()]).then(([pending, app, last]) => {
      const now: Chat[] = pending
        .filter((p) => p.state === "working" || p.replyId)
        .map((p) => ({ id: p.id, title: p.title, project: p.project, icon: p.icon, takes: p.state === "working" ? "working" : "reply" }));
      const rest: Chat[] = app.sessions
        .filter((s) => !now.some((c) => c.id === s.id))
        .map((s) => ({ id: s.id, title: s.title ?? s.project, project: s.project, icon: s.icon, takes: null }));
      setShortcut(shortcutLabel(app.settings.talkShortcut));
      const all = [...now, ...rest];
      const first = all.findIndex((c) => c.id === last);
      setChats(first > 0 ? [all[first], ...all.filter((_, i) => i !== first)] : all);
    });
    voice.start();
  }, []);

  const chat = chats[at];
  const send = async () => {
    if (!chat || !text.trim()) return onClose();
    try {
      if (chat.takes) await bridge.sendToSession(chat.id, text.trim());
      else {
        await navigator.clipboard.writeText(text.trim());
        await bridge.openPending(chat.id);
      }
      onClose();
    } catch (e) {
      setStatus(`Couldn't send it: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  // The keys, and the shortcut pressed again: stop listening, or once it has, send
  const keys = useRef<(e: KeyboardEvent) => void>(() => {});
  keys.current = (e) => {
    if (e.key === "Escape") {
      e.preventDefault();
      bridge.listenCancel();
      onClose();
    } else if (e.key === "Tab" && chats.length > 1) {
      e.preventDefault();
      setAt((i) => (i + (e.shiftKey ? chats.length - 1 : 1)) % chats.length);
    } else if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (voice.listening) voice.stop();
      else send();
    }
  };
  const closed = useRef(onClose);
  closed.current = onClose;
  const again = useRef(() => {});
  again.current = () => (voice.listening ? voice.stop() : send());
  useEffect(() => {
    const listener = (e: KeyboardEvent) => keys.current(e);
    window.addEventListener("keydown", listener);
    const stop = bridge.onPopover("talk-key", () => again.current());
    // Gone like a menu when the user goes to another app
    const blur = () =>
      setTimeout(
        () =>
          bridge.headroomInFront().then((ours) => {
            if (ours || document.hasFocus()) return;
            bridge.listenCancel();
            closed.current();
          }),
        120,
      );
    window.addEventListener("blur", blur);
    return () => {
      window.removeEventListener("keydown", listener);
      window.removeEventListener("blur", blur);
      stop.then((f) => f());
    };
  }, []);

  const heard = voice.listening?.partial ?? "";
  return (
    <div className="popover-fade" style={{ ...style, "--arrow-right": `${arrow}px` } as React.CSSProperties}>
      <div className="popover-wrap" ref={ref}>
        <div className="popover-material" style={{ clipPath: path ? `path("${path}")` : undefined }}>
          <div className="popover-sheen" />
        </div>
        <svg className="popover-edge" width={width} height={height} aria-hidden>
          <path d={path} />
        </svg>
        <div className="popover voice-panel talk">
          <div className="voice-anywhere">
            {chat?.icon && <Avatar icon={chat.icon} size={30} />}
            <div className="voice-anywhere-text">
              <div className="voice-sub">
                {chat ? (
                  <>
                    Talking to <b>{chat.title}</b> · {chat.project}
                  </>
                ) : (
                  "No chats to talk to yet"
                )}
              </div>
              <div className="voice-words">
                {/* The words coming in take the box's place until there's something in it */}
                {(text || !heard) && (
                  <textarea
                    className="talk-text"
                    rows={1}
                    value={text}
                    placeholder={voice.listening ? "Listening…" : "Say something, or type it"}
                    onChange={(e) => setText(e.target.value)}
                  />
                )}
                {heard && <span className="talk-heard">{heard}</span>}
              </div>
            </div>
            {voice.listening && <Waveform levels={voice.listening.levels} />}
          </div>
          {(voice.problem || status) && <div className="ask-problem">{voice.problem || status}</div>}
          <div className="voice-keys">
            <span>
              <kbd>↩</kbd> {voice.listening ? "Stop" : chat && !chat.takes ? "Copy and open" : "Send"}
            </span>
            {chats.length > 1 && (
              <span>
                <kbd>⇥</kbd> Another chat
              </span>
            )}
            <span>
              <kbd>⎋</kbd> Cancel
            </span>
            <span className="voice-keys-note">
              {voice.listening ? `A pause stops it too${shortcut && `, or ${shortcut} again`}` : shortcut && `${shortcut} again sends it`}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
