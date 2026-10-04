import { useEffect, useState } from "react";
import { api } from "./api";
import { linkColor } from "./format";
import { ScheduleEditor } from "./ScheduleEditor";
import type { LinkRule, LinksResponse, Settings as S, SpikeReport } from "./types";

export function SettingsPanel({ settings, apiPort, apiOk, onChange, onClose }: { settings: S; apiPort: number; apiOk: boolean; onChange: (s: S) => void; onClose: () => void }) {
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

  const patch = async (p: Partial<S>) => onChange(await api.setSettings(p));
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
          <h2>Settings</h2>
          <button className="primary" onClick={onClose}>Done</button>
        </div>

        <section>
          <h3>Downloads</h3>
          <div className="field">
            <span>Save to</span>
            <code>{settings.dest_dir}</code>
            <button onClick={async () => { const d = await api.pickFolder(); if (d) patch({ dest_dir: d }); }}>Change…</button>
          </div>
          <div className="field">
            <span>Max connections per link</span>
            <input type="number" min={1} max={32} value={settings.conns_per_route} onChange={(e) => patch({ conns_per_route: +e.target.value })} />
          </div>
          <div className="field">
            <span>Speed limit (MB/s, 0 = none)</span>
            <input type="number" min={0} max={100000} step={0.5} value={settings.speed_limit_kbps / 1024} onChange={(e) => patch({ speed_limit_kbps: Math.round(Math.max(0, +e.target.value) * 1024) })} />
          </div>
          <div className="field">
            <span>Proxy (all downloads)</span>
            <input defaultValue={settings.proxy} placeholder="None, or socks5://127.0.0.1:1080" spellCheck={false} onBlur={(e) => e.target.value !== settings.proxy && patch({ proxy: e.target.value })} />
          </div>
          <div className="field">
            <span>Retry failed downloads</span>
            <input type="number" min={0} max={10} value={settings.auto_retry} onChange={(e) => patch({ auto_retry: +e.target.value })} />
            <span className="muted">times, with growing delays</span>
          </div>
          <div className="field">
            <span>Simultaneous downloads</span>
            <input type="number" min={1} max={10} value={settings.max_active} onChange={(e) => patch({ max_active: +e.target.value })} />
          </div>
        </section>

        <section>
          <h3>Schedule</h3>
          <ScheduleEditor settings={settings} onChange={onChange} />
        </section>

        <section>
          <h3>When all downloads finish</h3>
          <div className="field">
            <span>Command to run</span>
            <input defaultValue={settings.after_command} placeholder="Optional shell command, for example: open ~/Downloads" spellCheck={false} onBlur={(e) => e.target.value !== settings.after_command && patch({ after_command: e.target.value })} />
          </div>
          <p className="muted">Pick what happens (sleep, quit or run this command) from the ··· menu in the main window. It applies once and is not remembered after you quit.</p>
        </section>

        <section>
          <h3>Files and integrations</h3>
          <label className="link">
            <input type="checkbox" checked={settings.quarantine} onChange={(e) => patch({ quarantine: e.target.checked })} />
            Mark finished files as downloaded from the internet (macOS checks apps before they first open)
          </label>
          <div className="field">
            <span>Webhook</span>
            <input defaultValue={settings.webhook_url} placeholder="Optional URL that receives a JSON event for each finished or failed download" spellCheck={false} onBlur={(e) => e.target.value !== settings.webhook_url && patch({ webhook_url: e.target.value })} />
          </div>
          <p className="muted">The webhook receives the file name, link, size and result. Nothing is sent unless you set one.</p>
        </section>

        <section>
          <h3>Backup and support</h3>
          <div className="field">
            <button onClick={async () => { try { setNote((await api.exportSettings()) ? "Settings exported." : null); } catch (e) { setNote(String(e)); } }}>Export settings…</button>
            <button onClick={async () => { try { const s = await api.importSettings(); if (s) { onChange(s); setNote("Settings imported."); } } catch (e) { setNote(String(e)); } }}>Import settings…</button>
            <button onClick={async () => { try { setNote((await api.exportQueue()) ? "Link list saved." : null); } catch (e) { setNote(String(e)); } }}>Export link list…</button>
          </div>
          <p className="muted">Settings files leave out the extension token and any proxy password. The link list holds everything not finished yet, one per line, and can be pasted into Add Download.</p>
          <div className="field">
            <button onClick={async () => { const r = await api.diagnostics(); setDiag(r); try { await navigator.clipboard.writeText(r); setNote("Report copied. It contains no file names or full links."); } catch { setNote("Select the report below and copy it."); } }}>Copy diagnostics report</button>
          </div>
          {diag && <pre className="report-text">{diag}</pre>}
          {note && <p className="muted small" role="status">{note}</p>}
        </section>

        <section>
          <h3>System</h3>
          <label className="link">
            <input type="checkbox" checked={!!autostart} disabled={autostart === null} onChange={(e) => toggleAutostart(e.target.checked)} />
            Launch at login
          </label>
          <label className="link">
            <input type="checkbox" checked={settings.sound} onChange={(e) => patch({ sound: e.target.checked })} />
            Play a sound when a download finishes
          </label>
          <p className="muted">
            Starts hidden in the menu bar so the browser extension always has somewhere to send downloads. Closing the window keeps grabnr running; quit it from the menu bar icon.
          </p>
          {autostartErr && <p className="err">{autostartErr}</p>}
        </section>

        <section>
          <h3>Network links</h3>
          {!links && <p className="muted">Looking for links…</p>}
          {links?.links.length === 0 && <p className="muted">No active links found. Downloads use the system default route.</p>}
          {links?.links.map((l) => {
            const rule = settings.link_rules[l.name] ?? { share_pct: 100, limit_kbps: 0 };
            const setRule = (p: Partial<LinkRule>) => patch({ link_rules: { ...settings.link_rules, [l.name]: { ...rule, ...p } } });
            return (
              <div key={l.name} className="link-rule">
                <label className="link">
                  <input type="checkbox" checked={enabled(l.name)} onChange={() => toggle(l.name)} />
                  <i style={{ background: linkColor(l.name) }} />
                  <span><b>{l.name}</b> {l.label} <span className="muted">{l.kind.replace("_", "-")} · {l.ipv4}{l.is_default_route ? " · default route" : ""}</span></span>
                </label>
                {enabled(l.name) && (
                  <div className="rule-fields">
                    <label>Share
                      <select value={rule.share_pct} onChange={(e) => setRule({ share_pct: +e.target.value })} aria-label={`${l.label} share of connections`}>
                        {[100, 75, 50, 25].map((p) => <option key={p} value={p}>{p}%</option>)}
                      </select>
                    </label>
                    <label>Limit
                      <input type="number" min={0} step={0.5} value={rule.limit_kbps / 1024} onChange={(e) => setRule({ limit_kbps: Math.round(Math.max(0, +e.target.value) * 1024) })} aria-label={`${l.label} speed limit in MB/s`} /> MB/s
                    </label>
                  </div>
                )}
              </div>
            );
          })}
          <label className="link">
            <input type="checkbox" checked={settings.skip_cellular} onChange={(e) => patch({ skip_cellular: e.target.checked })} />
            Leave cellular links out unless ticked above
          </label>
          {links?.shared_gateways.map(([a, b]) => (
            <p key={a + b} className="notice">{a} and {b} share a gateway, so using both will not add bandwidth.</p>
          ))}
          <button onClick={runTest} disabled={testing || !links?.links.length}>{testing ? "Testing (about 30 s)…" : "Test links"}</button>
          {testErr && <p className="err">{testErr}</p>}
          {report && (
            <div className="report">
              {report.links.map((r) => (
                <div key={r.link}>
                  <b>{r.link}</b>: {r.solo.mbps.toFixed(1)} Mbps alone
                  {r.modes.map((m) => <span key={m.mode} className={`pill ${m.verdict}`}>{m.mode} {m.verdict.replace("_", " ")}</span>)}
                </div>
              ))}
              <p>Together {report.combined_mbps.toFixed(1)} Mbps vs best single link {report.best_solo_mbps.toFixed(1)} Mbps.</p>
              <p className="muted">"bound" means the traffic really left through that link. "same egress" means the same public address as normal routing, which is expected when links share an ISP.</p>
            </div>
          )}
        </section>

        <section>
          <h3>Browser extension</h3>
          <p className="muted">
            {apiOk ? `Listening on 127.0.0.1:${apiPort}.` : `Could not listen on port ${apiPort} (another program is using it). The extension cannot connect.`}
          </p>
          <div className="field">
            <button onClick={async () => setPairLeft(await api.allowPairing())} disabled={!apiOk}>
              {pairLeft > 0 ? `Pairing open: ${pairLeft}s` : "Allow pairing (60 s)"}
            </button>
            <span className="muted">then click Pair in the extension options</span>
          </div>
          <div className="field">
            <span>Token</span>
            <code>{showToken ? settings.token : "•".repeat(16)}</code>
            <button className="ghost" onClick={() => setShowToken(!showToken)}>{showToken ? "Hide" : "Show"}</button>
            <button className="ghost" onClick={() => navigator.clipboard.writeText(settings.token)}>Copy</button>
          </div>
        </section>
      </div>
    </div>
  );
}
