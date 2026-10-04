// Words and keys that differ between operating systems.
export const isMac = /Mac/.test(navigator.platform);
export const isWindows = /Win/.test(navigator.platform);

/** The shortcut modifier as shown to the user. */
export const MOD = isMac ? "⌘" : "Ctrl+";
export const FILE_MANAGER = isMac ? "Finder" : isWindows ? "File Explorer" : "the file manager";
export const TRAY = isMac ? "menu bar" : isWindows ? "system tray" : "system tray";
