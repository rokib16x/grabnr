import { useEffect, useState } from "react";
import { api } from "./api";
import { linkColor } from "./format";
import { ScheduleEditor } from "./ScheduleEditor";
import type { LinkRule, LinksResponse, Settings as S, SpikeReport } from "./types";
import { isMac, TRAY } from "./platform";
import { chooseLanguage, LANGUAGES, savedChoice, t, type Choice } from "./i18n";

export function SettingsPanel({ settings, apiPort, apiOk, keychainAvailable, onChange, onClose }: { settings: S; apiPort: number; apiOk: boolean; keychainAvailable: boolean; onChange: (s: S) => void; onClose: () => void }) {
  const [links, setLinks] = useState<LinksResponse | null>(null);
  const [pairLeft, setPairLeft] = useState(0);
  const [showToken, setShowToken] = useState(false);
  const [report, setReport] = useState<SpikeReport | null>(null);
  const [testing, setTesting] = useState(false);
  const [testErr, setTestErr] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [diag, setDiag] = useState<string | null>(null);
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartErr, setAutostartErr] = useState<string | null>(null);

  useEffect(() => void api.links().then(setLinks), []);
  useEffect(() => void api.autostart().then(setAutostart).catch(() => setAutostart(false)), []);

  async function toggleAutostart(on: boolean) {
    setAutostartErr(null);
    try {
      setAutostart(await api.setAutostart(on));
    } catch (e) {
      setAutostartErr(String(e));
    }
  }
  useEffect(() => {
    if (pairLeft <= 0) return;
    const t = setTimeout(() => setPairLeft(pairLeft - 1), 1000);
    return () => clearTimeout(t);
  }, [pairLeft]);

  const patch = async (p: Partial<S>) => {
    try {
      onChange(await api.setSettings(p));
    } catch (e) {
      setNote(String(e)); // for example the Keychain refusing access
    }
  };
  const enabled = (name: string) => settings.enabled_links === null || settings.enabled_links.includes(name);
  const toggle = (name: string) => {
    const all = links?.links.map((l) => l.name) ?? [];
    const cur = settings.enabled_links ?? all;
    const next = cur.includes(name) ? cur.filter((n) => n !== name) : [...cur, name];
    patch({ enabled_links: next.length === all.length ? null : next });
  };

  async function runTest() {
    setTesting(true);
    setTestErr(null);
    setReport(null);
    try {
      setReport(await api.spike(links?.links.filter((l) => enabled(l.name)).map((l) => l.name) ?? [], 6));
    } catch (e) {
      setTestErr(String(e));
    } finally {
      setTesting(false);
    }
  }

  return (
    <div className="modal" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="sheet wide">
        <div className="sheet-head">
          <h2>{t("Settings")}</h2>
          <button className="primary" onClick={onClose}>{t("Done")}</button>
        </div>

        <section>
          <h3>{t("Downloads")}</h3>
          <div className="field">
            <span>{t("Save to")}</span>
            <code>{settings.dest_dir}</code>
            <button onClick={async () => { const d = await api.pickFolder(); if (d) patch({ dest_dir: d }); }}>{t("Change…")}</button>
          </div>
          <div className="field">
            <span>{t("Max connections per link")}</span>
            <input type="number" min={1} max={32} value={settings.conns_per_route} onChange={(e) => patch({ conns_per_route: +e.target.value })} />
          </div>
          <div className="field">
            <span>{t("Speed limit (MB/s, 0 = none)")}</span>
            <input type="number" min={0} max={100000} step={0.5} value={settings.speed_limit_kbps / 1024} onChange={(e) => patch({ speed_limit_kbps: Math.round(Math.max(0, +e.target.value) * 1024) })} />
          </div>
          <div className="field">
            <span>{t("Proxy (all downloads)")}</span>
            <input defaultValue={settings.proxy} placeholder={t("None, or socks5://127.0.0.1:1080")} spellCheck={false} onBlur={(e) => e.target.value !== settings.proxy && patch({ proxy: e.target.value })} />
          </div>
          <div className="field">
            <span>{t("Retry failed downloads")}</span>
            <input type="number" min={0} max={10} value={settings.auto_retry} onChange={(e) => patch({ auto_retry: +e.target.value })} />
            <span className="muted">{t("times, with growing delays")}</span>
          </div>
          <div className="field">
            <span>{t("Simultaneous downloads")}</span>
            <input type="number" min={1} max={10} value={settings.max_active} onChange={(e) => patch({ max_active: +e.target.value })} />
          </div>
        </section>

        <section>
          <h3>{t("Schedule")}</h3>
          <ScheduleEditor settings={settings} onChange={onChange} />
        </section>

        <section>
          <h3>{t("When all downloads finish")}</h3>
          <div className="field">
            <span>{t("Command to run")}</span>
            <input defaultValue={settings.after_command} placeholder={t("Optional shell command, for example: open ~/Downloads")} spellCheck={false} onBlur={(e) => e.target.value !== settings.after_command && patch({ after_command: e.target.value })} />
          </div>
          <p className="muted">{t("Pick what happens (sleep, quit or run this command) from the ··· menu in the main window. It applies once and is not remembered after you quit.")}</p>
        </section>

        <section>
          <h3>{t("Files and integrations")}</h3>
          {keychainAvailable && (
            <label className="link">
              <input type="checkbox" checked={settings.keychain} onChange={(e) => patch({ keychain: e.target.checked })} />
              {t("Keep sign-in details and proxy passwords in the {where} instead of a file", { where: isMac ? t("macOS Keychain") : t("system keychain") })}
            </label>
          )}
          <label className="link">
            <input type="checkbox" checked={settings.hls_to_mp4} onChange={(e) => patch({ hls_to_mp4: e.target.checked })} />
             {t("Turn streaming videos (.m3u8) into MP4 files when ffmpeg is installed")}</label>
          <label className="link">
            <input type="checkbox" checked={settings.quarantine} onChange={(e) => patch({ quarantine: e.target.checked })} />
             {t("Mark finished files as downloaded from the internet (macOS checks apps before they first open)")}</label>
          <div className="field">
            <span>{t("SFTP private key")}</span>
            <input defaultValue={settings.ssh_key} placeholder={t("Optional path to a key file, for sftp:// links")} spellCheck={false} onBlur={(e) => e.target.value !== settings.ssh_key && patch({ ssh_key: e.target.value })} />
          </div>
          <div className="field">
            <span>{t("Webhook")}</span>
            <input defaultValue={settings.webhook_url} placeholder={t("Optional URL that receives a JSON event for each finished or failed download")} spellCheck={false} onBlur={(e) => e.target.value !== settings.webhook_url && patch({ webhook_url: e.target.value })} />
          </div>
          <p className="muted">{t("The webhook receives the file name, link, size and result. Nothing is sent unless you set one.")}</p>
        </section>

        <section>
          <h3>{t("Backup and support")}</h3>
          <div className="field">
            <button onClick={async () => { try { setNote((await api.exportSettings()) ? t("Settings exported.") : null); } catch (e) { setNote(String(e)); } }}>{t("Export settings…")}</button>
            <button onClick={async () => { try { const s = await api.importSettings(); if (s) { onChange(s); setNote(t("Settings imported.")); } } catch (e) { setNote(String(e)); } }}>{t("Import settings…")}</button>
            <button onClick={async () => { try { setNote((await api.exportQueue()) ? t("Link list saved.") : null); } catch (e) { setNote(String(e)); } }}>{t("Export link list…")}</button>
          </div>
          <p className="muted">{t("Settings files leave out the extension token and any proxy password. The link list holds everything not finished yet, one per line, and can be pasted into Add Download.")}</p>
          <div className="field">
            <button onClick={async () => { const r = await api.diagnostics(); setDiag(r); try { await navigator.clipboard.writeText(r); setNote(t("Report copied. It contains no file names or full links.")); } catch { setNote(t("Select the report below and copy it.")); } }}>{t("Copy diagnostics report")}</button>
          </div>
          {diag && <pre className="report-text">{diag}</pre>}
          {note && <p className="muted small" role="status">{note}</p>}
        </section>

        <section>
          <h3>{t("System")}</h3>
          <div className="field">
            <span>{t("Language")}</span>
            <select aria-label={t("Language")} defaultValue={savedChoice()} onChange={(e) => void chooseLanguage(e.target.value as Choice)}>
              {LANGUAGES.map((l) => <option key={l.id} value={l.id}>{l.id === "system" ? t(l.label) : l.label}</option>)}
            </select>
          </div>
          <label className="link">
            <input type="checkbox" checked={!!autostart} disabled={autostart === null} onChange={(e) => toggleAutostart(e.target.checked)} />
             {t("Launch at login")}</label>
          <label className="link">
            <input type="checkbox" checked={settings.sound} onChange={(e) => patch({ sound: e.target.checked })} />
             {t("Play a sound when a download finishes")}</label>
          <p className="muted">
            {t("Starts hidden in the {tray} so the browser extension always has somewhere to send downloads. Closing the window keeps grabnr running; quit it from the {tray} icon.", { tray: t(TRAY) })}
          </p>
          {autostartErr && <p className="err">{autostartErr}</p>}
        </section>

        <section>
          <h3>{t("Network links")}</h3>
          {!links && <p className="muted">{t("Looking for links…")}</p>}
          {links?.links.length === 0 && <p className="muted">{t("No active links found. Downloads use the system default route.")}</p>}
          {links?.links.map((l) => {
            const rule = settings.link_rules[l.name] ?? { share_pct: 100, limit_kbps: 0 };
            const setRule = (p: Partial<LinkRule>) => patch({ link_rules: { ...settings.link_rules, [l.name]: { ...rule, ...p } } });
            return (
              <div key={l.name} className="link-rule">
                <label className="link">
                  <input type="checkbox" checked={enabled(l.name)} onChange={() => toggle(l.name)} />
                  <i style={{ background: linkColor(l.name) }} />
                  <span><b>{l.name}</b> {l.label} <span className="muted">{l.kind.replace("_", "-")} · {l.ipv4}{l.is_default_route ? ` · ${t("default route")}` : ""}</span></span>
                </label>
                {enabled(l.name) && (
                  <div className="rule-fields">
                    <label>{t("Share")}<select value={rule.share_pct} onChange={(e) => setRule({ share_pct: +e.target.value })} aria-label={t("{name} share of connections", { name: l.label })}>
                        {[100, 75, 50, 25].map((p) => <option key={p} value={p}>{p}%</option>)}
                      </select>
                    </label>
                    <label>{t("Limit")}
                      <input type="number" min={0} step={0.5} value={rule.limit_kbps / 1024} onChange={(e) => setRule({ limit_kbps: Math.round(Math.max(0, +e.target.value) * 1024) })} aria-label={t("{name} speed limit in MB/s", { name: l.label })} /> {t("MB/s")}
                    </label>
                  </div>
                )}
              </div>
            );
          })}
          <label className="link">
            <input type="checkbox" checked={settings.skip_cellular} onChange={(e) => patch({ skip_cellular: e.target.checked })} />
             {t("Leave cellular links out unless ticked above")}</label>
          {links?.binding === "source_address" && (
            <p className="notice">{t("On this system a download is steered to a connection by its address. That works when each connection has its own router; use Test links to check that traffic really leaves through each one.")}</p>
          )}
          {links?.shared_gateways.map(([a, b]) => (
            <p key={a + b} className="notice">{t("{a} and {b} share a gateway, so using both will not add bandwidth.", { a, b })}</p>
          ))}
          <button onClick={runTest} disabled={testing || !links?.links.length}>{testing ? t("Testing (about 30 s)…") : t("Test links")}</button>
          {testErr && <p className="err">{testErr}</p>}
          {report && (
            <div className="report">
              {report.links.map((r) => (
                <div key={r.link}>
                  <b>{r.link}</b>: {t("{mbps} Mbps alone", { mbps: r.solo.mbps.toFixed(1) })}
                  {r.modes.map((m) => <span key={m.mode} className={`pill ${m.verdict}`}>{m.mode} {m.verdict.replace("_", " ")}</span>)}
                </div>
              ))}
              <p>{t("Together {a} Mbps vs best single link {b} Mbps.", { a: report.combined_mbps.toFixed(1), b: report.best_solo_mbps.toFixed(1) })}</p>
              <p className="muted">{t("\"bound\" means the traffic really left through that link. \"same egress\" means the same public address as normal routing, which is expected when links share an ISP.")}</p>
            </div>
          )}
        </section>

        <section>
          <h3>{t("Browser extension")}</h3>
          <p className="muted">
            {apiOk ? t("Listening on 127.0.0.1:{port}.", { port: apiPort }) : t("Could not listen on port {port} (another program is using it). The extension cannot connect.", { port: apiPort })}
          </p>
          <div className="field">
            <button onClick={async () => setPairLeft(await api.allowPairing())} disabled={!apiOk}>
              {pairLeft > 0 ? t("Pairing open: {s}s", { s: pairLeft }) : t("Allow pairing (60 s)")}
            </button>
            <span className="muted">{t("then click Pair in the extension options")}</span>
          </div>
          <div className="field">
            <span>{t("Token")}</span>
            <code>{showToken ? settings.token : "•".repeat(16)}</code>
            <button className="ghost" onClick={() => setShowToken(!showToken)}>{showToken ? t("Hide") : t("Show")}</button>
            <button className="ghost" onClick={() => navigator.clipboard.writeText(settings.token)}>{t("Copy")}</button>
          </div>
        </section>
      </div>
    </div>
  );
}
