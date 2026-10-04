import { useEffect, useRef, useState } from "react";
import { api } from "./api";

export function AddDialog({ onClose }: { onClose: () => void }) {
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);

  useEffect(() => {
    input.current?.focus();
    // Pre-fill from the clipboard when it holds a link.
    navigator.clipboard?.readText().then((t) => /^https?:\/\//i.test(t.trim()) && setUrl((u) => u || t.trim())).catch(() => {});
  }, []);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    try {
      await api.add(url, name);
      onClose();
    } catch (x) {
      setErr(String(x));
    }
  }

  return (
    <div className="modal" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <form className="sheet" onSubmit={submit}>
        <h2>Add download</h2>
        <label>
          Link
          <input ref={input} value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://…" spellCheck={false} />
        </label>
        <label>
          Save as <small>(optional)</small>
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Use the server's file name" />
        </label>
        {err && <p className="err">{err}</p>}
        <div className="sheet-actions">
          <button type="button" className="ghost" onClick={onClose}>Cancel</button>
          <button type="submit" className="primary" disabled={!url.trim()}>Download</button>
        </div>
      </form>
    </div>
  );
}
