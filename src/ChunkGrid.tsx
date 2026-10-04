import { useEffect, useRef } from "react";
import { routeColor } from "./format";

/** One cell per chunk, coloured by the link that fetched it. Very large files are bucketed. */
export function ChunkGrid({ chunks, version }: { chunks: Uint8Array; version: number }) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const c = ref.current;
    if (!c) return;
    const dpr = window.devicePixelRatio || 1;
    const width = c.clientWidth;
    const cell = 9;
    const cols = Math.max(1, Math.floor(width / cell));
    const buckets = Math.min(chunks.length, cols * 12);
    const per = chunks.length / Math.max(buckets, 1);
    const rows = Math.ceil(buckets / cols);
    c.width = width * dpr;
    c.height = rows * cell * dpr;
    c.style.height = `${rows * cell}px`;
    const g = c.getContext("2d")!;
    g.scale(dpr, dpr);
    const empty = getComputedStyle(c).getPropertyValue("--cell-empty") || "#8884";
    for (let b = 0; b < buckets; b++) {
      // A bucket shows the link that fetched most of its chunks, dimmed until complete.
      const from = Math.floor(b * per);
      const to = Math.max(from + 1, Math.floor((b + 1) * per));
      const counts = new Map<number, number>();
      let done = 0;
      for (let i = from; i < to; i++) {
        if (chunks[i]) {
          done++;
          counts.set(chunks[i], (counts.get(chunks[i]) ?? 0) + 1);
        }
      }
      let top = 0;
      let best = 0;
      counts.forEach((n, r) => n > best && ((best = n), (top = r)));
      g.globalAlpha = done === 0 ? 1 : done === to - from ? 1 : 0.55;
      g.fillStyle = done === 0 ? empty : routeColor(top - 1);
      g.fillRect((b % cols) * cell, Math.floor(b / cols) * cell, cell - 2, cell - 2);
    }
  }, [chunks, version]);

  return <canvas ref={ref} className="grid" aria-label="Chunk map" />;
}
