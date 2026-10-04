import { api } from "./api";
import type { ScheduleMode, ScheduleRule, Settings } from "./types";

const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const hhmm = (m: number) => `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
const minutes = (s: string) => {
  const [h, m] = s.split(":").map(Number);
  return Number.isFinite(h) && Number.isFinite(m) ? Math.min(1439, h * 60 + m) : 0;
};
const newId = () => (typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : String(Math.random()).slice(2));

const modeKind = (m: ScheduleMode) => m.kind;
const withKind = (kind: ScheduleMode["kind"], prev: ScheduleMode): ScheduleMode =>
  kind === "limit" ? { kind, kbps: prev.kind === "limit" ? prev.kbps : 2048 } : ({ kind } as ScheduleMode);

/** Rules like "weekdays 9-17: limit to 2 MB/s". The first matching rule wins; a window may cross midnight. */
export function ScheduleEditor({ settings, onChange }: { settings: Settings; onChange: (s: Settings) => void }) {
  const rules = settings.schedule;
  const save = async (next: ScheduleRule[]) => onChange(await api.setSettings({ schedule: next }));
  const patch = (id: string, p: Partial<ScheduleRule>) => save(rules.map((r) => (r.id === id ? { ...r, ...p } : r)));

  return (
    <div className="schedule">
      {rules.length === 0 && <p className="muted">No rules. Add one to pause downloads or limit the speed at certain times, for example during work hours.</p>}
      {rules.map((r) => (
        <div key={r.id} className={`rule${r.enabled ? "" : " off"}`}>
          <div className="rule-head">
            <input className="rule-name" value={r.name} placeholder="Name" aria-label="Rule name" onChange={(e) => patch(r.id, { name: e.target.value })} />
            <label className="link"><input type="checkbox" checked={r.enabled} onChange={(e) => patch(r.id, { enabled: e.target.checked })} aria-label="Rule enabled" /></label>
            <button className="ghost danger" onClick={() => save(rules.filter((x) => x.id !== r.id))}>Delete</button>
          </div>
          <div className="days" role="group" aria-label="Days">
            {DAYS.map((d, i) => (
              <button key={d} type="button" className={r.days[i] ? "day on" : "day"} aria-pressed={r.days[i]} onClick={() => patch(r.id, { days: r.days.map((v, j) => (j === i ? !v : v)) })}>{d}</button>
            ))}
          </div>
          <div className="rule-when">
            <label>From <input type="time" value={hhmm(r.start)} onChange={(e) => patch(r.id, { start: minutes(e.target.value) })} /></label>
            <label>To <input type="time" value={hhmm(r.end)} onChange={(e) => patch(r.id, { end: minutes(e.target.value) })} /></label>
            <select value={modeKind(r.mode)} onChange={(e) => patch(r.id, { mode: withKind(e.target.value as ScheduleMode["kind"], r.mode) })} aria-label="What to do">
              <option value="pause">Hold all downloads</option>
              <option value="limit">Limit the speed</option>
              <option value="full">Full speed</option>
            </select>
            {r.mode.kind === "limit" && (
              <label>
                <input type="number" min={0.1} step={0.5} value={r.mode.kbps / 1024} aria-label="Speed limit in MB/s" onChange={(e) => patch(r.id, { mode: { kind: "limit", kbps: Math.max(100, Math.round(+e.target.value * 1024)) } })} /> MB/s
              </label>
            )}
          </div>
          {r.start === r.end && <p className="muted small">Same start and end time means the whole day.</p>}
          {r.start > r.end && <p className="muted small">Runs overnight, into the next morning.</p>}
        </div>
      ))}
      <button onClick={() => save([...rules, { id: newId(), name: "Work hours", enabled: true, days: [true, true, true, true, true, false, false], start: 9 * 60, end: 17 * 60, mode: { kind: "limit", kbps: 2048 } }])}>Add rule</button>
    </div>
  );
}
