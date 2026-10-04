import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

type Link = {
  name: string;
  label: string;
  kind: string;
  ipv4: string;
  gateway: string | null;
  is_default_route: boolean;
};
type ModeResult = { mode: string; public_ip: string | null; verdict: string; error: string | null };
type Throughput = { link: string; mbps: number; error: string | null };
type Report = {
  baseline_ip: string | null;
  links: { link: string; modes: ModeResult[]; solo: Throughput }[];
  combined_mbps: number;
  best_solo_mbps: number;
};

export default function App() {
  const [links, setLinks] = useState<Link[]>([]);
  const [shared, setShared] = useState<[string, string][]>([]);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [report, setReport] = useState<Report | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<{ links: Link[]; shared_gateways: [string, string][] }>("get_links").then((r) => {
      setLinks(r.links);
      setShared(r.shared_gateways);
      setPicked(new Set(r.links.map((l) => l.name)));
    });
  }, []);

  async function runSpike() {
    setBusy(true);
    setError(null);
    setReport(null);
    try {
      setReport(await invoke<Report>("run_spike", { only: [...picked], secs: 6 }));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <main>
      <h1>grabnr</h1>
      <p className="sub">Interface binding spike</p>

      <h2>Links</h2>
      {links.length === 0 && <p>No active links found.</p>}
      <ul className="links">
        {links.map((l) => (
          <li key={l.name}>
            <label>
              <input
                type="checkbox"
                checked={picked.has(l.name)}
                onChange={() => {
                  const n = new Set(picked);
                  n.has(l.name) ? n.delete(l.name) : n.add(l.name);
                  setPicked(n);
                }}
              />
              <b>{l.name}</b> {l.label} · {l.kind} · {l.ipv4}
              {l.is_default_route && <span className="tag">default route</span>}
            </label>
          </li>
        ))}
      </ul>
      {shared.map(([a, b]) => (
        <p className="warn" key={a + b}>
          {a} and {b} share a gateway and will not add bandwidth.
        </p>
      ))}

      <button onClick={runSpike} disabled={busy || picked.size === 0}>
        {busy ? "Testing…" : "Run spike"}
      </button>
      {error && <p className="warn">{error}</p>}

      {report && (
        <section>
          <p>Baseline public IP (OS routing): {report.baseline_ip ?? "?"}</p>
          {report.links.map((r) => (
            <div key={r.link} className="card">
              <b>{r.link}</b>
              {r.modes.map((m) => (
                <div key={m.mode}>
                  {m.mode}: {m.public_ip ?? "-"} → <b>{m.verdict}</b>
                  {m.error && ` (${m.error})`}
                </div>
              ))}
              <div>solo: {r.solo.mbps.toFixed(1)} Mbps {r.solo.error && `(${r.solo.error})`}</div>
            </div>
          ))}
          <p>
            Combined <b>{report.combined_mbps.toFixed(1)} Mbps</b> vs best single link{" "}
            {report.best_solo_mbps.toFixed(1)} Mbps
          </p>
        </section>
      )}
    </main>
  );
}
