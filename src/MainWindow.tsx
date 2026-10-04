import { useCallback, useEffect, useMemo, useState } from "react";
import { api, onOpenSettings } from "./api";
import { AddDialog } from "./AddDialog";
import { CommandPalette, type Command } from "./CommandPalette";
import { DownloadRow, progressOf, type DragProps } from "./DownloadRow";
import { fileName } from "./format";
import { HistoryView } from "./HistoryView";
import { Icon } from "./icons";
import { Inspector } from "./Inspector";
import { LinkDialog } from "./LinkDialog";
import { MoreMenu } from "./Menu";
import { Onboarding } from "./Onboarding";
import { SettingsPanel } from "./Settings";
import { filterTitle, matchFilter, Sidebar, type Filter } from "./Sidebar";
import { useDownloads } from "./store";
import type { AfterAll, Item, Live, Settings } from "./types";
import { MOD } from "./platform";

type Sort = "queue" | "newest" | "oldest" | "name" | "size" | "progress";
const SORTS: { id: Sort; label: string }[] = [
  { id: "queue", label: "Queue order (drag to reorder)" },
  { id: "newest", label: "Newest first" },
  { id: "oldest", label: "Oldest first" },
  { id: "name", label: "Name" },
  { id: "size", label: "Size" },
  { id: "progress", label: "Progress" },
];

function sorter(sort: Sort, live: Record<string, Live>, order: Map<string, number>) {
  const name = (i: Item) => fileName(i.url, i.filename).toLowerCase();
  const pct = (i: Item) => progressOf(i, live[i.id]).pct;
  switch (sort) {
    case "queue": return (a: Item, b: Item) => (order.get(a.id) ?? 0) - (order.get(b.id) ?? 0);
    case "oldest": return (a: Item, b: Item) => a.added - b.added;
    case "name": return (a: Item, b: Item) => name(a).localeCompare(name(b));
    case "size": return (a: Item, b: Item) => (b.total ?? 0) - (a.total ?? 0);
    case "progress": return (a: Item, b: Item) => pct(b) - pct(a);
    default: return (a: Item, b: Item) => b.added - a.added;
  }
}

function remembered<T extends string>(key: string, fallback: T, allowed: readonly T[]): T {
  try {
    const v = localStorage.getItem(key) as T | null;
    return v && allowed.includes(v) ? v : fallback;
  } catch {
    return fallback;
  }
}

/** URLs in dropped text or a dropped text file; one per line or whitespace separated. */
async function urlsFromDrop(dt: DataTransfer): Promise<string> {
  const parts: string[] = [dt.getData("text/uri-list"), dt.getData("text/plain")];
  for (const f of Array.from(dt.files)) {
    if (/\.(txt|list|csv|urls?)$/i.test(f.name) && f.size < 2_000_000) parts.push(await f.text());
  }
  return parts.join("\n");
}

export function MainWindow() {
  const d = useDownloads();
  const [filter, setFilter] = useState<Filter>("all");
  const [sort, setSort] = useState<Sort>(() => remembered("grabnr.sort", "newest", SORTS.map((s) => s.id)));
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [palette, setPalette] = useState(false);
  const [dropping, setDropping] = useState(false);
  const [toast, setToast] = useState<string | null>(null);
  const [skipOnboarding, setSkipOnboarding] = useState(false);
  const [relink, setRelink] = useState<Item | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const [overId, setOverId] = useState<string | null>(null);

  useEffect(() => onOpenSettings(() => setSettingsOpen(true)), []);
  useEffect(() => { try { localStorage.setItem("grabnr.sort", sort); } catch { /* storage may be unavailable */ } }, [sort]);
  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 3500);
    return () => clearTimeout(t);
  }, [toast]);

  const addText = useCallback(async (text: string) => {
    try {
      const r = await api.addBatch(text);
      setToast(r.added ? `Added ${r.added} ${r.added === 1 ? "download" : "downloads"}${r.duplicates ? `, ${r.duplicates} already in the list` : ""}.` : r.duplicates ? "Already in your list." : "No links added.");
    } catch (e) {
      setToast(String(e));
    }
  }, []);

  // Paste a link anywhere in the window to add it; keyboard shortcuts; drag links or text files onto the window.
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      const t = e.clipboardData?.getData("text")?.trim() ?? "";
      if (/^((https?|s?ftp):\/\/|magnet:\?)\S+/i.test(t)) void addText(t);
    };
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey)) return;
      const k = e.key.toLowerCase();
      if (k === "n") { e.preventDefault(); setAdding(true); }
      if (k === ",") { e.preventDefault(); setSettingsOpen(true); }
      if (k === "k") { e.preventDefault(); setPalette((p) => !p); }
    };
    let depth = 0;
    const hasPayload = (e: DragEvent) => !!e.dataTransfer && (e.dataTransfer.types.includes("Files") || e.dataTransfer.types.includes("text/uri-list") || e.dataTransfer.types.includes("text/plain"));
    const enter = (e: DragEvent) => { if (hasPayload(e)) { depth++; setDropping(true); } };
    const over = (e: DragEvent) => { if (hasPayload(e)) e.preventDefault(); };
    const leave = () => { depth = Math.max(0, depth - 1); if (depth === 0) setDropping(false); };
    const drop = async (e: DragEvent) => {
      e.preventDefault();
      depth = 0;
      setDropping(false);
      if (e.dataTransfer) await addText(await urlsFromDrop(e.dataTransfer));
    };
    window.addEventListener("paste", onPaste);
    window.addEventListener("keydown", onKey);
    window.addEventListener("dragenter", enter);
    window.addEventListener("dragover", over);
    window.addEventListener("dragleave", leave);
    window.addEventListener("drop", drop);
    return () => {
      window.removeEventListener("paste", onPaste);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("dragenter", enter);
      window.removeEventListener("dragover", over);
      window.removeEventListener("dragleave", leave);
      window.removeEventListener("drop", drop);
    };
  }, [addText]);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    // The order the downloader uses: highest priority first, then oldest. Items are stored newest first.
    const order = new Map([...d.items].reverse().sort((a, b) => b.priority - a.priority).map((i, n) => [i.id, n]));
    return d.items
      .filter((i) => matchFilter(filter, i) && (!q || fileName(i.url, i.filename).toLowerCase().includes(q) || i.url.toLowerCase().includes(q)))
      .sort(sorter(sort, d.live, order));
  }, [d.items, d.live, filter, query, sort]);
  const canDrag = sort === "queue" && !query.trim();
  const dragFor = (i: Item): DragProps | undefined =>
    canDrag && i.status !== "done"
      ? {
          over: overId === i.id && dragId !== i.id,
          onDragStart: () => setDragId(i.id),
          onDragOver: () => setOverId(i.id),
          onDrop: () => {
            if (dragId) void api.reorder(dragId, i.id);
            setDragId(null);
            setOverId(null);
          },
          onDragEnd: () => { setDragId(null); setOverId(null); },
        }
      : undefined;
  const AFTER: { id: AfterAll; label: string }[] = [
    { id: "none", label: "Do nothing" },
    { id: "sleep", label: "Put the Mac to sleep" },
    { id: "quit", label: "Quit grabnr" },
    ...(d.app?.settings.after_command.trim() ? [{ id: "command" as AfterAll, label: "Run my command" }] : []),
  ];

  // Keep something selected so the inspector is never blank while there are downloads.
  const current = d.items.find((i) => i.id === selected) ?? shown.find((i) => i.status === "downloading") ?? shown[0];
  const hasDone = d.items.some((i) => i.status === "done");
  const canPause = d.items.some((i) => i.status === "downloading" || i.status === "queued");
  const canResume = d.items.some((i) => i.status === "paused");
  const doneCount = d.items.filter((i) => i.status === "done").length;

  const commands: Command[] = useMemo(() => [
    { id: "add", label: "Add Download…", hint: `${MOD}N`, icon: "plus", run: () => setAdding(true) },
    { id: "pause", label: "Pause All", icon: "pause", run: () => void api.pauseAll() },
    { id: "resume", label: "Resume All", icon: "play", run: () => void api.resumeAll() },
    { id: "clear", label: "Remove Completed from the List", icon: "trash", run: () => d.items.filter((i) => i.status === "done").forEach((i) => void api.remove(i.id, false)) },
    { id: "prefs", label: "Preferences…", hint: `${MOD},`, icon: "sliders", run: () => setSettingsOpen(true) },
    ...(["all", "downloading", "done", "paused", "history"] as Filter[]).map((f) => ({ id: `go-${f}`, label: `Show ${filterTitle(f)}`, icon: "inbox", run: () => setFilter(f) })),
    ...d.items.map((i) => ({ id: `item-${i.id}`, label: fileName(i.url, i.filename), hint: i.status, icon: "download", run: () => { setFilter("all"); setSelected(i.id); } })),
  ], [d.items]);

  return (
    <div className="app">
      <Sidebar items={d.items} filter={filter} onFilter={setFilter} links={d.links} linkSpeeds={d.linkSpeeds} settings={d.app?.settings} onManage={() => setSettingsOpen(true)} />

      <section className="main">
        <header data-tauri-drag-region>
          <div className="title" data-tauri-drag-region>
            <h1>{filterTitle(filter)}</h1>
            <p>{d.items.length} {d.items.length === 1 ? "item" : "items"}{d.active.length ? ` · ${d.active.length} downloading` : ""}</p>
            {(d.schedule.rule || d.afterAll !== "none") && (
              <div className="chips">
                {d.schedule.rule && (
                  <span className="chip warn" title="Set in Preferences, under Schedule">
                    <Icon name="clock" size={12} />
                    {d.schedule.hold ? `Held by schedule: ${d.schedule.rule}` : d.schedule.limit_kbps ? `${d.schedule.rule}: limited to ${(d.schedule.limit_kbps / 1024).toFixed(1)} MB/s` : `${d.schedule.rule}: full speed`}
                  </span>
                )}
                {d.afterAll !== "none" && (
                  <span className="chip">
                    When finished: {AFTER.find((a) => a.id === d.afterAll)?.label ?? d.afterAll}
                    <button className="chip-x" aria-label="Cancel the when-finished action" onClick={() => void api.setAfterAll("none")}><Icon name="x" size={11} /></button>
                  </span>
                )}
              </div>
            )}
          </div>
          <div className="tools">
            <button className="round" aria-label="Add download" title={`Add download (${MOD}N)`} onClick={() => setAdding(true)}><Icon name="plus" /></button>
            <button className="round" aria-label="Resume all" title="Resume all" disabled={!canResume} onClick={() => api.resumeAll()}><Icon name="play" /></button>
            <button className="round" aria-label="Pause all" title="Pause all" disabled={!canPause} onClick={() => api.pauseAll()}><Icon name="pause" /></button>
            <button className="round" aria-label="Remove completed" title="Remove completed from the list" disabled={!hasDone} onClick={() => d.items.filter((i) => i.status === "done").forEach((i) => api.remove(i.id, false))}><Icon name="trash" /></button>
            <MoreMenu
              items={[
                { label: "Add Download…", onClick: () => setAdding(true) },
                { label: "Command Palette…", onClick: () => setPalette(true) },
                { label: "Export link list…", onClick: () => void api.exportQueue().then((p) => p && setToast("Link list saved.")).catch((e) => setToast(String(e))) },
                "sep",
                ...AFTER.map((a) => ({ label: `${d.afterAll === a.id ? "✓ " : "    "}When finished: ${a.label}`, onClick: () => void api.setAfterAll(a.id) })),
                "sep",
                ...SORTS.map((s) => ({ label: `${sort === s.id ? "✓ " : "    "}Sort by ${s.label}`, onClick: () => setSort(s.id) })),
                "sep",
                { label: "Preferences…", onClick: () => setSettingsOpen(true) },
              ]}
            />
          </div>
          <label className="search">
            <Icon name="search" size={15} />
            <input type="search" placeholder="Search downloads…" value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Search downloads" />
          </label>
        </header>

        {filter === "history" ? (
          <div className="content single">
            <main><HistoryView links={d.links} refreshKey={doneCount} /></main>
          </div>
        ) : (
          <div className="content">
            <main>
              {shown.length === 0 ? (
                <div className="empty">
                  <img src="/grabnr.svg" alt="" width="64" height="64" />
                  <h2>{d.items.length ? "Nothing here" : "No downloads yet"}</h2>
                  <p>Paste a link anywhere in this window, drop links or a text file here, press <b>+</b>, or install the browser extension so downloads from Chrome and Brave start here.</p>
                  {!d.items.length && <button className="primary" onClick={() => setAdding(true)}>Add Download</button>}
                </div>
              ) : (
                <ul className="list">
                  {shown.map((i) => (
                    <DownloadRow key={i.id} item={i} live={d.live[i.id]} selected={current?.id === i.id} onSelect={() => setSelected(i.id)} drag={dragFor(i)} onChangeLink={setRelink} />
                  ))}
                </ul>
              )}
            </main>
            <Inspector item={current} live={current ? d.live[current.id] : undefined} links={d.links} />
          </div>
        )}
      </section>

      {dropping && <div className="drop-overlay" aria-hidden="true"><div><Icon name="download" size={28} /><p>Drop links to download</p></div></div>}
      {toast && <div className="toast" role="status">{toast}</div>}
      {adding && <AddDialog defaultDir={d.app?.settings.dest_dir} onClose={() => setAdding(false)} />}
      {relink && <LinkDialog item={relink} onClose={() => setRelink(null)} />}
      {palette && <CommandPalette commands={commands} onClose={() => setPalette(false)} />}
      {settingsOpen && d.app && (
        <SettingsPanel settings={d.app.settings} apiPort={d.app.api_port} apiOk={d.app.api_ok} keychainAvailable={d.app.keychain_available} onChange={(settings: Settings) => d.setApp({ ...d.app!, settings })} onClose={() => setSettingsOpen(false)} />
      )}
      {d.app && !d.app.settings.onboarded && !skipOnboarding && <Onboarding app={d.app} onDone={() => { setSkipOnboarding(true); d.setApp({ ...d.app!, settings: { ...d.app!.settings, onboarded: true } }); }} />}
    </div>
  );
}
