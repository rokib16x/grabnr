import { useEffect, useState } from "react";
import { initLanguage, useLanguageVersion } from "./i18n";
import { MainWindow } from "./MainWindow";
import { TrayPopover } from "./TrayPopover";
import "./App.css";
import { isMac as mac } from "./platform";

// The same bundle serves the main window and the menu bar popover (`?view=tray`).
const view = new URLSearchParams(window.location.search).get("view");
const native = "__TAURI_INTERNALS__" in window;

export default function App() {
  const [ready, setReady] = useState(false);
  // Changing the language rebuilds the window, so every text is looked up again.
  const language = useLanguageVersion();
  useEffect(() => {
    const r = document.documentElement;
    // Only the native macOS windows have a vibrancy material behind the page; elsewhere use solid colours.
    r.dataset.vibrancy = native && mac ? "on" : "off";
    r.dataset.mac = mac ? "on" : "off";
    r.dataset.view = view === "tray" ? "tray" : "main";
    void initLanguage().then(() => setReady(true));
  }, []);
  if (!ready) return null;
  return view === "tray" ? <TrayPopover key={language} /> : <MainWindow key={language} />;
}
