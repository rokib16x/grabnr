import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import { Icon } from "./icons";

const folderName = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;

export function AddDialog({ defaultDir, onClose }: { defaultDir?: string; onClose: () => void }) {
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [sum, setSum] = useState("");
  const [dir, setDir] = useState(defaultDir ?? "");
  const [err, setErr] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    input.current?.focus();
    // Pre-fill from the clipboard when it holds a link.
    navigator.clipboard?.readText().then((t) => /^https?:\/\//i.test(t.trim()) && setUrl((u) => u || t.trim())).catch(() => {});
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [onClose]);

  const guess = (() => { try { return decodeURIComponent(new URL(url).pathname.split("/").filter(Boolean).pop() ?? ""); } catch { return ""; } })();

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    try {
      await api.add(url, name, dir === defaultDir ? undefined : dir, sum);
      onClose();
    } catch (x) {
      setErr(String(x));
    }
  }

  return (
    <div className="modal" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <form className="sheet add" onSubmit={submit} role="dialog" aria-label="Add Download">
        <h2>Add Download</h2>
        <label className="url-field">
          <Icon name="link" size={16} />
          <input ref={input} value={url} onChange={(e) => { setUrl(e.target.value); setErr(null); }} placeholder="https://example.com/file.zip" spellCheck={false} aria-label="Link" />
        </label>

        <div className="form-row">
          <span>Save to</span>
          <button type="button" className="select" onClick={async () => { const d = await api.pickFolder(); if (d) setDir(d); }} title={dir}>
            <Icon name="folder" size={15} />
            <span>{dir ? folderName(dir) : "Choose…"}</span>
            <Icon name="chevron" size={13} />
          </button>
        </div>
        <div className="form-row">
          <span>Rename</span>
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder={guess || "Use the server's file name"} spellCheck={false} aria-label="Rename" />
        </div>
        <div className="form-row">
          <span>Checksum</span>
          <input value={sum} onChange={(e) => { setSum(e.target.value); setErr(null); }} placeholder="Optional: SHA-256, SHA-1 or MD5" spellCheck={false} aria-label="Checksum" />
        </div>
        <p className="hint">grabnr splits the file into chunks and pulls them over every connection you have enabled in Manage Connections.</p>

        {err && <p className="err">{err}</p>}
        <div className="sheet-actions">
          <button type="button" onClick={onClose}>Cancel</button>
          <button type="submit" className="primary" disabled={!url.trim()}>Add Download</button>
        </div>
      </form>
    </div>
  );
}
