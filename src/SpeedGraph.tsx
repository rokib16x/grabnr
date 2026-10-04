import { bytes, linkColor } from "./format";

const W = 300;
const H = 64;

export function SpeedGraph({ history, names }: { history: number[][]; names: string[] }) {
  const max = Math.max(1, ...history.flat());
  const points = (series: number[]) =>
    series.map((v, i) => `${((i / 59) * W).toFixed(1)},${(H - (v / max) * (H - 4) - 2).toFixed(1)}`).join(" ");
  return (
    <div>
      <svg viewBox={`0 0 ${W} ${H}`} className="graph" preserveAspectRatio="none" role="img" aria-label="Speed per link">
        {history.map((s, r) => (
          <polyline key={r} points={points(s.slice(-60))} fill="none" stroke={linkColor(names[r] ?? "")} strokeWidth="1.8" vectorEffect="non-scaling-stroke" />
        ))}
      </svg>
      <div className="graph-legend">
        <span>peak {bytes(max)}/s</span>
        {names.map((n) => (
          <span key={n}>
            <i style={{ background: linkColor(n) }} /> {n}
          </span>
        ))}
      </div>
    </div>
  );
}
