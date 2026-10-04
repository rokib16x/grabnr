import { useEffect, useRef, useState } from "react";
import { api } from "./api";
import { Icon } from "./icons";

const folderName = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;

export function AddDialog({ defaultDir, onClose }: { defaultDir?: string; onClose: () => void }) {
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [sum, setSum] = useState("");
  const [more, setMore] = useState(false);
  const [auth, setAuth] = useState<"none" | "basic" | "token">("none");
  const [user, setUser] = useState("");
  const [pass, setPass] = useState("");
  const [token, setToken] = useState("");
  const [proxy, setProxy] = useState("");
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
      const r = await api.add(url, {
        filename: name || undefined,
        dir: dir === defaultDir ? undefined : dir,
        checksum: sum || undefined,
        username: auth === "basic" ? user : undefined,
        password: auth === "basic" ? pass : undefined,
        token: auth === "token" ? token : undefined,
        proxy: proxy || undefined,
      });
      if (r.duplicate) {
        setErr("This link is already in your list.");
        return;
      }
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
        <button type="button" className="ghost disclose" aria-expanded={more} onClick={() => setMore(!more)}>
          <Icon name="chevron" size={12} className={more ? "open" : ""} />Sign-in and proxy
        </button>
        {more && (
          <div className="advanced">
            <div className="form-row">
              <span>Sign in</span>
              <select value={auth} onChange={(e) => setAuth(e.target.value as typeof auth)} aria-label="Sign-in type">
                <option value="none">None</option>
                <option value="basic">Username and password</option>
                <option value="token">Bearer token</option>
              </select>
            </div>
            {auth === "basic" && (
              <>
                <div className="form-row"><span>Username</span><input value={user} onChange={(e) => setUser(e.target.value)} autoComplete="off" spellCheck={false} aria-label="Username" /></div>
                <div className="form-row"><span>Password</span><input type="password" value={pass} onChange={(e) => setPass(e.target.value)} autoComplete="off" aria-label="Password" /></div>
              </>
            )}
            {auth === "token" && <div className="form-row"><span>Token</span><input type="password" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" aria-label="Bearer token" /></div>}
            <div className="form-row"><span>Proxy</span><input value={proxy} onChange={(e) => setProxy(e.target.value)} placeholder="socks5://127.0.0.1:1080" spellCheck={false} aria-label="Proxy" /></div>
          </div>
        )}
        <p className="hint">grabnr splits the file into chunks and pulls them over every connection you have enabled in Manage Connections.</p>

        {err && <p className="err" role="alert">{err}</p>}
        <div className="sheet-actions">
          <button type="button" onClick={onClose}>Cancel</button>
          <button type="submit" className="primary" disabled={!url.trim()}>Add Download</button>
        </div>
      </form>
    </div>
  );
}
