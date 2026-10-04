import { useSyncExternalStore } from "react";

// A small translation layer. The English text is the key, so the code reads naturally and a missing translation falls
// back to English. Text with a variable uses {name}; counts use tn("{n} item", "{n} items", n).

export type Lang = "en" | "es" | "fr" | "de" | "pt" | "bn";
export type Choice = Lang | "system";

/** Marks English text in a constant so it is translated where it is shown: `t(label)`. Returns the text unchanged. */
export const msg = (text: string) => text;

export const LANGUAGES: { id: Choice; label: string }[] = [
  { id: "system", label: msg("System default") },
  { id: "en", label: "English" },
  { id: "es", label: "Español" },
  { id: "fr", label: "Français" },
  { id: "de", label: "Deutsch" },
  { id: "pt", label: "Português" },
  { id: "bn", label: "বাংলা" },
];

type Dict = Record<string, string>;
const loaders: Record<Exclude<Lang, "en">, () => Promise<{ default: Dict }>> = {
  es: () => import("./locales/es"),
  fr: () => import("./locales/fr"),
  de: () => import("./locales/de"),
  pt: () => import("./locales/pt"),
  bn: () => import("./locales/bn"),
};

let current: Lang = "en";
let dict: Dict = {};

/** The language to use for a choice (`system` follows the browser or OS). */
export function resolve(choice: Choice, preferred: readonly string[] = typeof navigator === "undefined" ? [] : navigator.languages): Lang {
  if (choice !== "system") return choice;
  for (const p of preferred) {
    const base = p.toLowerCase().split("-")[0] as Lang;
    if (base === "en" || base in loaders) return base;
  }
  return "en";
}

export const lang = () => current;

/** Load a language and make it the current one. */
export async function setLanguage(choice: Choice): Promise<Lang> {
  const l = resolve(choice);
  dict = l === "en" ? {} : (await loaders[l]()).default;
  current = l;
  if (typeof document !== "undefined") document.documentElement.lang = l;
  return l;
}

/** Fill `{name}` placeholders. */
export function fill(text: string, vars?: Record<string, string | number>): string {
  return vars ? text.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k]) : m)) : text;
}

/** Translate `text` (English) into the current language. */
export function t(text: string, vars?: Record<string, string | number>): string {
  return fill(dict[text] ?? text, vars);
}

/** Pick the singular or plural English text by `n`, then translate it. */
export function tn(one: string, other: string, n: number, vars?: Record<string, string | number>): string {
  return t(n === 1 ? one : other, { n, ...vars });
}

/** Dates and numbers in the current language. */
export const locale = () => current;

// ---- the user's choice, remembered in the browser storage and shared between the app's windows ----

const KEY = "grabnr.lang";
let version = 0;
const subscribers = new Set<() => void>();
const bump = () => {
  version++;
  subscribers.forEach((f) => f());
};

export function savedChoice(): Choice {
  try {
    const v = localStorage.getItem(KEY) as Choice | null;
    return v && LANGUAGES.some((l) => l.id === v) ? v : "system";
  } catch {
    return "system";
  }
}

/** Change the language: remember it, load it and re-render everything that shows text. */
export async function chooseLanguage(choice: Choice): Promise<void> {
  try {
    localStorage.setItem(KEY, choice);
  } catch {
    /* storage may be unavailable; the choice then lasts until the window closes */
  }
  await setLanguage(choice);
  bump();
}

/** Load the remembered language at startup. */
export async function initLanguage(): Promise<void> {
  await setLanguage(savedChoice());
  bump();
}

/** Changes every time the language changes, so a component can use it as a `key` or dependency. */
export function useLanguageVersion(): number {
  return useSyncExternalStore(
    (cb) => {
      subscribers.add(cb);
      return () => subscribers.delete(cb);
    },
    () => version,
  );
}

// Another window of the app (the menu bar popover) changed the language.
if (typeof window !== "undefined") {
  window.addEventListener("storage", (e) => {
    if (e.key === KEY) void initLanguage();
  });
}
