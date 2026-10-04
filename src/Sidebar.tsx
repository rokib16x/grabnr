import { rate } from "./format";
import { linkColor } from "./format";
import { Icon, linkIcon } from "./icons";
import type { Item, Link, Settings } from "./types";

export type Filter = "all" | "downloading" | "done" | "paused" | "error";

const ENTRIES: { id: Filter; label: string; icon: string; match: (i: Item) => boolean }[] = [
  { id: "all", label: "All Downloads", icon: "inbox", match: () => true },
  { id: "downloading", label: "Downloading", icon: "download", match: (i) => i.status === "downloading" || i.status === "queued" },
  { id: "done", label: "Completed", icon: "check", match: (i) => i.status === "done" },
  { id: "paused", label: "Paused", icon: "pause", match: (i) => i.status === "paused" },
  { id: "error", label: "Failed", icon: "alert", match: (i) => i.status === "error" },
];
export const matchFilter = (f: Filter, i: Item) => ENTRIES.find((e) => e.id === f)!.match(i);

export function Sidebar({ items, filter, onFilter, links, linkSpeeds, settings, onManage }: { items: Item[]; filter: Filter; onFilter: (f: Filter) => void; links: Link[]; linkSpeeds: Map<string, number>; settings?: Settings; onManage: () => void }) {
  const enabled = (n: string) => !settings || settings.enabled_links === null || settings.enabled_links.includes(n);
  return (
    <nav className="sidebar" aria-label="Sidebar">
      <div className="brand" data-tauri-drag-region>
        <img src="/grabnr.svg" alt="" width="26" height="26" />
        <span>grabnr</span>
      </div>
      <ul className="nav">
        {ENTRIES.filter((e) => e.id !== "error" || items.some(e.match)).map((e) => {
          const n = items.filter(e.match).length;
          return (
            <li key={e.id}>
              <button className={`nav-item${filter === e.id ? " on" : ""}`} onClick={() => onFilter(e.id)} aria-current={filter === e.id}>
                <Icon name={e.icon} size={16} />
                <span>{e.label}</span>
                {n > 0 && <small>{n}</small>}
              </button>
            </li>
          );
        })}
      </ul>

      <h3>Connections</h3>
      <ul className="conns">
        {links.length === 0 && <li className="muted small">No active links</li>}
        {links.map((l) => (
          <li key={l.name} className={enabled(l.name) ? "" : "off"}>
            <i className="tile" style={{ background: linkColor(l.name) }}><Icon name={linkIcon(l.kind)} size={14} /></i>
            <span className="conn-name" title={`${l.name} · ${l.ipv4}`}>{l.label}</span>
            <small>{linkSpeeds.get(l.name) ? rate(linkSpeeds.get(l.name)!) : enabled(l.name) ? "Idle" : "Off"}</small>
          </li>
        ))}
      </ul>
      <button className="manage" onClick={onManage}><Icon name="sliders" size={15} />Manage Connections</button>
    </nav>
  );
}
