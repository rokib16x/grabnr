import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import { fileName } from "./format";
import type { Item } from "./types";
import { t } from "./i18n";

/** Replace the link of a stopped download, for example when a signed link expired. */
export function LinkDialog({ item, onClose }: { item: Item; onClose: () => void }) {
  const [url, setUrl] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    input.current?.focus();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [onClose]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    try {
      await api.updateLink(item.id, url);
      onClose();
    } catch (x) {
      setErr(String(x));
    }
  }

  return (
    <div className="modal" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <form className="sheet" onSubmit={submit} role="dialog" aria-label={t("Change link")}>
        <h2>{t("Change link")}</h2>
        <p className="muted small">{t("New link for {name}. Pieces already downloaded are kept if it is the same file.", { name: fileName(item.url, item.filename) })}</p>
        <input ref={input} value={url} onChange={(e) => { setUrl(e.target.value); setErr(null); }} placeholder={t("https://…")} spellCheck={false} aria-label={t("New link")} />
        {err && <p className="err" role="alert">{err}</p>}
        <div className="sheet-actions">
          <button type="button" onClick={onClose}>{t("Cancel")}</button>
          <button type="submit" className="primary" disabled={!url.trim()}>{t("Change and resume")}</button>
        </div>
      </form>
    </div>
  );
}
