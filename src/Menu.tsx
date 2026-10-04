import { useEffect, useRef, useState } from "react";
import { Icon } from "./icons";

export type MenuEntry = { label: string; onClick: () => void; danger?: boolean } | "sep";

/** Round "···" button with a small macOS-style pop-up menu. */
export function MoreMenu({ items, label = "More", align = "right" }: { items: MenuEntry[]; label?: string; align?: "left" | "right" }) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => !box.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", away);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", away);
      window.removeEventListener("keydown", esc);
    };
  }, [open]);

  return (
    <div className="menu-wrap" ref={box}>
      <button className="round" aria-label={label} aria-haspopup="menu" aria-expanded={open} onClick={(e) => { e.stopPropagation(); setOpen(!open); }}>
        <Icon name="more" />
      </button>
      {open && (
        <div className={`menu ${align}`} role="menu">
          {items.map((it, i) =>
            it === "sep" ? (
              <hr key={i} />
            ) : (
              <button key={i} role="menuitem" className={it.danger ? "danger" : ""} onClick={(e) => { e.stopPropagation(); setOpen(false); it.onClick(); }}>
                {it.label}
              </button>
            ),
          )}
        </div>
      )}
    </div>
  );
}
