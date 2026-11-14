import { msg, t } from "./i18n";
export function bytes(n: number): string {
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < u.length - 1) {
    n /= 1024;
    i++;
  }
  return `${n >= 100 || i === 0 ? n.toFixed(0) : n.toFixed(1)} ${u[i]}`;
}

export function rate(n: number): string {
  return `${bytes(n)}/s`;
}

export function eta(remaining: number, bps: number): string {
  if (bps < 1) return "";
  const s = Math.round(remaining / bps);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}

const KIND_COLORS: Record<string, string> = { wi_fi: "#10b981", ethernet: "#2563eb", cellular: "#a855f7", other: "#f59e0b", tunnel: "#06b6d4" };
const SPARE = ["#ec4899", "#14b8a6", "#eab308", "#8b5cf6", "#22c55e", "#f97316"];
const registry = new Map<string, string>();

/** Give each network link one colour, by kind, used by the sidebar, share bars, chunk map and graph. */
export function registerLinks(links: { name: string; kind: string }[]) {
  const used = new Set<string>();
  links.forEach((l) => {
    let c = KIND_COLORS[l.kind] ?? KIND_COLORS.other;
    if (used.has(c)) c = SPARE.find((s) => !used.has(s)) ?? c;
    used.add(c);
    registry.set(l.name, c);
  });
}

export const linkColor = (name: string) => {
  if (!registry.has(name)) registry.set(name, SPARE[registry.size % SPARE.length]);
  return registry.get(name)!;
};

export const fileName = (url: string, filename: string | null) => {
  if (filename) return filename;
  // A magnet link names the torrent in its `dn` parameter.
  if (/^magnet:/i.test(url)) return new URLSearchParams(url.slice(url.indexOf("?") + 1)).get("dn") ?? t("Torrent");
  try {
    return decodeURIComponent(new URL(url).pathname.split("/").filter(Boolean).pop() ?? url);
  } catch {
    return url;
  }
};

const KINDS: [RegExp, string, string][] = [
  [/^(zip|rar|7z|tar|gz|tgz|bz2|xz|zst)$/, "#f59e0b", "#b45309"],
  [/^(dmg|iso|img|pkg)$/, "#64748b", "#334155"],
  [/^(mp4|mkv|mov|avi|webm|m4v|ts)$/, "#3b82f6", "#1d4ed8"],
  [/^(mp3|m4a|flac|wav|aac|ogg)$/, "#ec4899", "#be185d"],
  [/^(pdf)$/, "#f97316", "#c2410c"],
  [/^(docx?|txt|md|rtf|epub)$/, "#0ea5e9", "#0369a1"],
  [/^(png|jpe?g|gif|webp|heic|svg)$/, "#a855f7", "#7e22ce"],
  [/^(exe|msi|deb|rpm|apk|appimage|app)$/, "#6366f1", "#4338ca"],
];
/** Tile colours and label for a file, picked from its extension. */
export function fileKind(name: string) {
  const ext = (name.split(".").pop() ?? "").toLowerCase();
  const hit = KINDS.find(([re]) => re.test(ext));
  return { from: hit?.[1] ?? "#94a3b8", to: hit?.[2] ?? "#475569", label: ext.length > 0 && ext.length <= 4 && ext !== name.toLowerCase() ? ext.toUpperCase() : "FILE" };
}

export type Category = "video" | "audio" | "image" | "document" | "archive" | "disk" | "app" | "other";
export const CATEGORIES: { id: Category; label: string; icon: string }[] = [
  { id: "video", label: msg("Video"), icon: "video" },
  { id: "audio", label: msg("Audio"), icon: "audio" },
  { id: "image", label: msg("Images"), icon: "image" },
  { id: "document", label: msg("Documents"), icon: "document" },
  { id: "archive", label: msg("Archives"), icon: "archive" },
  { id: "disk", label: msg("Disk images"), icon: "disk" },
  { id: "app", label: msg("Apps"), icon: "app" },
  { id: "other", label: msg("Other"), icon: "file" },
];

const CAT_EXT: [Category, RegExp][] = [
  ["video", /^(mp4|mkv|mov|avi|webm|m4v|wmv|flv|ts)$/],
  ["audio", /^(mp3|m4a|flac|wav|aac|ogg|opus)$/],
  ["image", /^(png|jpe?g|gif|webp|heic|svg|tiff?|bmp)$/],
  ["document", /^(pdf|docx?|xlsx?|pptx?|txt|md|epub|csv|rtf)$/],
  ["archive", /^(zip|rar|7z|tar|gz|tgz|bz2|xz|zst)$/],
  ["disk", /^(dmg|iso|img|pkg)$/],
  ["app", /^(exe|msi|deb|rpm|apk|appimage|app)$/],
];
export function categoryOf(name: string): Category {
  const ext = (name.split(".").pop() ?? "").toLowerCase();
  return CAT_EXT.find(([, re]) => re.test(ext))?.[0] ?? "other";
}

export function duration(secs: number): string {
  const s = Math.round(secs);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}
