import { useEffect, useReducer, useState } from "react";
import { api, subscribe } from "./api";
import { registerLinks } from "./format";
import type { AppState, EngineEvent, Item, Link, Live } from "./types";

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


/** Downloads, live progress and settings, kept in sync with the backend. Shared by the main window and the menu bar popover. */
export function useDownloads() {
  const [state, dispatch] = useReducer(reduce, { items: [], live: {} });
  const [app, setApp] = useState<AppState | null>(null);
  const [links, setLinks] = useState<Link[]>([]);

  useEffect(() => {
    api.state().then((s) => {
      setApp(s);
      dispatch({ t: "init", items: s.downloads });
    });
    api.links().then((r) => {
      registerLinks(r.links);
      setLinks(r.links);
    });
    return subscribe({
      item: (item) => dispatch({ t: "item", item }),
      removed: (id) => dispatch({ t: "removed", id }),
      event: (id, e) => dispatch({ t: "event", id, e }),
    });
  }, []);

  const active = state.items.filter((i) => i.status === "downloading");
  const speed = active.reduce((a, i) => a + (state.live[i.id]?.speed ?? 0), 0);
  const linkSpeeds = new Map<string, number>();
  active.forEach((i) => state.live[i.id]?.routes.forEach((r) => linkSpeeds.set(r.name, (linkSpeeds.get(r.name) ?? 0) + r.bytes_per_sec)));

  return { ...state, app, setApp, links, active, speed, linkSpeeds };
}
