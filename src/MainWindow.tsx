import { useEffect, useMemo, useState } from "react";
import { api, onOpenSettings } from "./api";
import { AddDialog } from "./AddDialog";
import { DownloadRow } from "./DownloadRow";
import { Icon } from "./icons";
import { Inspector } from "./Inspector";
import { MoreMenu } from "./Menu";
import { SettingsPanel } from "./Settings";
import { matchFilter, Sidebar, type Filter } from "./Sidebar";
import { useDownloads } from "./store";
import { fileName } from "./format";
import type { Settings } from "./types";

const TITLES: Record<Filter, string> = { all: "All Downloads", downloading: "Downloading", done: "Completed", paused: "Paused", error: "Failed" };

export function MainWindow() {
  const d = useDownloads();
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);

  useEffect(() => onOpenSettings(() => setSettingsOpen(true)), []);

  // Paste a link anywhere in the window to add it.
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      const t = e.clipboardData?.getData("text")?.trim() ?? "";
      if (/^https?:\/\/\S+$/i.test(t)) api.add(t).catch(() => {});
    };
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "n") { e.preventDefault(); setAdding(true); }
      if ((e.metaKey || e.ctrlKey) && e.key === ",") { e.preventDefault(); setSettingsOpen(true); }
    };
    window.addEventListener("paste", onPaste);
    window.addEventListener("keydown", onKey);
    return () => { window.removeEventListener("paste", onPaste); window.removeEventListener("keydown", onKey); };
  }, []);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return d.items.filter((i) => matchFilter(filter, i) && (!q || fileName(i.url, i.filename).toLowerCase().includes(q) || i.url.toLowerCase().includes(q)));
  }, [d.items, filter, query]);

  // Keep something selected so the inspector is never blank while there are downloads.
  const current = d.items.find((i) => i.id === selected) ?? shown.find((i) => i.status === "downloading") ?? shown[0];
  const hasDone = d.items.some((i) => i.status === "done");
  const canPause = d.items.some((i) => i.status === "downloading" || i.status === "queued");
  const canResume = d.items.some((i) => i.status === "paused");

  return (
    <div className="app">
      <Sidebar items={d.items} filter={filter} onFilter={setFilter} links={d.links} linkSpeeds={d.linkSpeeds} settings={d.app?.settings} onManage={() => setSettingsOpen(true)} />

      <section className="main">
        <header data-tauri-drag-region>
          <div className="title" data-tauri-drag-region>
            <h1>{TITLES[filter]}</h1>
            <p>{d.items.length} {d.items.length === 1 ? "item" : "items"}{d.active.length ? ` · ${d.active.length} downloading` : ""}</p>
          </div>
          <div className="tools">
            <button className="round" aria-label="Add download" title="Add download (⌘N)" onClick={() => setAdding(true)}><Icon name="plus" /></button>
            <button className="round" aria-label="Resume all" title="Resume all" disabled={!canResume} onClick={() => api.resumeAll()}><Icon name="play" /></button>
            <button className="round" aria-label="Pause all" title="Pause all" disabled={!canPause} onClick={() => api.pauseAll()}><Icon name="pause" /></button>
            <button className="round" aria-label="Remove completed" title="Remove completed from the list" disabled={!hasDone} onClick={() => d.items.filter((i) => i.status === "done").forEach((i) => api.remove(i.id, false))}><Icon name="trash" /></button>
            <MoreMenu items={[{ label: "Add Download…", onClick: () => setAdding(true) }, { label: "Preferences…", onClick: () => setSettingsOpen(true) }]} />
          </div>
          <label className="search">
            <Icon name="search" size={15} />
            <input type="search" placeholder="Search downloads…" value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Search downloads" />
          </label>
        </header>

        <div className="content">
          <main>
            {shown.length === 0 ? (
              <div className="empty">
                <img src="/grabnr.svg" alt="" width="64" height="64" />
                <h2>{d.items.length ? "Nothing here" : "No downloads yet"}</h2>
                <p>Paste a link anywhere in this window, press <b>+</b>, or install the browser extension so downloads from Chrome and Brave start here.</p>
                {!d.items.length && <button className="primary" onClick={() => setAdding(true)}>Add Download</button>}
              </div>
            ) : (
              <ul className="list">
                {shown.map((i) => (
                  <DownloadRow key={i.id} item={i} live={d.live[i.id]} selected={current?.id === i.id} onSelect={() => setSelected(i.id)} />
                ))}
              </ul>
            )}
          </main>
          <Inspector item={current} live={current ? d.live[current.id] : undefined} links={d.links} />
        </div>
      </section>

      {adding && <AddDialog defaultDir={d.app?.settings.dest_dir} onClose={() => setAdding(false)} />}
      {settingsOpen && d.app && (
        <SettingsPanel settings={d.app.settings} apiPort={d.app.api_port} apiOk={d.app.api_ok} onChange={(settings: Settings) => d.setApp({ ...d.app!, settings })} onClose={() => setSettingsOpen(false)} />
      )}
    </div>
  );
}
