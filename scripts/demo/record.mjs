// Records docs/demo.gif from stage.html, frame by frame, or renders
// docs/screenshot.png from screenshot.html.
//
//   cd scripts/demo && npm install && node record.mjs            # GIF
//   node record.mjs screenshot                                    # PNG
//
// Needs Chrome or Chromium (set CHROME_PATH, or have Playwright's browser
// cache or Google Chrome installed) and gifski (`brew install gifski`).

import { chromium } from "playwright-core";
import { createServer } from "node:http";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir, homedir } from "node:os";
import { join, extname, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const OUT = join(ROOT, "docs/demo.gif");
const FPS = Number(process.env.FPS || 15);
const WIDTH = 1000, HEIGHT = 690;

function findChrome() {
  if (process.env.CHROME_PATH) return process.env.CHROME_PATH;
  const cache = join(homedir(), "Library/Caches/ms-playwright");
  if (existsSync(cache)) {
    const dirs = readdirSync(cache).filter((d) => /^chromium-\d+$/.test(d)).sort((a, b) => b.split("-")[1] - a.split("-")[1]);
    for (const d of dirs) {
      for (const sub of ["chrome-mac-arm64", "chrome-mac", "chrome-mac-x64"]) {
        const p = join(cache, d, sub, "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing");
        if (existsSync(p)) return p;
      }
    }
  }
  const system = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
  if (existsSync(system)) return system;
  throw new Error("No Chrome found. Set CHROME_PATH.");
}

const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".png": "image/png", ".svg": "image/svg+xml" };
const server = createServer((req, res) => {
  const path = join(ROOT, decodeURIComponent(new URL(req.url, "http://x").pathname));
  if (!path.startsWith(ROOT) || !existsSync(path)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { "content-type": TYPES[extname(path)] || "application/octet-stream" }).end(readFileSync(path));
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const base = `http://127.0.0.1:${server.address().port}`;

const browser = await chromium.launch({ executablePath: findChrome() });

if (process.argv[2] === "screenshot") {
  try {
    const context = await browser.newContext({ viewport: { width: 1332, height: 730 }, deviceScaleFactor: 2, colorScheme: "light", reducedMotion: "reduce", locale: "en-US", timezoneId: "America/Sao_Paulo" });
    await context.clock.setFixedTime(new Date("2026-11-27T09:12:00-03:00"));
    const page = await context.newPage();
    page.on("pageerror", (e) => console.error("page error:", e.message));
    await page.goto(`${base}/scripts/demo/screenshot.html`);
    await page.evaluate(() => window.shotReady);
    const out = join(ROOT, "docs/screenshot.png");
    await page.locator("#shot").screenshot({ path: out });
    console.log(`wrote ${out}`);
  } finally {
    await browser.close();
    server.close();
  }
  process.exit(0);
}

const frames = mkdtempSync(join(tmpdir(), "limitlane-demo-"));
try {
  const context = await browser.newContext({
    viewport: { width: WIDTH, height: HEIGHT },
    deviceScaleFactor: 2,
    colorScheme: "dark",
    reducedMotion: "reduce", // no CSS transitions: every frame is set explicitly
    locale: "en-US",
    timezoneId: "America/Sao_Paulo",
  });
  // 09:12 in the page's time zone, whatever zone this machine is in.
  await context.clock.setFixedTime(new Date("2026-11-27T09:12:00-03:00"));
  const page = await context.newPage();
  page.on("pageerror", (e) => console.error("page error:", e.message));
  await page.goto(`${base}/scripts/demo/stage.html`);
  await page.evaluate(() => window.demo.ready);
  const duration = await page.evaluate(() => window.demo.duration);
  const count = Math.round(duration * FPS);
  for (let i = 0; i < count; i++) {
    await page.evaluate((ms) => window.demo.frame(ms), (i * 1000) / FPS);
    await page.screenshot({ path: join(frames, `f${String(i).padStart(4, "0")}.png`) });
    if (i % FPS === 0) process.stdout.write(`\rframe ${i}/${count}`);
  }
  process.stdout.write(`\rframe ${count}/${count}\n`);
} finally {
  await browser.close();
  server.close();
}

const files = readdirSync(frames).filter((f) => f.endsWith(".png")).sort().map((f) => join(frames, f));
execFileSync("gifski", ["--fps", String(FPS), "--width", String(WIDTH), "--quality", process.env.QUALITY || "90", "-o", OUT, ...files], { stdio: "inherit" });
if (!process.env.KEEP_FRAMES) rmSync(frames, { recursive: true, force: true });
else console.log(`frames kept in ${frames}`);
console.log(`wrote ${OUT}`);
