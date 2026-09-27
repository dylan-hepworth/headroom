// A chat's icon: an emoji or a picture the user picked for it, so a chat that wants their attention is easy to tell
// apart from the others at a glance. Shown in the list, on its cards, and in its notifications.

import { useEffect, useState, type ReactNode } from "react";
import { bridge } from "./bridge";
import "./popover.css";

export type ChatIcon = { emoji: string } | { image: string };

/** The emoji to pick from, in rows of eight: things, animals, food, and marks that stand out at 16 points. The same
 *  ones a chat can start with (see icons.rs). */
export const EMOJI = `
  🦊 🐙 🦉 🐝 🐢 🦋 🐳 🦕
  🚀 🛠️ 🧪 📦 🧭 🎨 📚 🔭
  🌵 🍋 🌶️ 🍄 🌊 🔥 ⚡ ❄️
  💎 🎯 🧩 🎲 🪐 🌙 ⭐ 🍀
`
  .trim()
  .split(/\s+/);

/** An emoji for a made-up chat in the previews, the same one every time for the same chat. The app picks real ones. */
export function previewIcon(id: string): ChatIcon {
  let hash = 0;
  for (const c of id) hash = (hash * 31 + c.charCodeAt(0)) | 0;
  return { emoji: EMOJI[Math.abs(hash) % EMOJI.length] };
}

/** The icon on a rounded tile, with something small on its corner: the chat's status, or the kind of request. */
export function Avatar({
  icon,
  size = 26,
  corner,
  onClick,
  title,
}: {
  icon: ChatIcon;
  size?: number;
  corner?: ReactNode;
  onClick?: () => void;
  title?: string;
}) {
  const Tag = onClick ? "button" : "span";
  return (
    <Tag
      className={onClick ? "avatar pickable" : "avatar"}
      style={{ width: size, height: size, fontSize: Math.round(size * 0.62) } as React.CSSProperties}
      onClick={
        onClick &&
        ((e: React.MouseEvent) => {
          e.stopPropagation();
          onClick();
        })
      }
      title={title}
    >
      {"emoji" in icon ? <span className="avatar-emoji">{icon.emoji}</span> : <img src={icon.image} alt="" draggable={false} />}
      {corner && <span className="avatar-corner">{corner}</span>}
    </Tag>
  );
}

/** Every emoji macOS has a name for, with its name, to search. Asked for once. */
let names: Promise<[string, string][]> | null = null;
const emojiNames = () => (names ??= bridge.emojiNames().catch(() => []));

/** The emoji whose names have every word of the search at the start of one of theirs: "red he" finds ❤️ "red heart".
 *  An emoji typed or pasted in is one of them. */
function search(all: [string, string][], query: string): [string, string][] {
  const typed = [...query.matchAll(/\p{Extended_Pictographic}\uFE0F?(\u200D\p{Extended_Pictographic}\uFE0F?)*/gu)].map((m): [string, string] => [
    m[0],
    "",
  ]);
  const words = query
    .toLowerCase()
    .replace(/\p{Extended_Pictographic}/gu, "")
    .split(/\s+/)
    .filter(Boolean);
  if (!words.length) return typed;
  const found = all.filter(([, name]) => {
    const nameWords = name.toLowerCase().split(/[\s-]+/);
    return words.every((word) => nameWords.some((n) => n.startsWith(word)));
  });
  return [...typed, ...found];
}

/** Picking a chat's icon: an emoji, from a few to hand or any of macOS's by name, a picture from the Mac, or back to the
 *  one it started with. It can go on every chat in the same project too. */
export function IconPicker({
  icon,
  project,
  onPick,
  onPicture,
  onReset,
}: {
  icon: ChatIcon;
  project: string;
  onPick: (icon: ChatIcon, wholeProject: boolean) => void;
  onPicture: (wholeProject: boolean) => void;
  onReset: () => void;
}) {
  const [wholeProject, setWholeProject] = useState(false);
  const [query, setQuery] = useState("");
  const [all, setAll] = useState<[string, string][]>([]);
  useEffect(() => {
    emojiNames().then(setAll);
  }, []);
  const shown: [string, string][] = query.trim()
    ? search(all, query).slice(0, 64)
    : EMOJI.map((emoji) => [emoji, all.find(([e]) => e === emoji)?.[1] ?? ""]);
  return (
    <div className="icon-picker" onClick={(e) => e.stopPropagation()}>
      <input
        className="icon-picker-search"
        placeholder="Search emoji"
        value={query}
        autoFocus
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          // Only the search, not the list or card around it
          e.stopPropagation();
          if (e.key === "Enter" && shown.length) onPick({ emoji: shown[0][0] }, wholeProject);
        }}
      />
      <div className="icon-picker-grid">
        {shown.map(([emoji, name]) => (
          <button
            key={emoji}
            className={"emoji" in icon && icon.emoji === emoji ? "icon-picker-emoji picked" : "icon-picker-emoji"}
            title={name}
            onClick={() => onPick({ emoji }, wholeProject)}
          >
            {emoji}
          </button>
        ))}
      </div>
      {query.trim() && shown.length === 0 && <div className="icon-picker-none">No emoji called “{query.trim()}”</div>}
      <div className="icon-picker-foot">
        <button className="ask-btn" onClick={() => onPicture(wholeProject)}>
          Choose Picture…
        </button>
        <button className="ask-btn ghost" onClick={onReset}>
          Reset
        </button>
      </div>
      <label className="icon-picker-project">
        <input type="checkbox" checked={wholeProject} onChange={(e) => setWholeProject(e.target.checked)} />
        Use for every chat in {project}
      </label>
    </div>
  );
}

/** A picture from the Mac for an icon: the user picks a file, and it's cut to a square from its middle and made small,
 *  whatever the size and kind of image it was. Null if they cancel, or it isn't a picture. */
export function choosePicture(): Promise<ChatIcon | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/*";
    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) return resolve(null);
      const url = URL.createObjectURL(file);
      const img = new Image();
      img.onload = () => {
        const side = Math.min(img.naturalWidth, img.naturalHeight);
        const canvas = document.createElement("canvas");
        canvas.width = canvas.height = 128;
        canvas.getContext("2d")?.drawImage(img, (img.naturalWidth - side) / 2, (img.naturalHeight - side) / 2, side, side, 0, 0, 128, 128);
        URL.revokeObjectURL(url);
        resolve({ image: canvas.toDataURL("image/png") });
      };
      img.onerror = () => {
        URL.revokeObjectURL(url);
        resolve(null);
      };
      img.src = url;
    };
    input.addEventListener("cancel", () => resolve(null));
    input.click();
  });
}
