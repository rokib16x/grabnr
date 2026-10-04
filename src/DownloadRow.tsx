import { useState } from "react";
import { api } from "./api";
import { ChunkGrid } from "./ChunkGrid";
import { bytes, eta, rate, routeColor } from "./format";
import { SpeedGraph } from "./SpeedGraph";
import type { Item, Live } from "./types";

const LABEL: Record<Item["status"], string> = { queued: "Queued", downloading: "Downloading", paused: "Paused", done: "Done", error: "Failed" };

export function DownloadRow({ item, live }: { item: Item; live?: Live }) {
  const [open, setOpen] = useState(false);
  const downloaded = item.status === "downloading" ? Math.max(live?.downloaded ?? 0, item.downloaded) : item.downloaded;
  const pct = item.total ? Math.min(100, (downloaded / item.total) * 100) : item.status === "done" ? 100 : 0;
  const speed = item.status === "downloading" ? (live?.speed ?? 0) : 0;
  const routes = live?.routes ?? [];
  const wire = routes.reduce((a, r) => a + r.bytes, 0) || 1;
  const name = item.filename ?? item.url.split("/").pop() ?? item.url;
  const active = item.status === "downloading";

  return (
    <li className={`row ${item.status}`}>
      <div className="row-main" onClick={() => setOpen(!open)}>
        <div className="row-top">
          <span className="name" title={item.url}>{name}</span>
          <span className={`badge ${item.status}`}>{LABEL[item.status]}</span>
        </div>
        <div className="bar" role="progressbar" aria-valuenow={Math.round(pct)} aria-valuemin={0} aria-valuemax={100}>
          {active && routes.length > 1 && routes.some((r) => r.bytes > 0) ? (
            // While downloading, colour the bar by each link's share of the bytes it carried.
            <div className="fill split" style={{ width: `${pct}%` }}>
              {routes.map((r, i) => (
                <span key={r.name} style={{ width: `${(r.bytes / wire) * 100}%`, background: routeColor(i) }} />
              ))}
            </div>
          ) : (
            <div className="fill" style={{ width: `${pct}%` }} />
          )}
        </div>
        <div className="row-meta">
          <span>
            {item.total ? `${bytes(downloaded)} of ${bytes(item.total)}` : bytes(downloaded)}
            {item.total ? ` · ${pct.toFixed(0)}%` : ""}
          </span>
          {active && (
            <span>
              {rate(speed)}
              {item.total ? ` · ${eta(item.total - downloaded, speed)} left` : ""}
            </span>
          )}
          {item.status === "error" && <span className="err">{item.error}</span>}
        </div>
        {active && routes.length > 0 && (
          <div className="chips">
            {routes.map((r, i) => (
              <span key={r.name} className="chip">
                <i style={{ background: routeColor(i) }} />
                {r.name} {rate(r.bytes_per_sec)} <small>{r.connections} conn</small>
              </span>
            ))}
          </div>
        )}
        {live?.notice && <div className="notice">{live.notice}</div>}
      </div>

      <div className="actions">
        {(item.status === "downloading" || item.status === "queued") && <button onClick={() => api.pause(item.id)}>Pause</button>}
        {(item.status === "paused" || item.status === "error") && <button onClick={() => api.resume(item.id)}>{item.status === "error" ? "Retry" : "Resume"}</button>}
        {item.status === "done" && (
          <>
            <button onClick={() => api.open(item.id)}>Open</button>
            <button onClick={() => api.reveal(item.id)}>Show in Finder</button>
          </>
        )}
        <button className="ghost" onClick={() => api.remove(item.id, false)}>Remove</button>
        {item.status !== "done" && (
          <button className="ghost danger" onClick={() => confirm("Remove and delete the partial file?") && api.remove(item.id, true)}>Delete</button>
        )}
      </div>

      {open && (
        <div className="detail">
          <div className="url">{item.url}</div>
          {live && live.chunks.length > 0 && (
            <>
              <h4>
                Chunks <small>{live.chunks.length} total{live.resumedChunks ? ` · ${live.resumedChunks} kept from an earlier session` : ""}</small>
              </h4>
              <ChunkGrid chunks={live.chunks} version={live.v} />
            </>
          )}
          {active && live && live.history.length > 0 && (
            <>
              <h4>Speed per link</h4>
              <SpeedGraph history={live.history} names={routes.map((r) => r.name)} />
            </>
          )}
          {!live && item.status !== "done" && <p className="muted">Details appear once the download is running.</p>}
        </div>
      )}
    </li>
  );
}
