import { api } from "./api";
import { FileIcon } from "./FileIcon";
import { bytes, eta, fileName, linkColor, rate } from "./format";
import { Icon } from "./icons";
import { MoreMenu } from "./Menu";
import type { Item, Live } from "./types";
import { FILE_MANAGER } from "./platform";

export function progressOf(item: Item, live?: Live) {
  const downloaded = item.status === "downloading" ? Math.max(live?.downloaded ?? 0, item.downloaded) : item.downloaded;
  const pct = item.total ? Math.min(100, (downloaded / item.total) * 100) : item.status === "done" ? 100 : 0;
  const speed = item.status === "downloading" ? (live?.speed ?? 0) : 0;
  return { downloaded, pct, speed };
}

/** Progress bar; while downloading it is coloured by how many bytes each link carried. */
export function ProgressBar({ item, live, thin }: { item: Item; live?: Live; thin?: boolean }) {
  const { pct } = progressOf(item, live);
  const routes = live?.routes ?? [];
  const wire = routes.reduce((a, r) => a + r.bytes, 0) || 1;
  const split = item.status === "downloading" && routes.length > 1 && routes.some((r) => r.bytes > 0);
  return (
    <div className={`bar ${item.status}${thin ? " thin" : ""}`} role="progressbar" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100}>
      {split ? (
        <div className="fill split" style={{ width: `${pct}%` }}>
          {routes.map((r) => (
            <span key={r.name} style={{ width: `${(r.bytes / wire) * 100}%`, background: linkColor(r.name) }} />
          ))}
        </div>
      ) : (
        <div className="fill" style={{ width: `${pct}%` }} />
      )}
    </div>
  );
}

export function statusLine(item: Item, downloaded: number, speed: number) {
  const size = item.total ? `${bytes(downloaded)} of ${bytes(item.total)}` : bytes(downloaded);
  if (item.status === "downloading") return `${size} · ${rate(speed)}${item.total && speed ? ` · ${eta(item.total - downloaded, speed)} left` : ""}`;
  if (item.status === "queued") return item.error ?? "Waiting to start";
  if (item.status === "paused") return `${item.total ? bytes(item.total) : bytes(downloaded)} · Paused`;
  if (item.status === "error") return item.error ?? "Failed";
  return `${item.total ? bytes(item.total) : bytes(downloaded)} · Completed`;
}

export function PlayPause({ item }: { item: Item }) {
  if (item.status === "downloading" || item.status === "queued") return <button className="round" aria-label="Pause" onClick={(e) => { e.stopPropagation(); api.pause(item.id); }}><Icon name="pause" /></button>;
  if (item.status === "done") return <span className="done-mark" aria-label="Completed"><Icon name="check" size={14} /></span>;
  return <button className="round" aria-label={item.status === "error" ? "Retry" : "Resume"} onClick={(e) => { e.stopPropagation(); api.resume(item.id); }}><Icon name="play" /></button>;
}

export type DragProps = { onDragStart: () => void; onDragOver: () => void; onDrop: () => void; onDragEnd: () => void; over: boolean };

export function DownloadRow({ item, live, selected, onSelect, drag, onChangeLink }: { item: Item; live?: Live; selected: boolean; onSelect: () => void; drag?: DragProps; onChangeLink?: (i: Item) => void }) {
  const { downloaded, pct, speed } = progressOf(item, live);
  const name = fileName(item.url, item.filename);
  return (
    <li
      className={`row ${item.status}${selected ? " selected" : ""}${drag?.over ? " drop-target" : ""}`}
      onClick={onSelect}
      onDoubleClick={() => item.status === "done" && api.open(item.id)}
      draggable={!!drag}
      onDragStart={(e) => { if (drag) { e.dataTransfer.setData("application/x-grabnr-row", item.id); e.dataTransfer.effectAllowed = "move"; drag.onDragStart(); } }}
      onDragOver={(e) => { if (drag) { e.preventDefault(); drag.onDragOver(); } }}
      onDrop={(e) => { if (drag) { e.preventDefault(); e.stopPropagation(); drag.onDrop(); } }}
      onDragEnd={() => drag?.onDragEnd()}
    >
      <FileIcon name={name} />
      <div className="row-body">
        <div className="row-line">
          <span className="name" title={item.url}>{name}</span>
          {item.status !== "done" && <span className="pct">{item.status === "error" ? "" : `${Math.round(pct)}%`}</span>}
        </div>
        <div className={`meta ${item.status === "error" ? "err" : ""}`}>{statusLine(item, downloaded, speed)}</div>
        <ProgressBar item={item} live={live} />
      </div>
      <div className="row-actions">
        <PlayPause item={item} />
        <MoreMenu
          items={[
            ...(item.status === "done" ? [{ label: "Open", onClick: () => api.open(item.id) }, { label: FILE_MANAGER === "Finder" ? "Show in Finder" : "Show in folder", onClick: () => api.reveal(item.id) }] : []),
            ...(item.status === "queued" || item.status === "paused" ? [{ label: "Download next", onClick: () => api.move(item.id, true) }, { label: "Download last", onClick: () => api.move(item.id, false) }] : []),
            ...(onChangeLink && (item.status === "paused" || item.status === "error" || item.status === "queued") ? [{ label: "Change link…", onClick: () => onChangeLink(item) }] : []),
            { label: "Copy link", onClick: () => void navigator.clipboard?.writeText(item.url) },
            "sep",
            { label: "Remove from list", onClick: () => api.remove(item.id, false) },
            ...(item.status !== "done" ? [{ label: "Cancel and delete partial file", danger: true, onClick: () => api.remove(item.id, true) }] : []),
          ]}
        />
      </div>
    </li>
  );
}
