// A mockup of talking instead of typing, with a made-up voice: the mic in the answer box on a question card and in the
// list, the panel the keyboard shortcut brings up to talk to a chat from anywhere, and the planner's boxes.
//
// `npm run ui`, then add `?voice` to the address.

import { useEffect, useState } from "react";
import { Avatar } from "./Avatar";
import { OtherAnswer } from "./Popover";
import { MicButton, Waveform, type Listening } from "./Voice";
import "./plan.css";
import "./popover.css";

/** A voice that's talking: louder and quieter by turns, and the words coming in a few at a time. */
function useVoice(words: string): Listening {
  const [levels, setLevels] = useState<number[]>([]);
  const [said, setSaid] = useState(0);
  useEffect(() => {
    const all = words.split(" ");
    let t = 0;
    const timer = setInterval(() => {
      t++;
      setLevels((l) => [...l.slice(-20), 0.25 + 0.6 * Math.abs(Math.sin(t / 2.3)) * Math.random()]);
      if (t % 4 === 0) setSaid((n) => (n >= all.length + 6 ? 0 : n + 1));
    }, 80);
    return () => clearInterval(timer);
  }, [words]);
  return { levels, partial: words.split(" ").slice(0, said).join(" ") };
}

function Panel({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="voice-slot">
      <div className="voice-label">{label}</div>
      <div className="popover-wrap">
        <div className="popover-material" style={{ borderRadius: 14 }} />
        <div className="popover voice-panel">{children}</div>
      </div>
    </div>
  );
}

export function VoicePreview() {
  const answer = useVoice("Keep the sixteen rules, but drop the boast about there being no exceptions");
  const message = useVoice("Skip the RAW files and just do the JPEGs");
  const anywhere = useVoice("Tell the coder to keep the session code in one file");
  const [typed, setTyped] = useState("");
  return (
    <div className="plan-stage voice-stage">
      <Panel label="On a question: the mic's by Add Image. Click it and pause when you're done, or hold it while you talk.">
        <div className="ask-question strong">Which version of the first-lesson grammar line?</div>
        <OtherAnswer value={typed} onChange={setTyped} images={[]} onImages={() => {}} onAddImages={() => {}} onSend={() => {}} listening={answer} />
      </Panel>

      <Panel label="Messaging a chat at work, from the list: the same box, the same mic.">
        <div className="voice-row">
          <Avatar icon={{ emoji: "📷" }} size={26} corner={<i className="count-dot working" />} />
          <div>
            <b>Photo library cleanup</b>
            <div className="voice-sub">photo-sorter · Working</div>
          </div>
        </div>
        <OtherAnswer
          placeholder="Tell Photo library cleanup something"
          value=""
          onChange={() => {}}
          images={[]}
          onImages={() => {}}
          onAddImages={() => {}}
          onSend={() => {}}
          listening={message}
        />
      </Panel>

      <Panel label="From anywhere: ⌃⌥Space brings this up under the menu bar, talking to the chat you last followed. ⇥ picks another.">
        <div className="voice-anywhere">
          <Avatar icon={{ emoji: "🛠️" }} size={30} />
          <div className="voice-anywhere-text">
            <div className="voice-sub">
              Talking to <b>Development</b> · My company
            </div>
            <div className="voice-words">
              {anywhere.partial}
              <span className="voice-caret" />
            </div>
          </div>
          <Waveform levels={anywhere.levels} />
        </div>
        <div className="voice-keys">
          <span>
            <kbd>↩</kbd> Send
          </span>
          <span>
            <kbd>⇥</kbd> Another chat
          </span>
          <span>
            <kbd>⎋</kbd> Cancel
          </span>
          <span className="voice-keys-note">A pause stops it too, or ⌃⌥Space again</span>
        </div>
      </Panel>

      <Panel label="The planner: the work, and an agent's standing instructions, can be said too.">
        <div className="inspector-label">The work</div>
        <div className="voice-field">
          <textarea className="inspector-field" rows={2} defaultValue="Launch the sign-in redesign: new login page, magic links" />
          <MicButton listening={false} onStart={() => {}} onStop={() => {}} />
        </div>
      </Panel>
    </div>
  );
}
