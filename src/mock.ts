// Browser-only stand-in for the Tauri backend, so the UI can be developed and checked without the native shell.
import type { AppState, EngineEvent, Item, LinksResponse, Settings } from "./types";

type Handlers = { item: (i: Item) => void; removed: (id: string) => void; event: (id: string, e: EngineEvent) => void };

let handlers: Handlers | null = null;
const items: Item[] = [];
const settings: Settings = { dest_dir: "/Users/you/Downloads", enabled_links: null, conns_per_route: 8, max_active: 3, speed_limit_kbps: 0, auto_retry: 3, link_rules: {}, skip_cellular: false, proxy: "", token: "d3adbeefcafe0123456789abcdef0123456789abcd" };
const timers = new Map<string, number>();
let autostart = false;

function seed(name: string, total: number, status: Item["status"], downloaded: number): Item {
  const it: Item = { id: Math.random().toString(16).slice(2, 10), url: `https://example.com/files/${name}`, filename: name, dir: settings.dest_dir, status, total, downloaded, path: status === "done" ? `${settings.dest_dir}/${name}` : null, error: null, added: Date.now() / 1000, checksum: null, priority: 0, retries: 0 };
  items.unshift(it);
  return it;
}

function run(it: Item) {
  const chunks = Math.ceil((it.total ?? 0) / (8 << 20));
  const done = new Set<number>();
  let next = 0;
  handlers?.event(it.id, { type: "started", filename: it.filename ?? "file", total: it.total, chunks, ranges: true, resumed_chunks: 0 });
  const speeds = [5.2e6, 8.1e6, 2.2e6];
  const wire = [0, 0, 0];
  const t = window.setInterval(() => {
    const jitter = () => 0.8 + Math.random() * 0.4;
    const rs = speeds.map((s, i) => {
      const bps = s * jitter();
      wire[i] += bps * 0.25;
      return { name: ["en0", "en5", "en7"][i], bytes: wire[i], bytes_per_sec: bps, connections: 8, down: false };
    });
    const total = rs.reduce((a, r) => a + r.bytes_per_sec, 0);
    it.downloaded = Math.min(it.total ?? 0, it.downloaded + total * 0.25);
    const should = Math.floor((it.downloaded / (it.total ?? 1)) * chunks);
    while (next < should) {
      const route = Math.floor(Math.random() * 3);
      done.add(next);
      handlers?.event(it.id, { type: "chunk_done", idx: next++, route });
    }
    handlers?.event(it.id, { type: "progress", downloaded: it.downloaded, total: it.total, bytes_per_sec: total, routes: rs });
    if (it.downloaded >= (it.total ?? 0)) {
      clearInterval(t);
      it.status = "done";
      it.path = `${it.dir}/${it.filename}`;
      handlers?.item({ ...it });
    }
  }, 250);
  timers.set(it.id, t);
  it.status = "downloading";
  handlers?.item({ ...it });
}

export const mock = {
  subscribe(h: Handlers) {
    handlers = h;
    setTimeout(() => items.filter((i) => i.status === "downloading").forEach(run), 50);
    return () => { handlers = null; };
  },
  async call(cmd: string, args: Record<string, unknown> = {}): Promise<unknown> {
    switch (cmd) {
      case "get_state": {
        if (!items.length) {
          seed("Lo-fi Mix.mp3", 120e6, "done", 120e6);
          seed("UI Inspiration Pack.zip", 850e6, "done", 850e6);
          seed("Project Files.zip", 2.1e9, "paused", 0.7e9);
          seed("Figma Setup.dmg", 520e6, "downloading", 160e6);
          seed("Beautiful Nature 4K.mp4", 2.4e9, "downloading", 1.2e9);
          const bad = seed("backup.zip", 400e6, "error", 0);
          bad.error = "HTTP 404 Not Found";
        }
        return { downloads: items, settings, api_port: 17653, api_ok: true } satisfies AppState;
      }
      case "get_links":
        return {
          links: [
            { name: "en0", label: "Wi-Fi", kind: "wi_fi", ipv4: "10.0.0.14", gateway: "10.0.0.1", is_default_route: false, link_speed_mbps: 866 },
            { name: "en5", label: "Ethernet", kind: "ethernet", ipv4: "192.168.89.56", gateway: "192.168.88.1", is_default_route: true, link_speed_mbps: 1000 },
            { name: "en7", label: "iPhone Hotspot", kind: "cellular", ipv4: "172.20.10.2", gateway: "172.20.10.1", is_default_route: false, link_speed_mbps: null },
          ],
          shared_gateways: [],
        } satisfies LinksResponse;
      case "add_download": {
        const dup = items.find((i) => i.url.endsWith("/" + String(args.url).split("/").pop()) && i.url === args.url);
        if (dup) return { id: dup.id, duplicate: true };
        const it = seed(String(args.url).split("/").pop() || "download", 600e6, "queued", 0);
        handlers?.item({ ...it });
        setTimeout(() => run(it), 300);
        return { id: it.id, duplicate: false };
      }
      case "move_download": { const it = items.find((i) => i.id === args.id); if (it) { it.priority += args.toFront ? 1 : -1; handlers?.item({ ...it }); } return; }
      case "pause_download": { const it = items.find((i) => i.id === args.id); if (it) { clearInterval(timers.get(it.id)); it.status = "paused"; handlers?.item({ ...it }); } return; }
      case "resume_download": { const it = items.find((i) => i.id === args.id); if (it) run(it); return; }
      case "remove_download": { const k = items.findIndex((i) => i.id === args.id); if (k >= 0) items.splice(k, 1); handlers?.removed(String(args.id)); return; }
      case "set_settings": Object.assign(settings, args.patch); return settings;
      case "get_autostart": return autostart;
      case "set_autostart": autostart = Boolean(args.enabled); return autostart;
      case "allow_pairing": return 60;
      case "pause_all": items.filter((i) => i.status === "downloading").forEach((i) => { clearInterval(timers.get(i.id)); i.status = "paused"; handlers?.item({ ...i }); }); return;
      case "resume_all": items.filter((i) => i.status === "paused").forEach(run); return;
      case "quit_app": case "show_main_window": return;
      case "run_spike": throw "Link test is only available in the desktop app";
      default: return;
    }
  },
};
