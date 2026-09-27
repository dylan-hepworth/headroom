// A chat's icon: an emoji or a picture the user picked for it, so a chat that wants their attention is easy to tell
// apart from the others at a glance. Shown in the list, on its cards, and in its notifications.

import { useState, type ReactNode } from "react";
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

/** Picking a chat's icon: an emoji, a picture from the Mac, or back to the one it started with. It can go on every
 *  chat in the same project too. */
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
  return (
    <div className="icon-picker" onClick={(e) => e.stopPropagation()}>
      <div className="icon-picker-grid">
        {EMOJI.map((emoji) => (
          <button
            key={emoji}
            className={"emoji" in icon && icon.emoji === emoji ? "icon-picker-emoji picked" : "icon-picker-emoji"}
            onClick={() => onPick({ emoji }, wholeProject)}
          >
            {emoji}
          </button>
        ))}
      </div>
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
