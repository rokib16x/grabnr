import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { mock } from "./mock";
import type { AppState, EngineEvent, Item, LinksResponse, Settings, SpikeReport } from "./types";

const native = "__TAURI_INTERNALS__" in window;
const call = <T,>(cmd: string, args?: Record<string, unknown>) => (native ? invoke<T>(cmd, args) : (mock.call(cmd, args) as Promise<T>));

export const api = {
  state: () => call<AppState>("get_state"),
  links: () => call<LinksResponse>("get_links"),
  add: (url: string, filename?: string, dir?: string) => call<string>("add_download", { url, filename: filename || null, dir: dir || null }),
  pauseAll: () => call<void>("pause_all"),
  resumeAll: () => call<void>("resume_all"),
  quit: () => call<void>("quit_app"),
  showMain: (settings = false) => call<void>("show_main_window", { settings }),
  pause: (id: string) => call<void>("pause_download", { id }),
  resume: (id: string) => call<void>("resume_download", { id }),
  remove: (id: string, deleteFiles: boolean) => call<void>("remove_download", { id, deleteFiles }),
  reveal: (id: string) => call<void>("reveal_download", { id }),
  open: (id: string) => call<void>("open_download", { id }),
  setSettings: (patch: Partial<Settings>) => call<Settings>("set_settings", { patch }),
  autostart: () => call<boolean>("get_autostart"),
  setAutostart: (enabled: boolean) => call<boolean>("set_autostart", { enabled }),
  allowPairing: () => call<number>("allow_pairing"),
  spike: (only: string[], secs: number) => call<SpikeReport>("run_spike", { only, secs }),
  pickFolder: async () => (native ? ((await open({ directory: true })) as string | null) : "/Users/you/Documents"),
};

type Handlers = { item: (i: Item) => void; removed: (id: string) => void; event: (id: string, e: EngineEvent) => void };

/** Called when the menu bar popover asks the main window to open Preferences. */
export function onOpenSettings(cb: () => void): () => void {
  if (!native) return () => {};
  const p = listen("open-settings", cb);
  return () => void p.then((un) => un());
}

/** Subscribe to backend events; returns an unsubscribe function. */
export function subscribe(h: Handlers): () => void {
  if (!native) return mock.subscribe(h);
  const subs = [
    listen<Item>("item-updated", (e) => h.item(e.payload)),
    listen<string>("item-removed", (e) => h.removed(e.payload)),
    listen<{ id: string; event: EngineEvent }>("download-event", (e) => h.event(e.payload.id, e.payload.event)),
  ];
  return () => subs.forEach((p) => p.then((un) => un()));
}
