import { categoryOf, fileKind, type Category } from "./format";
import { Icon } from "./icons";

const CAT_ICON: Record<Category, string> = { video: "video", audio: "audio", image: "image", document: "document", archive: "archive", disk: "disk", app: "app", other: "file" };

/** File tile: a bold app-icon-style square in the type's colour, a white category glyph and the extension under it. */
export function FileIcon({ name, size = 44 }: { name: string; size?: number }) {
  const k = fileKind(name);
  const withLabel = size >= 34 && k.label !== "FILE";
  const style = { width: size, height: size, "--c": k.from, "--c2": k.to } as React.CSSProperties;
  return (
    <span className={`file-icon${withLabel ? " labelled" : ""}`} style={style} aria-hidden="true">
      <Icon name={CAT_ICON[categoryOf(name)]} size={Math.round(size * (withLabel ? 0.4 : 0.52))} weight="fill" />
      {withLabel && <b style={{ fontSize: Math.max(8, Math.round(size * 0.18)) }}>{k.label}</b>}
    </span>
  );
}
