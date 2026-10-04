import { useEffect } from "react";
import { MainWindow } from "./MainWindow";
import { TrayPopover } from "./TrayPopover";
import "./App.css";

// The same bundle serves the main window and the menu bar popover (`?view=tray`).
const view = new URLSearchParams(window.location.search).get("view");
const native = "__TAURI_INTERNALS__" in window;
const mac = /Mac/.test(navigator.platform);

export default function App() {
  useEffect(() => {
    const r = document.documentElement;
    // Only the native macOS windows have a vibrancy material behind the page; elsewhere use solid colours.
    r.dataset.vibrancy = native && mac ? "on" : "off";
    r.dataset.mac = mac ? "on" : "off";
    r.dataset.view = view === "tray" ? "tray" : "main";
  }, []);
  return view === "tray" ? <TrayPopover /> : <MainWindow />;
}
