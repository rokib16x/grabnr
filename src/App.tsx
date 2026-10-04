import { useEffect, useMemo, useReducer, useState } from "react";
import { api, subscribe } from "./api";
import { AddDialog } from "./AddDialog";
import { bytes, rate } from "./format";
import { DownloadRow } from "./DownloadRow";
import { SettingsPanel } from "./Settings";
import type { AppState, EngineEvent, Item, Live, Settings } from "./types";
import "./App.css";

type State = { items: Item[]; live: Record<string, Live> };
type Action =
  | { t: "init"; items: Item[] }
  | { t: "item"; item: Item }
  | { t: "removed"; id: string }
  | { t: "event"; id: string; e: EngineEvent };

const HISTORY = 60;

function reduce(s: State, a: Action): State {
  switch (a.t) {
    case "init":
      return { ...s, items: a.items };
    case "item": {
      const i = s.items.findIndex((x) => x.id === a.item.id);
      const items = i < 0 ? [a.item, ...s.items] : s.items.map((x) => (x.id === a.item.id ? a.item : x));
      return { ...s, items };
    }
    case "removed": {
      const { [a.id]: _gone, ...live } = s.live;
      return { items: s.items.filter((x) => x.id !== a.id), live };
    }
    case "event": {
      const cur = s.live[a.id];
      const e = a.e;
      let next: Live | undefined = cur;
      if (e.type === "started") {
        next = { downloaded: 0, v: 0, routes: [], speed: 0, chunks: new Uint8Array(e.chunks), resumedChunks: e.resumed_chunks, history: [], notice: e.ranges ? null : "This server does not support ranges, so it downloads as a single stream." };
      } else if (cur && e.type === "progress") {
        const history = e.routes.map((r, i) => [...(cur.history[i] ?? []), r.bytes_per_sec].slice(-HISTORY));
        next = { ...cur, downloaded: e.downloaded, speed: e.bytes_per_sec, routes: e.routes, history };
      } else if (cur && e.type === "chunk_done") {
        cur.chunks[e.idx] = e.route + 1; // in place: a copy per chunk would be wasteful on huge files
        next = { ...cur, v: cur.v + 1 };
      } else if (cur && e.type === "route_down") {
        next = { ...cur, notice: `${cur.routes[e.route]?.name ?? "A link"} dropped out (${e.reason}); the other links are taking over.` };
      } else if (cur && e.type === "resume_discarded") {
        next = { ...cur, notice: `Started over: ${e.reason}.` };
      }
      return next ? { ...s, live: { ...s.live, [a.id]: next } } : s;
    }
  }
}

export default function App() {
  const [state, dispatch] = useReducer(reduce, { items: [], live: {} });
  const [app, setApp] = useState<AppState | null>(null);
  const [adding, setAdding] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [filter, setFilter] = useState<"all" | "active" | "done">("all");

  useEffect(() => {
    api.state().then((s) => {
      setApp(s);
      dispatch({ t: "init", items: s.downloads });
    });
    return subscribe({
      item: (item) => dispatch({ t: "item", item }),
      removed: (id) => dispatch({ t: "removed", id }),
      event: (id, e) => dispatch({ t: "event", id, e }),
    });
  }, []);

  // Paste a link anywhere in the window to add it.
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      if ((e.target as HTMLElement)?.tagName === "INPUT") return;
      const t = e.clipboardData?.getData("text")?.trim() ?? "";
      if (/^https?:\/\/\S+$/i.test(t)) api.add(t).catch(() => {});
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, []);

  const shown = useMemo(
    () => state.items.filter((i) => (filter === "all" ? true : filter === "done" ? i.status === "done" : i.status !== "done")),
    [state.items, filter],
  );
  const total = state.items.filter((i) => i.status === "downloading").reduce((a, i) => a + (state.live[i.id]?.speed ?? 0), 0);
  const activeCount = state.items.filter((i) => i.status === "downloading").length;
  const linkSpeeds = new Map<string, number>();
  state.items.forEach((i) => i.status === "downloading" && state.live[i.id]?.routes.forEach((r) => linkSpeeds.set(r.name, (linkSpeeds.get(r.name) ?? 0) + r.bytes_per_sec)));

  return (
    <div className="app">
      <header>
        <h1>grabnr</h1>
        <nav>
          {(["all", "active", "done"] as const).map((f) => (
            <button key={f} className={filter === f ? "tab on" : "tab"} onClick={() => setFilter(f)}>
              {f === "all" ? "All" : f === "active" ? "Active" : "Completed"}
            </button>
          ))}
        </nav>
        <div className="spacer" />
        <button className="primary" onClick={() => setAdding(true)}>Add link</button>
        <button className="ghost" onClick={() => setSettingsOpen(true)}>Settings</button>
      </header>

      <main>
        {shown.length === 0 ? (
          <div className="empty">
            <h2>{state.items.length ? "Nothing here" : "No downloads yet"}</h2>
            <p>Paste a link anywhere in this window, press Add link, or install the browser extension so downloads from Chrome and Brave start here.</p>
          </div>
        ) : (
          <ul className="list">
            {shown.map((i) => (
              <DownloadRow key={i.id} item={i} live={state.live[i.id]} />
            ))}
          </ul>
        )}
      </main>

      <footer>
        <span>{activeCount ? `${activeCount} downloading · ${rate(total)}` : "Idle"}</span>
        {[...linkSpeeds].map(([n, v]) => (
          <span key={n} className="muted">{n} {bytes(v)}/s</span>
        ))}
      </footer>

      {adding && <AddDialog onClose={() => setAdding(false)} />}
      {settingsOpen && app && (
        <SettingsPanel
          settings={app.settings}
          apiPort={app.api_port}
          apiOk={app.api_ok}
          onChange={(settings: Settings) => setApp({ ...app, settings })}
          onClose={() => setSettingsOpen(false)}
        />
      )}
    </div>
  );
}
