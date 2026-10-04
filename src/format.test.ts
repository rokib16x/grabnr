import { describe, expect, it } from "vitest";
import { bytes, categoryOf, duration, eta, fileKind, fileName, linkColor, rate, registerLinks } from "./format";

describe("sizes and times", () => {
  it("formats bytes and rates", () => {
    expect(bytes(0)).toBe("0 B");
    expect(bytes(1536)).toBe("1.5 KB");
    expect(bytes(5 * 1024 ** 3)).toBe("5.0 GB");
    expect(rate(2 * 1024 * 1024)).toBe("2.0 MB/s");
  });

  it("formats time left and durations", () => {
    expect(eta(1000, 0)).toBe("");
    expect(eta(1000, 100)).toBe("10s");
    expect(eta(100 * 90, 100)).toBe("1m 30s");
    expect(duration(3700)).toBe("1h 1m");
    expect(duration(45)).toBe("45s");
  });
});

describe("file names and types", () => {
  it("prefers the file name, else the last path segment", () => {
    expect(fileName("https://x.example/a/b/c%20d.zip?token=1", null)).toBe("c d.zip");
    expect(fileName("https://x.example/a.zip", "named.zip")).toBe("named.zip");
    expect(fileName("not a url", null)).toBe("not a url");
  });

  it("sorts files into categories", () => {
    expect(categoryOf("movie.MP4")).toBe("video");
    expect(categoryOf("song.flac")).toBe("audio");
    expect(categoryOf("a.tar.gz")).toBe("archive");
    expect(categoryOf("ubuntu.iso")).toBe("disk");
    expect(categoryOf("paper.pdf")).toBe("document");
    expect(categoryOf("README")).toBe("other");
  });

  it("labels tiles by extension", () => {
    expect(fileKind("a.zip").label).toBe("ZIP");
    expect(fileKind("noextension").label).toBe("FILE");
    expect(fileKind("archive.verylongext").label).toBe("FILE");
  });
});

describe("link colours", () => {
  it("keeps one colour per link and avoids repeats for the same kind", () => {
    registerLinks([
      { name: "en0", kind: "wi_fi" },
      { name: "en1", kind: "wi_fi" },
      { name: "en5", kind: "ethernet" },
    ]);
    expect(linkColor("en0")).toBe(linkColor("en0"));
    expect(linkColor("en0")).not.toBe(linkColor("en1"));
    expect(linkColor("en5")).not.toBe(linkColor("en0"));
  });
});
