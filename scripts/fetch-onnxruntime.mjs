#!/usr/bin/env node
// Downloads the official ONNX Runtime release from Microsoft's GitHub, verifies
// its SHA-256, and copies the shared library into src-tauri/resources/onnxruntime
// so it gets bundled with the app. Build-time only; the app itself never does this.
//
// Usage: node scripts/fetch-onnxruntime.mjs [--target <rust-target-triple>] [--out <dir>]
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { createWriteStream, existsSync, mkdirSync, readFileSync, copyFileSync, rmSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";

const ORT_VERSION = "1.22.0";
const BASE = `https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}`;

// sha256 of the release archives, pinned.
const ARCHIVES = {
  "x86_64-pc-windows-msvc": {
    file: `onnxruntime-win-x64-${ORT_VERSION}.zip`,
    sha256: "174c616efc0271194488642a72f1a514e01487da4dfe84c49296d66e40ebe0da",
    libs: [["lib/onnxruntime.dll", "onnxruntime.dll"], ["lib/onnxruntime_providers_shared.dll", "onnxruntime_providers_shared.dll"]],
  },
  "aarch64-apple-darwin": {
    file: `onnxruntime-osx-arm64-${ORT_VERSION}.tgz`,
    sha256: "cab6dcbd77e7ec775390e7b73a8939d45fec3379b017c7cb74f5b204c1a1cc07",
    libs: [[`lib/libonnxruntime.${ORT_VERSION}.dylib`, "libonnxruntime.dylib"]],
  },
  "x86_64-apple-darwin": {
    file: `onnxruntime-osx-x86_64-${ORT_VERSION}.tgz`,
    sha256: "e4ec94a7696de74fb1b12846569aa94e499958af6ffa186022cfde16c9d617f0",
    libs: [[`lib/libonnxruntime.${ORT_VERSION}.dylib`, "libonnxruntime.dylib"]],
  },
  "x86_64-unknown-linux-gnu": {
    file: `onnxruntime-linux-x64-${ORT_VERSION}.tgz`,
    sha256: "8344d55f93d5bc5021ce342db50f62079daf39aaafb5d311a451846228be49b3",
    libs: [[`lib/libonnxruntime.so.${ORT_VERSION}`, "libonnxruntime.so"]],
  },
};

function hostTarget() {
  const { platform, arch } = process;
  if (platform === "win32") return "x86_64-pc-windows-msvc";
  if (platform === "darwin") return arch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin";
  if (platform === "linux" && arch === "x64") return "x86_64-unknown-linux-gnu";
  throw new Error(`unsupported host ${platform}/${arch}`);
}

const args = process.argv.slice(2);
const argVal = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const target = argVal("--target") ?? hostTarget();
const outDir = argVal("--out") ?? join(root, "src-tauri", "resources", "onnxruntime");
const spec = ARCHIVES[target];
if (!spec) {
  console.error(`No ONNX Runtime archive configured for ${target}. Known: ${Object.keys(ARCHIVES).join(", ")}`);
  process.exit(1);
}

const marker = join(outDir, `.ort-${ORT_VERSION}-${target}`);
if (existsSync(marker)) {
  console.log(`ONNX Runtime ${ORT_VERSION} for ${target} already present in ${outDir}`);
  process.exit(0);
}

const work = join(tmpdir(), `shotfind-ort-${process.pid}`);
mkdirSync(work, { recursive: true });
const archive = join(work, spec.file);
console.log(`Downloading ${spec.file} ...`);
const res = await fetch(`${BASE}/${spec.file}`);
if (!res.ok) throw new Error(`download failed: HTTP ${res.status}`);
await pipeline(Readable.fromWeb(res.body), createWriteStream(archive));

const digest = createHash("sha256").update(readFileSync(archive)).digest("hex");
if (digest !== spec.sha256) {
  rmSync(work, { recursive: true, force: true });
  throw new Error(`SHA-256 mismatch for ${spec.file}: got ${digest}, expected ${spec.sha256}`);
}
console.log("SHA-256 verified.");

// bsdtar (Windows 10+, macOS) and GNU tar (Linux, .tgz only) both handle these.
execFileSync("tar", ["-xf", archive, "-C", work], { stdio: "inherit" });
const extracted = join(work, spec.file.replace(/\.(zip|tgz)$/, ""));

mkdirSync(outDir, { recursive: true });
for (const [from, to] of spec.libs) {
  copyFileSync(join(extracted, from), join(outDir, to));
  console.log(`  -> ${join(outDir, to)}`);
}
copyFileSync(join(extracted, "LICENSE"), join(outDir, "ONNXRUNTIME-LICENSE.txt"));
copyFileSync(join(extracted, "ThirdPartyNotices.txt"), join(outDir, "ONNXRUNTIME-ThirdPartyNotices.txt"));
writeFileSync(marker, `${ORT_VERSION}\n`);
rmSync(work, { recursive: true, force: true });
console.log("Done.");
