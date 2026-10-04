import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
// @ts-expect-error plain JS helper without types
import { keys } from "../scripts/i18n-keys.mjs";
import { fill, resolve, t, tn, setLanguage, LANGUAGES } from "./i18n";
import bn from "./locales/bn";
import de from "./locales/de";
import es from "./locales/es";
import fr from "./locales/fr";
import pt from "./locales/pt";

const dicts = { es, fr, de, pt, bn } as Record<string, Record<string, string>>;
const used: string[] = keys();
const placeholders = (s: string) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

describe("translations", () => {
  it("covers every text the UI asks for, in every language", () => {
    for (const [lang, d] of Object.entries(dicts)) {
      const missing = used.filter((k) => !(k in d));
      expect(missing, `${lang} is missing translations`).toEqual([]);
    }
  });

  it("has no stale entries", () => {
    for (const [lang, d] of Object.entries(dicts)) {
      const stale = Object.keys(d).filter((k) => !used.includes(k));
      expect(stale, `${lang} has translations nobody uses`).toEqual([]);
    }
  });

  it("keeps every {placeholder} of the English text", () => {
    for (const [lang, d] of Object.entries(dicts)) {
      for (const k of used) expect(placeholders(d[k]), `${lang}: ${k}`).toEqual(placeholders(k));
    }
  });

  it("does not leave a translation empty", () => {
    for (const [lang, d] of Object.entries(dicts)) for (const k of used) expect(d[k].trim(), `${lang}: ${k}`).not.toBe("");
  });

  it("has a language file for every language in the selector", () => {
    const ids = LANGUAGES.map((l) => l.id).filter((id) => id !== "system" && id !== "en");
    expect(ids.sort()).toEqual(Object.keys(dicts).sort());
    expect(readdirSync("src/locales").map((f: string) => f.replace(".ts", "")).sort()).toEqual(Object.keys(dicts).sort());
  });

  it("only uses translated texts through t, tn or msg (no unwrapped text in components)", () => {
    for (const f of readdirSync("src").filter((f: string) => f.endsWith(".tsx") && f !== "main.tsx")) {
      const text = readFileSync(`src/${f}`, "utf8");
      expect(/from "\.\/i18n"/.test(text) || !/>\s*[A-Z][a-z]+ [a-z]+/.test(text), `${f} shows text without importing i18n`).toBe(true);
    }
  });
});

describe("t, tn and resolve", () => {
  it("fills placeholders and falls back to English", async () => {
    await setLanguage("en");
    expect(t("Hello {name}, {n} left", { name: "Ana", n: 3 })).toBe("Hello Ana, 3 left");
    expect(fill("{x} stays", {})).toBe("{x} stays");
    expect(tn("{n} item", "{n} items", 1)).toBe("1 item");
    expect(tn("{n} item", "{n} items", 4)).toBe("4 items");
  });

  it("translates once a language is loaded, and leaves unknown texts alone", async () => {
    await setLanguage("de");
    expect(t("Cancel")).toBe("Abbrechen");
    expect(tn("{n} item", "{n} items", 2)).toBe("2 Elemente");
    expect(t("A text nobody translated")).toBe("A text nobody translated");
    await setLanguage("en");
  });

  it("follows the system language when asked", () => {
    expect(resolve("system", ["bn-BD", "en"])).toBe("bn");
    expect(resolve("system", ["pt-BR"])).toBe("pt");
    expect(resolve("system", ["ja-JP", "fr-CA"])).toBe("fr");
    expect(resolve("system", ["ja-JP"])).toBe("en");
    expect(resolve("es", ["de"])).toBe("es");
  });
});
