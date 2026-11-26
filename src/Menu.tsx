import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Icon } from "./icons";
import { msg, t } from "./i18n";

export type MenuEntry = { label: string; onClick: () => void; danger?: boolean } | "sep";

function Entries({ items, onPick }: { items: MenuEntry[]; onPick: () => void }) {
  return (
    <>
      {items.map((it, i) =>
        it === "sep" ? (
          <hr key={i} />
        ) : (
          <button key={i} role="menuitem" className={it.danger ? "danger" : ""} onClick={(e) => { e.stopPropagation(); onPick(); it.onClick(); }}>
            {it.label}
          </button>
        ),
      )}
    </>
  );
}

/** Pop-up menu at the pointer, for a right-click. */
export function ContextMenu({ x, y, items, onClose }: { x: number; y: number; items: MenuEntry[]; onClose: () => void }) {
  const box = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });
  useLayoutEffect(() => {
    const r = box.current?.getBoundingClientRect();
    if (r) setPos({ x: Math.max(8, Math.min(x, window.innerWidth - r.width - 8)), y: Math.max(8, Math.min(y, window.innerHeight - r.height - 8)) });
  }, [x, y]);
  useEffect(() => {
    const away = (e: MouseEvent) => !box.current?.contains(e.target as Node) && onClose();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", away);
    window.addEventListener("keydown", esc);
    window.addEventListener("blur", onClose);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("mousedown", away);
      window.removeEventListener("keydown", esc);
      window.removeEventListener("blur", onClose);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);
  return (
    <div ref={box} className="menu ctx" role="menu" style={{ left: pos.x, top: pos.y }} onContextMenu={(e) => e.preventDefault()}>
      <Entries items={items} onPick={onClose} />
    </div>
  );
}

/** Round "···" button with a small macOS-style pop-up menu. */
export function MoreMenu({ items, label = msg("More"), align = "right" }: { items: MenuEntry[]; label?: string; align?: "left" | "right" }) {
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
      <button className="round" aria-label={t(label)} aria-haspopup="menu" aria-expanded={open} onClick={(e) => { e.stopPropagation(); setOpen(!open); }}>
        <Icon name="more" />
      </button>
      {open && (
        <div className={`menu ${align}`} role="menu">
          <Entries items={items} onPick={() => setOpen(false)} />
        </div>
      )}
    </div>
  );
}
