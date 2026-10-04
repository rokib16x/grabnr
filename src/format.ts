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

const KIND_COLORS: Record<string, string> = { wi_fi: "#34c759", ethernet: "#0a84ff", cellular: "#af52de", other: "#ff9f0a", tunnel: "#5ac8fa" };
const SPARE = ["#ff375f", "#64d2ff", "#ffd60a", "#bf5af2", "#30d158", "#ff6b57"];
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
  if (/^magnet:/i.test(url)) return new URLSearchParams(url.slice(url.indexOf("?") + 1)).get("dn") ?? "Torrent";
  try {
    return decodeURIComponent(new URL(url).pathname.split("/").filter(Boolean).pop() ?? url);
  } catch {
    return url;
  }
};

const KINDS: [RegExp, string, string][] = [
  [/^(zip|rar|7z|tar|gz|tgz|bz2|xz|zst)$/, "#ff9f0a", "#e08600"],
  [/^(dmg|iso|img|pkg)$/, "#8e8e93", "#636366"],
  [/^(mp4|mkv|mov|avi|webm|m4v)$/, "#0a84ff", "#0060d0"],
  [/^(mp3|m4a|flac|wav|aac|ogg)$/, "#ff375f", "#d70f3c"],
  [/^(pdf)$/, "#ff453a", "#d62a20"],
  [/^(png|jpe?g|gif|webp|heic|svg)$/, "#30b0c7", "#1a8fa3"],
  [/^(exe|msi|deb|rpm|apk|appimage|app)$/, "#5e5ce6", "#4240b8"],
];
/** Tile colours and label for a file, picked from its extension. */
export function fileKind(name: string) {
  const ext = (name.split(".").pop() ?? "").toLowerCase();
  const hit = KINDS.find(([re]) => re.test(ext));
  return { from: hit?.[1] ?? "#64a8ff", to: hit?.[2] ?? "#2f7cf6", label: ext.length > 0 && ext.length <= 4 && ext !== name.toLowerCase() ? ext.toUpperCase() : "FILE" };
}

export type Category = "video" | "audio" | "image" | "document" | "archive" | "disk" | "app" | "other";
export const CATEGORIES: { id: Category; label: string; icon: string }[] = [
  { id: "video", label: "Video", icon: "play" },
  { id: "audio", label: "Audio", icon: "bolt" },
  { id: "image", label: "Images", icon: "search" },
  { id: "document", label: "Documents", icon: "folder" },
  { id: "archive", label: "Archives", icon: "inbox" },
  { id: "disk", label: "Disk images", icon: "download" },
  { id: "app", label: "Apps", icon: "sliders" },
  { id: "other", label: "Other", icon: "link" },
];

const CAT_EXT: [Category, RegExp][] = [
  ["video", /^(mp4|mkv|mov|avi|webm|m4v|wmv|flv)$/],
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
