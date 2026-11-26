export type Theme = "system" | "light" | "dark";
export const THEMES: Theme[] = ["system", "light", "dark"];

export function savedTheme(): Theme {
  try {
    const v = localStorage.getItem("grabnr.theme") as Theme | null;
    return v && THEMES.includes(v) ? v : "system";
  } catch {
    return "system";
  }
}

/** Light or dark on purpose, or follow the system. Forced themes drop the macOS vibrancy, which always follows the system. */
export function applyTheme(theme: Theme, native = false, mac = false) {
  const r = document.documentElement;
  if (theme === "system") delete r.dataset.theme;
  else r.dataset.theme = theme;
  r.dataset.vibrancy = native && mac && theme === "system" ? "on" : "off";
}

export function chooseTheme(theme: Theme) {
  try { localStorage.setItem("grabnr.theme", theme); } catch { /* storage may be unavailable */ }
  applyTheme(theme, "__TAURI_INTERNALS__" in window, /Mac/.test(navigator.platform));
}
