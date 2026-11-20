// What the pages say and do differently on Windows: the taskbar where a Mac has the menu bar, Ctrl where a Mac has ⌘,
// File Explorer for Finder. Add `?windows` to the address in a plain browser to see the Windows version on a Mac.

export const WINDOWS =
  (typeof navigator !== "undefined" && /Windows/.test(navigator.userAgent)) ||
  (typeof location !== "undefined" && new URLSearchParams(location.search).has("windows"));

/** Where Headroom lives: the menu bar, or on Windows, the taskbar. */
export const BAR = WINDOWS ? "taskbar" : "menu bar";

/** The computer, as a person calls it. */
export const COMPUTER = WINDOWS ? "PC" : "Mac";

/** Is the key that goes with a shortcut held: ⌘ on a Mac, Ctrl on Windows? */
export function primary(e: { metaKey: boolean; ctrlKey: boolean }) {
  return WINDOWS ? e.ctrlKey && !e.metaKey : e.metaKey;
}

/** A shortcut written the way the system writes it: "⌘S" on a Mac, "Ctrl+S" on Windows. `keys` uses the Mac's marks. */
export function keys(mac: string) {
  if (!WINDOWS) return mac;
  const names: Record<string, string> = { "⌘": "Ctrl", "⌃": "Ctrl", "⌥": "Alt", "⇧": "Shift", "↩": "Enter", "⌫": "Backspace" };
  const parts: string[] = [];
  let rest = "";
  for (const c of mac) {
    if (names[c] && c !== "↩" && c !== "⌫") parts.push(names[c]);
    else rest += names[c] ?? c;
  }
  // In the order Windows writes them
  const order = ["Ctrl", "Alt", "Shift"];
  parts.sort((a, b) => order.indexOf(a) - order.indexOf(b));
  return [...parts, rest].filter(Boolean).join("+");
}
