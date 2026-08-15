import { describe, expect, it } from "vitest";
import {
  basename,
  dirname,
  fileSizeTier,
  formatBytes,
  formatCount,
  formatRelativeTime,
  LARGE_FILE_BYTES,
  stripExt,
  VERY_LARGE_FILE_BYTES,
} from "./formatBytes";

describe("formatBytes", () => {
  it("formats zero and plain bytes without decimals", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1)).toBe("1 B");
    expect(formatBytes(999)).toBe("999 B");
    expect(formatBytes(1023)).toBe("1023 B");
  });

  it("uses base-1024 unit boundaries", () => {
    expect(formatBytes(1024)).toBe("1.00 KB");
    expect(formatBytes(1024 ** 2)).toBe("1.00 MB");
    expect(formatBytes(1024 ** 3)).toBe("1.00 GB");
    expect(formatBytes(1024 ** 4)).toBe("1.00 TB");
  });

  it("respects a custom decimal count for non-byte units", () => {
    expect(formatBytes(1536, 1)).toBe("1.5 KB");
    expect(formatBytes(1536, 0)).toBe("2 KB");
    expect(formatBytes(2.5 * 1024 ** 2, 3)).toBe("2.500 MB");
  });

  it("returns '—' for negative and non-finite values", () => {
    expect(formatBytes(-1)).toBe("—");
    expect(formatBytes(Number.NaN)).toBe("—");
    expect(formatBytes(Number.POSITIVE_INFINITY)).toBe("—");
    expect(formatBytes(Number.NEGATIVE_INFINITY)).toBe("—");
  });
});

describe("fileSizeTier", () => {
  it("treats values immediately below 500 MiB as normal", () => {
    expect(fileSizeTier(LARGE_FILE_BYTES - 1)).toBe("normal");
  });

  it("treats values exactly at 500 MiB as large", () => {
    expect(fileSizeTier(LARGE_FILE_BYTES)).toBe("large");
  });

  it("treats values immediately below 2 GiB as large", () => {
    expect(fileSizeTier(VERY_LARGE_FILE_BYTES - 1)).toBe("large");
  });

  it("treats values exactly at 2 GiB as very large", () => {
    expect(fileSizeTier(VERY_LARGE_FILE_BYTES)).toBe("veryLarge");
  });
});

describe("formatCount", () => {
  it("formats counts with en-US thousands separators", () => {
    expect(formatCount(0)).toBe("0");
    expect(formatCount(999)).toBe("999");
    expect(formatCount(1000)).toBe("1,000");
    expect(formatCount(1234567)).toBe("1,234,567");
  });
});

describe("formatRelativeTime", () => {
  // Keep assertions under 30 days because older dates use toLocaleDateString(),
  // which can vary by environment and locale.
  const now = Date.UTC(2026, 7, 13, 12, 0, 0);

  it("returns 'just now' for times less than 45 seconds ago", () => {
    expect(formatRelativeTime(now, now)).toBe("just now");
    expect(formatRelativeTime(now - 44_000, now)).toBe("just now");
  });

  it("returns 'X min ago' for times less than 60 minutes ago", () => {
    expect(formatRelativeTime(now - 60_000, now)).toBe("1 min ago");
    expect(formatRelativeTime(now - 5 * 60_000, now)).toBe("5 min ago");
    expect(formatRelativeTime(now - 59 * 60_000, now)).toBe("59 min ago");
  });

  it("returns 'X hour(s) ago' for times less than 24 hours ago", () => {
    expect(formatRelativeTime(now - 60 * 60_000, now)).toBe("1 hour ago");
    expect(formatRelativeTime(now - 5 * 60 * 60_000, now)).toBe("5 hours ago");
    expect(formatRelativeTime(now - 23 * 60 * 60_000, now)).toBe("23 hours ago");
  });

  it("returns 'X day(s) ago' for times less than 30 days ago", () => {
    expect(formatRelativeTime(now - 24 * 60 * 60_000, now)).toBe("1 day ago");
    expect(formatRelativeTime(now - 5 * 24 * 60 * 60_000, now)).toBe("5 days ago");
    expect(formatRelativeTime(now - 29 * 24 * 60 * 60_000, now)).toBe("29 days ago");
  });

  it("treats future timestamps as just now", () => {
    expect(formatRelativeTime(now + 60_000, now)).toBe("just now");
  });
});

describe("basename", () => {
  it("returns the last part of a path with POSIX separators", () => {
    expect(basename("foo/bar/baz.pdf")).toBe("baz.pdf");
    expect(basename("/foo/bar/baz.pdf")).toBe("baz.pdf");
  });

  it("returns the last part of a path with Windows separators", () => {
    expect(basename("foo\\bar\\baz.pdf")).toBe("baz.pdf");
    expect(basename("\\foo\\bar\\baz.pdf")).toBe("baz.pdf");
  });
});

describe("dirname", () => {
  it("returns the directory part of a path with POSIX separators", () => {
    expect(dirname("foo/bar/baz.pdf")).toBe("foo/bar");
    expect(dirname("/foo/bar/baz.pdf")).toBe("/foo/bar");
  });

  it("returns the directory part of a path with Windows separators", () => {
    expect(dirname("foo\\bar\\baz.pdf")).toBe("foo\\bar");
    expect(dirname("\\foo\\bar\\baz.pdf")).toBe("\\foo\\bar");
  });

  it("returns an empty string for a path with no directory", () => {
    expect(dirname("baz.pdf")).toBe("");
  });
});

describe("stripExt", () => {
  it("strips the extension from a normal filename", () => {
    expect(stripExt("report.pdf")).toBe("report");
  });

  it("strips only the final extension from a filename with multiple dots", () => {
    expect(stripExt("report.final.v2.pdf")).toBe("report.final.v2");
  });

  it("preserves a dotfile", () => {
    expect(stripExt(".env")).toBe(".env");
  });

  it("preserves a filename without an extension", () => {
    expect(stripExt("README")).toBe("README");
  });
});
