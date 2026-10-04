import { useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "./icons";

export type Command = { id: string; label: string; hint?: string; icon: string; run: () => void };

/** ⌘K: type to find an action or a download, Enter to run it. */
export function CommandPalette({ commands, onClose }: { commands: Command[]; onClose: () => void }) {
  const [q, setQ] = useState("");
  const [sel, setSel] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);

  const shown = useMemo(() => {
    const words = q.toLowerCase().split(/\s+/).filter(Boolean);
    return commands.filter((c) => words.every((w) => c.label.toLowerCase().includes(w) || c.hint?.toLowerCase().includes(w))).slice(0, 12);
  }, [q, commands]);
  useEffect(() => setSel(0), [q]);

  const run = (c?: Command) => {
    if (!c) return;
    onClose();
    c.run();
  };

  return (
    <div className="modal palette-wrap" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="sheet palette" role="dialog" aria-label="Command palette">
        <label className="search wide">
          <Icon name="search" size={15} />
          <input
            ref={input}
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Type a command or a download name"
            aria-label="Command"
            onKeyDown={(e) => {
              if (e.key === "Escape") onClose();
              else if (e.key === "ArrowDown") { e.preventDefault(); setSel((s) => Math.min(shown.length - 1, s + 1)); }
              else if (e.key === "ArrowUp") { e.preventDefault(); setSel((s) => Math.max(0, s - 1)); }
              else if (e.key === "Enter") run(shown[sel]);
            }}
          />
        </label>
        <ul role="listbox">
          {shown.map((c, i) => (
            <li key={c.id} role="option" aria-selected={i === sel} className={i === sel ? "on" : ""} onMouseEnter={() => setSel(i)} onClick={() => run(c)}>
              <Icon name={c.icon} size={15} /><span>{c.label}</span>{c.hint && <small>{c.hint}</small>}
            </li>
          ))}
          {shown.length === 0 && <li className="muted">Nothing matches.</li>}
        </ul>
      </div>
    </div>
  );
}
