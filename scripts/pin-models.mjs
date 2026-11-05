#!/usr/bin/env node
// Pins SHA-256 hashes (and exact sizes) for model downloads in
// crates/shotfind-core/models.json. The app refuses any download whose hash is
// not pinned, so run this once on a machine with internet access, REVIEW the
// diff, and commit it.
//
//   node scripts/pin-models.mjs           fill in missing hashes
//   node scripts/pin-models.mjs --check   exit 1 if any hash is missing (no network)
//   node scripts/pin-models.mjs --verify  re-download everything and compare
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const manifestPath = join(root, "crates", "shotfind-core", "models.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const mode = process.argv.includes("--check") ? "check" : process.argv.includes("--verify") ? "verify" : "pin";
const isHex64 = (s) => /^[0-9a-f]{64}$/i.test(s ?? "");

async function hashUrl(url) {
  const res = await fetch(url, { redirect: "follow", headers: { "user-agent": "shotfind-pin-models" } });
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  const h = createHash("sha256");
  let size = 0;
  const reader = res.body.getReader();
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    h.update(value);
    size += value.length;
  }
  return { sha256: h.digest("hex"), size };
}

let missing = 0;
let changed = false;
let failed = false;
for (const model of manifest.models) {
  for (const d of model.downloads) {
    const pinned = isHex64(d.sha256);
    if (!pinned) missing++;
    if (mode === "check") {
      if (!pinned) console.error(`UNPINNED: ${model.id} ${d.url}`);
      continue;
    }
    if (mode === "pin" && pinned) {
      console.log(`ok (already pinned) ${d.url}`);
      continue;
    }
    process.stdout.write(`hashing ${d.url} ... `);
    const { sha256, size } = await hashUrl(d.url);
    console.log(`${sha256} (${size} bytes)`);
    if (mode === "verify") {
      if (pinned && sha256 !== d.sha256.toLowerCase()) {
        console.error(`  MISMATCH: manifest has ${d.sha256}`);
        failed = true;
      }
      continue;
    }
    d.sha256 = sha256;
    d.size = size;
    changed = true;
  }
}

if (mode === "check") {
  if (missing > 0) {
    console.error(`${missing} download(s) have no pinned SHA-256. Run: node scripts/pin-models.mjs`);
    process.exit(1);
  }
  console.log("All model downloads are pinned.");
} else if (mode === "verify") {
  process.exit(failed ? 1 : 0);
} else if (changed) {
  writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + "\n");
  console.log(`Updated ${manifestPath}. Review the diff (git diff) before committing.`);
}
