import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "./api";
import { Icon } from "./icons";
import type { AddOptions, HlsVariant, PageLink } from "./types";
import { t } from "./i18n";

const folderName = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
const isUrl = (s: string) => /^((https?|s?ftp):\/\/|magnet:\?)\S+$/i.test(s);

export function AddDialog({ defaultDir, onClose }: { defaultDir?: string; onClose: () => void }) {
  const [text, setText] = useState("");
  const [name, setName] = useState("");
  const [sum, setSum] = useState("");
  const [more, setMore] = useState(false);
  const [auth, setAuth] = useState<"none" | "basic" | "token">("none");
  const [user, setUser] = useState("");
  const [pass, setPass] = useState("");
  const [token, setToken] = useState("");
  const [proxy, setProxy] = useState("");
  const [mirrors, setMirrors] = useState("");
  const [variants, setVariants] = useState<HlsVariant[] | null>(null);
  const [quality, setQuality] = useState("best");
  const [dir, setDir] = useState(defaultDir ?? "");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [page, setPage] = useState<{ links: PageLink[]; picked: Set<string>; filter: string } | null>(null);
  const input = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    input.current?.focus();
    // Pre-fill from the clipboard when it holds a link.
    navigator.clipboard?.readText().then((t) => isUrl(t.trim().split(/\s+/)[0] ?? "") && setText((u) => u || t.trim())).catch(() => {});
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [onClose]);

  const urls = useMemo(() => [...new Set(text.split(/\s+/).filter(isUrl))], [text]);
  const many = urls.length > 1;
  const guess = (() => { try { return decodeURIComponent(new URL(urls[0] ?? "").pathname.split("/").filter(Boolean).pop() ?? ""); } catch { return ""; } })();

  const options = (): AddOptions => ({
    filename: !many && name ? name : undefined,
    dir: dir === defaultDir ? undefined : dir,
    checksum: !many && sum ? sum : undefined,
    username: auth === "basic" ? user : undefined,
    password: auth === "basic" ? pass : undefined,
    token: auth === "token" ? token : undefined,
    proxy: proxy || undefined,
    mirrors: !many && mirrors.trim() ? mirrors.split(/\s+/).filter(Boolean) : undefined,
    quality: quality !== "best" ? quality : undefined,
  });

  async function report(r: { added: number; duplicates: number; skipped: number }) {
    if (r.added === 0) {
      setErr(r.duplicates ? t("Those links are already in your list.") : t("None of those links could be added."));
      return;
    }
    onClose();
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (busy) return;
    setBusy(true);
    try {
      if (many) {
        await report(await api.addBatch(urls.join("\n"), options()));
      } else {
        const r = await api.add(urls[0] ?? text, options());
        await report({ added: r.duplicate ? 0 : 1, duplicates: r.duplicate ? 1 : 0, skipped: 0 });
      }
    } catch (x) {
      setErr(String(x));
    } finally {
      setBusy(false);
    }
  }

  const isStream = urls.length === 1 && /\.m3u8($|[?#])/i.test(urls[0]);
  // A different link means a different list of qualities.
  useEffect(() => { setVariants(null); setQuality("best"); }, [urls[0]]);

  async function findQualities() {
    setBusy(true);
    setErr(null);
    try {
      const v = await api.hlsVariants(urls[0]);
      setVariants(v);
      if (v.length === 0) setErr(t("This stream has only one quality."));
    } catch (x) {
      setErr(String(x));
    } finally {
      setBusy(false);
    }
  }

  async function findLinks() {
    setBusy(true);
    setErr(null);
    try {
      const links = await api.grabLinks(urls[0] ?? text);
      setPage({ links, picked: new Set(), filter: "" });
    } catch (x) {
      setErr(String(x));
    } finally {
      setBusy(false);
    }
  }

  async function addPicked() {
    if (!page || page.picked.size === 0) return;
    setBusy(true);
    try {
      await report(await api.addBatch([...page.picked].join("\n"), options()));
    } catch (x) {
      setErr(String(x));
    } finally {
      setBusy(false);
    }
  }

  if (page) {
    const f = page.filter.trim().toLowerCase();
    const shown = page.links.filter((l) => !f || l.url.toLowerCase().includes(f) || l.text.toLowerCase().includes(f));
    const toggle = (u: string) => {
      const picked = new Set(page.picked);
      picked.has(u) ? picked.delete(u) : picked.add(u);
      setPage({ ...page, picked });
    };
    return (
      <div className="modal" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
        <div className="sheet add" role="dialog" aria-label={t("Links on this page")}>
          <h2>{t("Links on this page")}</h2>
          <label className="search wide">
            <Icon name="search" size={15} />
            <input value={page.filter} onChange={(e) => setPage({ ...page, filter: e.target.value })} placeholder={t("Filter, for example .zip or .pdf")} aria-label={t("Filter links")} autoFocus />
          </label>
          <ul className="pick-list">
            {shown.map((l) => (
              <li key={l.url}>
                <label>
                  <input type="checkbox" checked={page.picked.has(l.url)} onChange={() => toggle(l.url)} />
                  <span className="pick-text"><b>{l.text}</b><small>{l.url}</small></span>
                </label>
              </li>
            ))}
            {shown.length === 0 && <li className="muted">{t("No links match.")}</li>}
          </ul>
          {err && <p className="err" role="alert">{err}</p>}
          <div className="sheet-actions">
            <button type="button" className="ghost" onClick={() => setPage({ ...page, picked: new Set(shown.map((l) => l.url)) })}>{t("Select shown")}</button>
            <button type="button" className="ghost" onClick={() => setPage({ ...page, picked: new Set() })}>{t("Clear")}</button>
            <span className="spacer" />
            <button type="button" onClick={() => setPage(null)}>{t("Back")}</button>
            <button type="button" className="primary" disabled={page.picked.size === 0 || busy} onClick={addPicked}>
              {page.picked.size === 1 ? t("Add 1 Download") : t("Add {n} Downloads", { n: page.picked.size })}
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="modal" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <form className="sheet add" onSubmit={submit} role="dialog" aria-label={t("Add Download")}>
        <h2>{t("Add Download")}</h2>
        <label className="url-field">
          <Icon name="link" size={16} />
          <textarea
            ref={input}
            value={text}
            rows={Math.min(5, Math.max(1, text.split("\n").length))}
            onChange={(e) => { setText(e.target.value); setErr(null); }}
            onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey && !text.includes("\n")) { e.preventDefault(); e.currentTarget.form?.requestSubmit(); } }}
            placeholder={t("https://example.com/file.zip (paste several links to add them all)")}
            spellCheck={false}
            aria-label={t("Links")}
          />
        </label>
        {isStream && (
          <div className="form-row">
            <span>{t("Quality")}</span>
            {variants && variants.length > 0 ? (
              <select value={quality} onChange={(e) => setQuality(e.target.value)} aria-label={t("Quality")}>
                <option value="best">{t("Best available")}</option>
                {variants.map((v) => <option key={v.url} value={String(v.height ?? "best")}>{v.label}</option>)}
                <option value="worst">{t("Smallest")}</option>
              </select>
            ) : (
              <button type="button" className="select" onClick={findQualities} disabled={busy}><span>{t("Best available. Choose quality…")}</span></button>
            )}
          </div>
        )}
        {urls.length === 1 && !isStream && (
          <button type="button" className="ghost disclose" onClick={findLinks} disabled={busy}>
            <Icon name="search" size={13} />{t("List the links on this page…")}</button>
        )}

        <div className="form-row">
          <span>{t("Save to")}</span>
          <button type="button" className="select" onClick={async () => { const d = await api.pickFolder(); if (d) setDir(d); }} title={dir}>
            <Icon name="folder" size={15} />
            <span>{dir ? folderName(dir) : t("Choose…")}</span>
            <Icon name="chevron" size={13} />
          </button>
        </div>
        {!many && (
          <>
            <div className="form-row">
              <span>{t("Rename")}</span>
              <input value={name} onChange={(e) => setName(e.target.value)} placeholder={guess || t("Use the server's file name")} spellCheck={false} aria-label={t("Rename")} />
            </div>
            <div className="form-row">
              <span>{t("Checksum")}</span>
              <input value={sum} onChange={(e) => { setSum(e.target.value); setErr(null); }} placeholder={t("Optional: SHA-256, SHA-1 or MD5")} spellCheck={false} aria-label={t("Checksum")} />
            </div>
          </>
        )}
        <button type="button" className="ghost disclose" aria-expanded={more} onClick={() => setMore(!more)}>
          <Icon name="chevron" size={12} className={more ? "open" : ""} />{t("More options")}</button>
        {more && (
          <div className="advanced">
            <div className="form-row">
              <span>{t("Sign in")}</span>
              <select value={auth} onChange={(e) => setAuth(e.target.value as typeof auth)} aria-label={t("Sign-in type")}>
                <option value="none">{t("None")}</option>
                <option value="basic">{t("Username and password")}</option>
                <option value="token">{t("Bearer token")}</option>
              </select>
            </div>
            {auth === "basic" && (
              <>
                <div className="form-row"><span>{t("Username")}</span><input value={user} onChange={(e) => setUser(e.target.value)} autoComplete="off" spellCheck={false} aria-label={t("Username")} /></div>
                <div className="form-row"><span>{t("Password")}</span><input type="password" value={pass} onChange={(e) => setPass(e.target.value)} autoComplete="off" aria-label={t("Password")} /></div>
              </>
            )}
            {auth === "token" && <div className="form-row"><span>{t("Token")}</span><input type="password" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" aria-label={t("Bearer token")} /></div>}
            <div className="form-row"><span>{t("Proxy")}</span><input value={proxy} onChange={(e) => setProxy(e.target.value)} placeholder={t("socks5://127.0.0.1:1080")} spellCheck={false} aria-label={t("Proxy")} /></div>
            {!many && (
              <div className="form-row top">
                <span>{t("Mirrors")}</span>
                <textarea className="plain" rows={2} value={mirrors} onChange={(e) => setMirrors(e.target.value)} placeholder={t("Other links to the same file, one per line")} spellCheck={false} aria-label={t("Mirrors")} />
              </div>
            )}
          </div>
        )}
        <p className="hint">
          {many ? t("{n} links will be added.", { n: urls.length }) : urls[0] && /^ftp:/i.test(urls[0]) ? t("FTP sends the password without encryption. Use sftp:// if the server offers it.") : urls[0] && /^magnet:/i.test(urls[0]) ? t("Torrents are split by file over your connections. A single large file uses one connection.") : t("grabnr splits the file into chunks and pulls them over every connection you have enabled in Manage Connections. Metalink files (.meta4) add their mirrors and checksum automatically.")}
        </p>

        {err && <p className="err" role="alert">{err}</p>}
        <div className="sheet-actions">
          <button type="button" onClick={onClose}>{t("Cancel")}</button>
          <button type="submit" className="primary" disabled={urls.length === 0 || busy}>{many ? t("Add {n} Downloads", { n: urls.length }) : t("Add Download")}</button>
        </div>
      </form>
    </div>
  );
}
