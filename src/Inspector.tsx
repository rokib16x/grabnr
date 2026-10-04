import { useEffect, useState } from "react";
import { api } from "./api";
import { ChunkGrid } from "./ChunkGrid";
import { FileIcon } from "./FileIcon";
import { bytes, eta, fileName, linkColor, rate } from "./format";
import { Icon, linkIcon } from "./icons";
import { ProgressBar, progressOf } from "./DownloadRow";
import { SpeedGraph } from "./SpeedGraph";
import type { Item, Link, Live } from "./types";
import { FILE_MANAGER } from "./platform";
import { t } from "./i18n";

export function Inspector({ item, live, links }: { item?: Item; live?: Live; links: Link[] }) {
  const [confirming, setConfirming] = useState(false);
  const [thumb, setThumb] = useState<string | null>(null);
  useEffect(() => setConfirming(false), [item?.id]);
  // A preview of finished files (macOS Quick Look); nothing when none can be made.
  useEffect(() => {
    setThumb(null);
    if (item?.status !== "done") return;
    let live = true;
    api.thumbnail(item.id).then((t) => live && setThumb(t)).catch(() => {});
    return () => { live = false; };
  }, [item?.id, item?.status]);

  if (!item) {
    return (
      <aside className="inspector empty-pane">
        <Icon name="download" size={28} />
        <p>{t("Select a download to see its details.")}</p>
      </aside>
    );
  }
  const { downloaded, pct, speed } = progressOf(item, live);
  const name = fileName(item.url, item.filename);
  const routes = live?.routes ?? [];
  const active = item.status === "downloading";
  const kindOf = (n: string) => links.find((l) => l.name === n)?.kind ?? "other";

  return (
    <aside className="inspector">
      <div className="preview">{thumb ? <img className="thumb" src={thumb} alt={t("")} /> : <FileIcon name={name} size={84} />}</div>
      <h2 title={name}>{name}</h2>
      <a className="url" title={item.url} href={item.url} onClick={(e) => { e.preventDefault(); void navigator.clipboard?.writeText(item.url); }}>{item.url}</a>

      <div className="insp-progress">
        <ProgressBar item={item} live={live} />
        <span>{Math.round(pct)}%</span>
      </div>

      <dl className="stats">
        <div><dt><Icon name="download" size={15} />{t("Downloaded")}</dt><dd>{item.total ? `${bytes(downloaded)} of ${bytes(item.total)}` : bytes(downloaded)}</dd></div>
        <div><dt><Icon name="bolt" size={15} />{t("Speed")}</dt><dd>{active ? rate(speed) : "—"}</dd></div>
        <div><dt><Icon name="clock" size={15} />{t("Time left")}</dt><dd>{active && item.total && speed ? eta(item.total - downloaded, speed) : "—"}</dd></div>
        <div><dt><Icon name="link" size={15} />{t("Connections")}</dt><dd>{active ? t("{n} active", { n: routes.reduce((a, r) => a + r.connections, 0) }) : "—"}</dd></div>
      </dl>

      {active && routes.length > 0 && (
        <div className="link-chips">
          {routes.filter((r) => !r.down).map((r) => (
            <span key={r.name} className="link-chip" title={t("{name} · {n} connections", { name: r.name, n: r.connections })}>
              <i style={{ background: linkColor(r.name) }}><Icon name={linkIcon(kindOf(r.name))} size={12} /></i>
              {rate(r.bytes_per_sec)}
            </span>
          ))}
        </div>
      )}
      {item.status === "error" && <p className="notice err">{item.error}</p>}
      {live?.notice && <p className="notice">{live.notice}</p>}

      {live && live.chunks.length > 0 && (
        <section>
          <h4>{t("Chunks")}<small>{live.chunks.length}{live.resumedChunks ? ` · ${live.resumedChunks} kept from an earlier session` : ""}</small></h4>
          <ChunkGrid chunks={live.chunks} version={live.v} names={routes.map((r) => r.name)} />
        </section>
      )}
      {active && live && live.history.length > 0 && (
        <section>
          <h4>{t("Speed per link")}</h4>
          <SpeedGraph history={live.history} names={routes.map((r) => r.name)} />
        </section>
      )}

      <div className="insp-actions">
        {(active || item.status === "queued") && <button onClick={() => api.pause(item.id)}><Icon name="pause" size={14} />{t("Pause")}</button>}
        {(item.status === "paused" || item.status === "error") && <button onClick={() => api.resume(item.id)}><Icon name="play" size={14} />{item.status === "error" ? t("Retry") : t("Resume")}</button>}
        {item.status === "done" && <button onClick={() => api.open(item.id)}>{t("Open")}</button>}
        {item.status === "done" && <button onClick={() => api.reveal(item.id)}><Icon name="folder" size={14} />{FILE_MANAGER === "Finder" ? t("Show in Finder") : t("Show in folder")}</button>}
        {item.status === "done" ? (
          <button onClick={() => api.remove(item.id, false)}>{t("Remove")}</button>
        ) : confirming ? (
          <button className="danger solid" onClick={() => api.remove(item.id, true)} onBlur={() => setConfirming(false)} autoFocus>{t("Delete partial file?")}</button>
        ) : (
          <button className="danger" onClick={() => setConfirming(true)}><Icon name="trash" size={14} />{t("Cancel")}</button>
        )}
      </div>
    </aside>
  );
}
