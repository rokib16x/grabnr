export type Status = "queued" | "downloading" | "paused" | "done" | "error";

export type Item = {
  id: string;
  url: string;
  filename: string | null;
  dir: string;
  status: Status;
  total: number | null;
  downloaded: number;
  path: string | null;
  error: string | null;
  added: number;
  checksum: string | null;
  priority: number;
  retries: number;
};

export type LinkRule = { share_pct: number; limit_kbps: number };

export type AddOptions = { filename?: string; dir?: string; checksum?: string; username?: string; password?: string; token?: string; proxy?: string; mirrors?: string[] };

export type BatchResult = { added: number; duplicates: number; skipped: number };
export type PageLink = { url: string; text: string };

export type AddResult = { id: string; duplicate: boolean };

export type RouteStat = { name: string; bytes: number; bytes_per_sec: number; connections: number; down: boolean };

export type EngineEvent =
  | { type: "started"; filename: string; total: number | null; chunks: number; ranges: boolean; resumed_chunks: number }
  | { type: "resume_discarded"; reason: string }
  | { type: "resume_checked"; checked: number; redo: number }
  | { type: "progress"; downloaded: number; total: number | null; bytes_per_sec: number; routes: RouteStat[] }
  | { type: "chunk_done"; idx: number; route: number }
  | { type: "route_down"; route: number; reason: string }
  | { type: "route_up"; route: number; name: string }
  | { type: "finished"; path: string };

export type Settings = {
  dest_dir: string;
  enabled_links: string[] | null;
  conns_per_route: number;
  max_active: number;
  token: string;
  speed_limit_kbps: number;
  auto_retry: number;
  link_rules: Record<string, LinkRule>;
  skip_cellular: boolean;
  proxy: string;
  onboarded: boolean;
  sound: boolean;
  schedule: ScheduleRule[];
  after_command: string;
  webhook_url: string;
  quarantine: boolean;
};

export type ScheduleMode = { kind: "pause" } | { kind: "limit"; kbps: number } | { kind: "full" };
export type ScheduleRule = { id: string; name: string; enabled: boolean; days: boolean[]; start: number; end: number; mode: ScheduleMode };
export type ScheduleEffect = { hold: boolean; limit_kbps: number; rule: string | null };
export type AfterAll = "none" | "sleep" | "quit" | "command";

export type AppState = { downloads: Item[]; settings: Settings; api_port: number; api_ok: boolean; schedule: ScheduleEffect; after_all: AfterAll };

export type Link = {
  name: string;
  label: string;
  kind: string;
  ipv4: string;
  gateway: string | null;
  is_default_route: boolean;
  link_speed_mbps: number | null;
};
export type LinksResponse = { links: Link[]; shared_gateways: [string, string][] };

export type ModeResult = { mode: string; public_ip: string | null; verdict: string; error: string | null };
export type Throughput = { link: string; mbps: number; error: string | null };
export type SpikeReport = {
  baseline_ip: string | null;
  links: { link: string; modes: ModeResult[]; solo: Throughput }[];
  combined_mbps: number;
  best_solo_mbps: number;
};

/** Live, UI-only data kept per download. */
export type Live = {
  downloaded: number;
  v: number; // bumped when `chunks` is mutated in place
  routes: RouteStat[];
  speed: number;
  chunks: Uint8Array; // 0 = pending, route+1 = fetched by that link
  resumedChunks: number;
  history: number[][]; // per route, bytes/s samples
  notice: string | null;
};

export type HistoryRec = {
  id: string;
  name: string;
  url: string;
  bytes: number;
  finished: number;
  active_secs: number;
  link_bytes: Record<string, number>;
  saved_secs: number;
};
