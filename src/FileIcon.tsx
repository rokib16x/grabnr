import { fileKind } from "./format";

export function FileIcon({ name, size = 44 }: { name: string; size?: number }) {
  const k = fileKind(name);
  return (
    <span className="file-icon" style={{ width: size, height: size, background: `linear-gradient(160deg, ${k.from}, ${k.to})`, fontSize: Math.max(9, size * 0.23) }} aria-hidden="true">
      {k.label}
    </span>
  );
}
