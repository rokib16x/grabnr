const P: Record<string, string> = {
  plus: "M12 5v14M5 12h14",
  pause: "M8 5h3v14H8zM13 5h3v14h-3z",
  play: "M8 5.5v13l11-6.5z",
  trash: "M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 002 2h6a2 2 0 002-2l1-12M9 7V4h6v3",
  more: "M5 12h.01M12 12h.01M19 12h.01",
  search: "M11 5a6 6 0 100 12 6 6 0 000-12zM20 20l-4.2-4.2",
  link: "M10 14a4 4 0 005.7 0l3-3a4 4 0 00-5.7-5.7l-1 1M14 10a4 4 0 00-5.7 0l-3 3a4 4 0 005.7 5.7l1-1",
  wifi: "M2.5 9a14 14 0 0119 0M6 12.5a9 9 0 0112 0M9.2 15.8a4.4 4.4 0 015.6 0M12 19.2h.01",
  ethernet: "M4 5h16v10H4zM8 9v3M12 9v3M16 9v3M12 15v4M8 19h8",
  phone: "M8 3h8a1 1 0 011 1v16a1 1 0 01-1 1H8a1 1 0 01-1-1V4a1 1 0 011-1zM11 18h2",
  usb: "M12 3v15M12 18a2 2 0 100 4 2 2 0 000-4zM12 10l-4-2v3M12 13l4-2.5",
  sliders: "M4 7h9M17 7h3M4 17h3M11 17h9M15 5v4M9 15v4",
  check: "M5 12.5l4.5 4.5L19 7.5",
  folder: "M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z",
  download: "M12 4v11M7.5 10.5L12 15l4.5-4.5M5 19h14",
  inbox: "M4 13l2-8h12l2 8M4 13v5a1 1 0 001 1h14a1 1 0 001-1v-5M4 13h4l1 2h6l1-2h4",
  clock: "M12 4a8 8 0 100 16 8 8 0 000-16zM12 8v4.5l3 1.8",
  bolt: "M13 3L5 13.5h6L10 21l8-10.5h-6z",
  alert: "M12 4a8 8 0 100 16 8 8 0 000-16zM12 8v5M12 16.5v.01",
  x: "M6 6l12 12M18 6L6 18",
  power: "M12 3v8M7 6.8a7 7 0 1010 0",
  copy: "M9 9h10v10H9zM5 15V5h10",
  chevron: "M9 6l6 6-6 6",
};
const FILLED = new Set(["play", "pause"]);

export function Icon({ name, size = 16, className }: { name: keyof typeof P | string; size?: number; className?: string }) {
  const filled = FILLED.has(name);
  return (
    <svg className={className} width={size} height={size} viewBox="0 0 24 24" fill={filled ? "currentColor" : "none"} stroke={filled ? "none" : "currentColor"} strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d={P[name] ?? P.link} />
    </svg>
  );
}

export const linkIcon = (kind: string) => (kind === "wi_fi" ? "wifi" : kind === "ethernet" ? "ethernet" : kind === "cellular" ? "phone" : "usb");
