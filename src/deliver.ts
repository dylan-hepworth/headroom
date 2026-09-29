// Handing a chat some words from Headroom, however it can take them.

import { bridge } from "./bridge";

/** Give a chat some words: straight in, if it's at work or its finished turn is held for a reply, or copied, with the
 *  chat opened to paste them into. Says which. */
export async function deliver(chat: string, text: string): Promise<"sent" | "copied"> {
  const now = (await bridge.pendingSessions()).find((p) => p.id === chat);
  if (now && (now.state === "working" || now.replyId)) {
    await bridge.sendToSession(chat, text);
    return "sent";
  }
  await navigator.clipboard.writeText(text);
  await bridge.openPending(chat);
  return "copied";
}
