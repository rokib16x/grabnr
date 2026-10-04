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

/** One stable colour per link index, shared by the share bar, chunk grid and graph. */
export const ROUTE_COLORS = ["#3b82f6", "#f59e0b", "#10b981", "#a855f7", "#ef4444", "#14b8a6"];
export const routeColor = (i: number) => ROUTE_COLORS[i % ROUTE_COLORS.length];
