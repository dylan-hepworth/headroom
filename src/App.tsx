import { useEffect, useMemo, useRef, useState } from "react";
import { bridge, needsAnswer, yourTurn, type AppState, type Settings } from "./bridge";
import { Bell, BrandMark, Calendar, Gauge, Hook, MenuBar as MenuBarIcon, Terminal } from "./icons";
import { Ring, SearchField } from "./ui";
import { pieces } from "./searchMatch";
import { reveal, searchSettings, SETTINGS, type PaneId, type Result } from "./settingsIndex";
import General from "./panes/General";
import Usage from "./panes/Usage";
import Alerts from "./panes/Alerts";
import Sessions from "./panes/Sessions";
import Hooks from "./panes/Hooks";
import Recap from "./panes/Recap";
import MenuBar from "./panes/MenuBar";

/** Changing a setting resolves to why it couldn't be changed, or null when it was. */
export type PaneProps = { app: AppState; settings: Settings; set: <K extends keyof Settings>(key: K, value: Settings[K]) => Promise<string | null> };

type Pane = {
  id: PaneId;
  title: string;
  tile: string;
  icon: React.ReactNode;
  view: (p: PaneProps) => React.ReactNode;
};

const GROUPS: { heading?: string; panes: Pane[] }[] = [
  {
    panes: [
      {
        id: "general",
        title: "Headroom",
        tile: "brand",
        icon: <BrandMark size={20} />,
        view: (p) => <General {...p} />,
      },
    ],
  },
  {
    heading: "Usage",
    panes: [
      {
        id: "usage",
        title: "Usage",
        tile: "clay",
        icon: <Gauge />,
        view: (p) => <Usage {...p} />,
      },
      {
        id: "alerts",
        title: "Alerts",
        tile: "red",
        icon: <Bell />,
        view: (p) => <Alerts {...p} />,
      },
    ],
  },
  {
    heading: "Claude Code",
    panes: [
      {
        id: "sessions",
        title: "Sessions",
        tile: "orange",
        icon: <Terminal />,
        view: (p) => <Sessions {...p} />,
      },
      {
        id: "hooks",
        title: "Hooks",
        tile: "purple",
        icon: <Hook />,
        view: (p) => <Hooks {...p} />,
      },
      {
        id: "recap",
        title: "Daily Recap",
        tile: "teal",
        icon: <Calendar />,
        view: (p) => <Recap {...p} />,
      },
    ],
  },
  {
    heading: "Look",
    panes: [
      {
        id: "menu_bar",
        title: "Menu Bar",
        tile: "blue",
        icon: <MenuBarIcon />,
        view: (p) => <MenuBar {...p} />,
      },
    ],
  },
];
const PANES = GROUPS.flatMap((g) => g.panes);

function useWindowFocus() {
  const [focused, setFocused] = useState(() => document.hasFocus());
  useEffect(() => {
    const on = () => setFocused(true);
    const off = () => setFocused(false);
    window.addEventListener("focus", on);
    window.addEventListener("blur", off);
    return () => {
      window.removeEventListener("focus", on);
      window.removeEventListener("blur", off);
    };
  }, []);
  return focused;
}

/** The search field and, while there's a query, its results in place of the list of panes. */
function useSearch(open: (r: Result) => void) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const field = useRef<HTMLInputElement>(null);
  const results = useMemo(() => searchSettings(query), [query]);
  useEffect(() => setSelected(0), [query]);
  // ⌘F, as in every Mac app with a search field
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey && !e.altKey && !e.ctrlKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        field.current?.focus();
        field.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelected((i) => Math.min(i + 1, results.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelected((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter" && results[selected]) {
      e.preventDefault();
      open(results[selected]);
    } else if (e.key === "Escape") {
      e.preventDefault();
      setQuery("");
      field.current?.blur();
    }
  };
  return { query, setQuery, results, selected, setSelected, field, onKeyDown };
}

export default function App() {
  const [current, setCurrent] = useState<PaneId>(() => (new URLSearchParams(location.search).get("pane") as PaneId) || "general");
  // A result picked in the search, to scroll to once its pane is showing. It's wrapped in a new object each time, so
  // picking the same result again scrolls to it again.
  const [found, setFound] = useState<{ result: Result } | null>(null);
  const open = (r: Result) => {
    setCurrent(r.setting.pane);
    setFound({ result: r });
  };
  const search = useSearch(open);
  const [app, setApp] = useState<AppState | null>(null);
  const focused = useWindowFocus();
  useEffect(() => {
    if (!found || !app) return;
    const frame = requestAnimationFrame(() => reveal(found.result.setting));
    return () => cancelAnimationFrame(frame);
  }, [found, current, !app]);

  useEffect(() => {
    const load = () => bridge.load().then(setApp);
    load();
    // Clicking a notification can open Settings at one of its settings, the Updates row say, and outline it
    const showSetting = (pane: string, title: string) => {
      setCurrent(pane as PaneId);
      const setting = SETTINGS.find((s) => s.pane === pane && s.title === title);
      if (setting) setFound({ result: { setting, hits: [], score: 0 } });
    };
    const opened = new URLSearchParams(location.search);
    const [pane, setting] = [opened.get("pane"), opened.get("setting")];
    if (pane && setting) showSetting(pane, setting);
    const stops = [
      bridge.onChange(load),
      bridge.onShowPane((pane) => setCurrent(pane as PaneId)),
      bridge.onShowSetting(({ pane, setting }) => showSetting(pane, setting)),
    ];
    return () => stops.forEach((s) => s.then((stop) => stop()));
  }, []);

  if (!app) return null;
  const settings = app.settings;
  // Show the change right away, then let the app confirm it (or put it back) when it reports what it actually did
  const set: PaneProps["set"] = (key, value) => {
    setApp((a) => a && { ...a, settings: { ...a.settings, [key]: value } });
    return bridge.set(key, value).then(
      () => null,
      (e) => {
        bridge.load().then(setApp);
        return String(e);
      },
    );
  };

  const pane = PANES.find((p) => p.id === current) ?? PANES[0];
  const needYou = app.sessions.filter((s) => needsAnswer(s) || yourTurn(s)).length;
  const five = app.limits.find((l) => l.key === "5h");
  const weekly = app.limits.find((l) => l.key === "7d");
  const highest = Math.max(0, ...app.limits.map((l) => l.pct));

  return (
    <div className={focused ? "window" : "window inactive"}>
      <aside className="sidebar">
        <div className="drag" data-tauri-drag-region />
        <SearchField value={search.query} onChange={search.setQuery} placeholder="Search" inputRef={search.field} onKeyDown={search.onKeyDown} />
        {search.query.trim() ? (
          <div className="results" role="listbox" aria-label="Search results">
            {search.results.length === 0 ? (
              <div className="no-results">Nothing matches “{search.query.trim()}”</div>
            ) : (
              search.results.map((r, i) => {
                const where = PANES.find((p) => p.id === r.setting.pane)?.title;
                return (
                  <button
                    key={`${r.setting.pane}-${r.setting.title}`}
                    role="option"
                    aria-selected={i === search.selected}
                    onMouseEnter={() => search.setSelected(i)}
                    onClick={() => {
                      search.setSelected(i);
                      open(r);
                    }}
                  >
                    <span className={`setting-tile small ${r.setting.tile[0]}`}>{r.setting.tile[1]}</span>
                    <span className="result-text">
                      <span className="result-title">
                        {pieces(r.setting.title, r.hits).map((p, k) => (p.hit ? <b key={k}>{p.text}</b> : p.text))}
                      </span>
                      <span className="result-sub">{r.setting.section ? `${where} › ${r.setting.section}` : where}</span>
                    </span>
                  </button>
                );
              })
            )}
          </div>
        ) : (
          <nav className="nav">
            {GROUPS.map((g, i) => (
              <div className="nav-group" key={i}>
                {g.heading && <div className="nav-heading">{g.heading}</div>}
                {g.panes.map((p) => (
                  <button key={p.id} aria-current={p.id === current ? "page" : undefined} onClick={() => setCurrent(p.id)}>
                    <span className={`tile ${p.tile}`}>{p.icon}</span>
                    {p.title}
                    {p.id === "sessions" && settings.hooks && needYou > 0 && <span className="badge">{needYou}</span>}
                  </button>
                ))}
              </div>
            ))}
          </nav>
        )}
      </aside>
      <main className="content">
        <header className="topbar drag" data-tauri-drag-region>
          <h1 data-tauri-drag-region>{pane.title}</h1>
          <div className="spacer" data-tauri-drag-region />
          <div className="usage-pill" title={app.status}>
            {app.ok ? <Ring pct={weekly?.pct ?? 0} critical={highest >= 95} size={16} stroke={2.2} /> : <span className="pill-warning">⚠</span>}
            {app.ok && (
              <span className="pill-name">
                {five ? `${five.pct}%` : "–"} · {weekly ? `${weekly.pct}%` : "–"}
              </span>
            )}
            {app.ok && <span className="pill-sep" />}
            <span className="pill-status">{app.status}</span>
          </div>
        </header>
        <div className="scroll">
          <div className="pane" key={pane.id}>
            {pane.view({ app, settings, set })}
          </div>
        </div>
      </main>
    </div>
  );
}
