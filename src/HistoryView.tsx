import { useEffect, useState } from "react";
import { api } from "./api";
import { FileIcon } from "./FileIcon";
import { bytes, duration, linkColor } from "./format";
import { Icon } from "./icons";
import type { HistoryRec, Link } from "./types";
import { locale, t } from "./i18n";

const when = (unix: number) => new Date(unix * 1000).toLocaleString(locale(), { dateStyle: "medium", timeStyle: "short" });

export function HistoryView({ links, refreshKey }: { links: Link[]; refreshKey: number }) {
  const [rows, setRows] = useState<HistoryRec[] | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);

  useEffect(() => void api.history().then(setRows), [refreshKey]);

  if (!rows) return <div className="history"><p className="muted">{t("Loading…")}</p></div>;

  const total = rows.reduce((a, r) => a + r.bytes, 0);
  const saved = rows.reduce((a, r) => a + r.saved_secs, 0);
  const perLink = new Map<string, number>();
  rows.forEach((r) => Object.entries(r.link_bytes).forEach(([n, b]) => perLink.set(n, (perLink.get(n) ?? 0) + b)));
  const wire = [...perLink.values()].reduce((a, b) => a + b, 0) || 1;
  const label = (n: string) => links.find((l) => l.name === n)?.label ?? n;

  async function exportCsv() {
    try {
      const p = await api.exportHistory();
      if (p) setNote(t("History exported."));
    } catch (e) {
      setNote(String(e));
    }
  }

  return (
    <div className="history">
      <div className="stat-cards">
        <div><small>{t("Downloads")}</small><b>{rows.length}</b></div>
        <div><small>{t("Total size")}</small><b>{bytes(total)}</b></div>
        <div title={t("Estimated: how long the fastest single link alone would have taken, minus the actual time")}><small>{t("Time saved (estimate)")}</small><b>{duration(saved)}</b></div>
      </div>

      {perLink.size > 0 && (
        <section>
          <h4>{t("Bytes per connection")}</h4>
          <div className="share-bar" role="img" aria-label={t("Share of bytes per connection")}>
            {[...perLink].map(([n, b]) => <span key={n} style={{ width: `${(b / wire) * 100}%`, background: linkColor(n) }} />)}
          </div>
          <ul className="share-legend">
            {[...perLink].map(([n, b]) => (
              <li key={n}><i style={{ background: linkColor(n) }} />{label(n)}<small>{bytes(b)} · {Math.round((b / wire) * 100)}%</small></li>
            ))}
          </ul>
        </section>
      )}

      <div className="history-tools">
        <button onClick={exportCsv} disabled={rows.length === 0}><Icon name="download" size={14} />{t("Export CSV")}</button>
        {confirming ? (
          <button className="danger solid" autoFocus onBlur={() => setConfirming(false)} onClick={async () => { await api.clearHistory(); setRows([]); setConfirming(false); }}>{t("Clear all history?")}</button>
        ) : (
          <button className="danger" disabled={rows.length === 0} onClick={() => setConfirming(true)}><Icon name="trash" size={14} />{t("Clear history")}</button>
        )}
        {note && <span className="muted small" role="status">{note}</span>}
      </div>

      {rows.length === 0 ? (
        <p className="muted">{t("Finished downloads show up here, even after you remove them from the list.")}</p>
      ) : (
        <ul className="history-list">
          {rows.map((r) => (
            <li key={r.id}>
              <FileIcon name={r.name} size={34} />
              <div className="row-body">
                <span className="name" title={r.url}>{r.name}</span>
                <span className="meta">{bytes(r.bytes)} · {when(r.finished)}</span>
              </div>
              <span className="meta right">{r.saved_secs > 0 ? t("{time} saved", { time: duration(r.saved_secs) }) : ""}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
