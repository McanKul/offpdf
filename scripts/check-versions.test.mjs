import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

const script = fileURLToPath(new URL("./check-versions.mjs", import.meta.url));
let root;

function run() {
  return spawnSync(process.execPath, [script], { cwd: root, encoding: "utf8" });
}

function updateJson(file, update) {
  const path = join(root, file);
  const value = JSON.parse(readFileSync(path, "utf8"));
  update(value);
  writeFileSync(path, JSON.stringify(value));
}

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "offpdf-versions-"));
  mkdirSync(join(root, "src-tauri"));
  writeFileSync(join(root, "package.json"), JSON.stringify({ name: "offpdf", version: "0.3.1" }));
  writeFileSync(join(root, "package-lock.json"), JSON.stringify({
    version: "0.3.1",
    packages: { "": { version: "0.3.1" }, "node_modules/example": { version: "9.0.0" } },
  }));
  writeFileSync(join(root, "src-tauri/tauri.conf.json"), JSON.stringify({ version: "0.3.1" }));
  writeFileSync(join(root, "src-tauri/Cargo.toml"), `
[package]
name = 'offpdf'
version = '0.3.1' # release version
[dependencies]
example = { version = "9.0.0" }
`);
  writeFileSync(join(root, "src-tauri/Cargo.lock"), `
version = 4
[[package]]
name = "example"
version = "9.0.0"
[[package]]
name = "offpdf"
version = "0.3.1"
`);
});

afterEach(() => {
  rmSync(root, { recursive: true, force: true });
});

describe("release version check", () => {
  it("accepts matching metadata without comparing dependency or lockfile format versions", () => {
    const result = run();
    expect(result.stderr).toBe("");
    expect(result.status).toBe(0);
    expect(result.stdout).toContain("0.3.1");
  });

  it.each([
    "package.json",
    "package-lock.json",
    "src-tauri/tauri.conf.json",
    "src-tauri/Cargo.toml",
    "src-tauri/Cargo.lock",
  ])("rejects a mismatched version in %s and reports files and values", (file) => {
    const path = join(root, file);
    writeFileSync(path, readFileSync(path, "utf8").replace("0.3.1", "0.3.2"));
    const result = run();
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("Release version metadata is inconsistent");
    expect(result.stderr).toContain(file);
    expect(result.stderr).toContain("0.3.2");
    expect(result.stderr).toContain("0.3.1");
  });

  it("checks the npm lockfile root package independently of its top-level version", () => {
    updateJson("package-lock.json", (lock) => { lock.packages[""].version = "0.3.2"; });
    const result = run();
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('package-lock.json packages[""].version: "0.3.2"');
  });

  it("rejects a missing version instead of treating it as a match", () => {
    updateJson("src-tauri/tauri.conf.json", (config) => { delete config.version; });
    const result = run();
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("src-tauri/tauri.conf.json version: <missing>");
  });

  it("requires an OffPDF package entry in the Cargo lockfile", () => {
    const path = join(root, "src-tauri/Cargo.lock");
    writeFileSync(path, readFileSync(path, "utf8").replace('name = "offpdf"', 'name = "other"'));
    const result = run();
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("src-tauri/Cargo.lock offpdf.version: <missing>");
  });

  it.each(["package.json", "src-tauri/Cargo.toml"])("reports the filename when %s cannot be parsed", (file) => {
    writeFileSync(join(root, file), "invalid {");
    const result = run();
    expect(result.status).toBe(1);
    expect(result.stderr).toContain(`Could not read ${file}:`);
  });
});
