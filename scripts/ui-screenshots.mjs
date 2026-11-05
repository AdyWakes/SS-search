#!/usr/bin/env node
// Renders the UI against the in-browser mock backend and saves screenshots to
// docs/screenshots. Needs Playwright with Chromium available.
//   node scripts/ui-screenshots.mjs [--scale 2] [--out docs/screenshots]
import { createServer } from "vite";
import { createRequire } from "node:module";
import { mkdirSync } from "node:fs";
import { join, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
let playwright;
try {
  playwright = require("playwright");
} catch {
  playwright = require(join(process.env.NODE_PATH ?? "", "playwright"));
}
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const arg = (n, d) => {
  const i = process.argv.indexOf(n);
  return i >= 0 ? process.argv[i + 1] : d;
};
const out = resolve(root, arg("--out", "docs/screenshots"));
const scale = Number(arg("--scale", "2"));
mkdirSync(out, { recursive: true });

const server = await createServer({ root, server: { port: 1430, strictPort: false }, logLevel: "error" });
await server.listen();
const base = server.resolvedUrls.local[0].replace(/\/$/, "");
const browser = await playwright.chromium.launch();

async function shot(name, { url = "/", size = [1180, 780], scheme = "dark", steps = async () => {} } = {}) {
  const ctx = await browser.newContext({ viewport: { width: size[0], height: size[1] }, deviceScaleFactor: scale, colorScheme: scheme });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => console.error(`[${name}] page error:`, e.message));
  await page.goto(base + url);
  await page.waitForTimeout(400);
  await steps(page);
  await page.waitForTimeout(350);
  await page.screenshot({ path: join(out, `${name}.png`) });
  console.log("saved", name);
  await ctx.close();
}

const type = (q) => async (p) => {
  await p.locator("input").first().fill(q);
  await p.waitForTimeout(450);
};

await shot("01-recent");
await shot("02-text-search", { steps: type("upi payment rahul") });
await shot("03-meaning-search", { steps: type("cat meme") });
await shot("04-typo", { steps: type("eror cargo") });
await shot("05-preview", {
  steps: async (p) => {
    await type("receipt total")(p);
    await p.locator('[role="option"]').first().click({ button: "right" });
    await p.waitForTimeout(300);
  },
});
await shot("06-settings", { steps: async (p) => p.getByLabel("Settings").click() });
await shot("07-onboarding", { url: "/?mock=onboarding" });
await shot("08-indexing", { url: "/?mock=indexing", steps: type("error") });
await shot("09-quick-bar", { url: "/?view=bar", size: [760, 470], steps: type("payment") });
await shot("10-light", { scheme: "light", steps: type("whiteboard") });

await browser.close();
await server.close();
