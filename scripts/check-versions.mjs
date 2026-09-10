import { readFileSync } from "node:fs";
import { parse as parseToml } from "smol-toml";

function read(file, parse = JSON.parse) {
  try {
    return parse(readFileSync(file, "utf8"));
  } catch (error) {
    throw new Error(`Could not read ${file}: ${error.message}`);
  }
}

try {
  const pkg = read("package.json");
  const npmLock = read("package-lock.json");
  const cargo = read("src-tauri/Cargo.toml", parseToml);
  const cargoLock = read("src-tauri/Cargo.lock", parseToml);
  const tauri = read("src-tauri/tauri.conf.json");
  const versions = [
    ["package.json version", pkg?.version],
    ["package-lock.json version", npmLock?.version],
    ['package-lock.json packages[""].version', npmLock?.packages?.[""]?.version],
    ["src-tauri/Cargo.toml package.version", cargo.package?.version],
    ["src-tauri/Cargo.lock offpdf.version", cargoLock.package?.find((entry) => entry.name === "offpdf")?.version],
    ["src-tauri/tauri.conf.json version", tauri?.version],
  ];
  const expected = pkg?.version;
  if (versions.some(([, version]) => typeof version !== "string" || !version.trim() || version !== expected)) {
    const details = versions.map(([field, version]) => `${field}: ${version === undefined ? "<missing>" : JSON.stringify(version)}`);
    throw new Error(`Release version metadata is inconsistent:\n${details.join("\n")}`);
  }
  console.log(`Release version metadata matches: ${expected}`);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
