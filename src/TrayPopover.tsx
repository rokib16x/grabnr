import { useEffect, useRef } from "react";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { api } from "./api";
import { FileIcon } from "./FileIcon";
import { fileName, rate } from "./format";
import { Icon } from "./icons";
import { PlayPause, ProgressBar, progressOf, statusLine } from "./DownloadRow";
import { useDownloads } from "./store";

/** Menu bar popover: what is downloading right now, plus the usual app actions. */
export function TrayPopover() {
  const d = useDownloads();
  const root = useRef<HTMLDivElement>(null);
  const running = d.items.filter((i) => i.status === "downloading" || i.status === "queued");
  const recent = d.items.filter((i) => i.status === "paused" || i.status === "error").slice(0, 2);
  const list = [...running, ...recent].slice(0, 4);
  const anyPaused = d.items.some((i) => i.status === "paused");

  // Fit the native window to its content so the vibrancy panel has no empty space.
  useEffect(() => {
    const el = root.current;
    if (!el || !("__TAURI_INTERNALS__" in window)) return;
    const fit = () => void getCurrentWindow().setSize(new LogicalSize(340, Math.ceil(el.getBoundingClientRect().height)));
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    fit();
    return () => ro.disconnect();
  }, []);

  return (
    <div className="tray" ref={root}>
      <div className="tray-head">
        <img src="/grabnr.svg" alt="" width="24" height="24" />
        <strong>grabnr</strong>
        <span className="tray-speed">{running.length ? rate(d.speed) : "Idle"}</span>
      </div>

      {list.length > 0 ? (
        <ul className="tray-list">
          {list.map((i) => {
            const p = progressOf(i, d.live[i.id]);
            return (
              <li key={i.id}>
                <FileIcon name={fileName(i.url, i.filename)} size={34} />
                <div className="row-body">
                  <div className="row-line"><span className="name">{fileName(i.url, i.filename)}</span>{i.status !== "error" && <span className="pct">{Math.round(p.pct)}%</span>}</div>
                  <div className="meta">{statusLine(i, p.downloaded, p.speed)}</div>
                  <ProgressBar item={i} live={d.live[i.id]} thin />
                </div>
                <PlayPause item={i} />
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="tray-empty">No active downloads</p>
      )}

      <div className="tray-menu">
        <button onClick={() => api.showMain()}><Icon name="inbox" />Show All Downloads{d.items.length > 0 && <small>{d.items.length}</small>}</button>
        {running.length > 0 ? (
          <button onClick={() => api.pauseAll()}><Icon name="pause" />Pause All</button>
        ) : (
          <button disabled={!anyPaused} onClick={() => api.resumeAll()}><Icon name="play" />Resume All</button>
        )}
        <hr />
        <button onClick={() => api.showMain(true)}><Icon name="sliders" />Preferences…</button>
        <button onClick={() => api.quit()}><Icon name="power" />Quit grabnr</button>
      </div>
    </div>
  );
}
