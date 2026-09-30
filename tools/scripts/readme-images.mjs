// Regenerates the README's concept images (docs/images/*.png) from the design
// mockups in docs/design, so they can be redone after a mockup changes and
// always come out framed the same way.
//
//   node tools/scripts/readme-images.mjs
//
// Each image is a screenshot of named elements in a mockup (the union of
// everything its selector matches, plus padding), not a fixed pixel region,
// so layout changes elsewhere on the page don't shift the crop. Rendered at 2x
// with reduced motion (the mockups then skip their entry animations) by the
// Edge or Chrome already installed, over the DevTools protocol; no npm
// packages. Names in the mockups are made up (The Expanse); keep it that way.

import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const out = join(root, "docs", "images");

/** name -> where it comes from. `take` limits how many matches are included. */
const SHOTS = {
  panel: { page: "alert-styles.html", selector: "#panel .stage" },
  strip: { page: "alert-styles.html", selector: "#strip .stage" },
  beacon: { page: "alert-styles.html", selector: "#beacon .stage" },
  toasts: { page: "windows-notification.html", selector: "#grid .toast", take: 2, pad: 10 },
  actioncenter: { page: "windows-notification.html", selector: ".center", pad: 10 },
  "perf-fps": { page: "performance.html", selector: "#fps", pad: 0 },
  "perf-memory": { page: "performance.html", selector: "#memory", pad: 0 },
  "perf-cpu": { page: "performance.html", selector: "#cpu", pad: 0 },
};
const SCALE = 2;
const VIEWPORT = { width: 1400, height: 1800 };

const BROWSERS = [
  "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
  "C:/Program Files/Google/Chrome/Application/chrome.exe",
];

function fail(msg) {
  console.error(`readme-images: ${msg}`);
  process.exit(1);
}

const exe = BROWSERS.find(existsSync) ?? fail("no Edge or Chrome found");
const profile = mkdtempSync(join(tmpdir(), "readme-images-"));
const browser = spawn(exe, ["--headless=new", "--disable-gpu", "--hide-scrollbars", "--remote-debugging-port=0", `--user-data-dir=${profile}`, "about:blank"], {
  stdio: ["ignore", "ignore", "pipe"],
});

// The browser prints its DevTools address on stderr once it's listening.
const wsUrl = await new Promise((ok, no) => {
  let buf = "";
  const t = setTimeout(() => no(new Error("the browser didn't start")), 20000);
  browser.stderr.on("data", (d) => {
    buf += d;
    const m = buf.match(/DevTools listening on (ws:\/\/\S+)/);
    if (m) {
      clearTimeout(t);
      ok(m[1]);
    }
  });
  browser.on("exit", () => no(new Error("the browser exited early")));
}).catch((e) => fail(e.message));

// A minimal DevTools protocol client.
const ws = new WebSocket(wsUrl);
await new Promise((ok) => ws.addEventListener("open", ok, { once: true }));
let nextId = 0;
const pending = new Map();
const waiters = [];
ws.addEventListener("message", (ev) => {
  const msg = JSON.parse(ev.data);
  if (msg.id !== undefined && pending.has(msg.id)) {
    const { ok, no } = pending.get(msg.id);
    pending.delete(msg.id);
    msg.error ? no(new Error(msg.error.message)) : ok(msg.result);
  } else if (msg.method) {
    for (const w of [...waiters]) if (w.method === msg.method && w.sessionId === msg.sessionId) {
      waiters.splice(waiters.indexOf(w), 1);
      w.ok(msg.params);
    }
  }
});
const send = (method, params = {}, sessionId) =>
  new Promise((ok, no) => {
    const id = ++nextId;
    pending.set(id, { ok, no });
    ws.send(JSON.stringify({ id, method, params, sessionId }));
  });
const event = (method, sessionId) => new Promise((ok) => waiters.push({ method, sessionId, ok }));

async function openPage(file) {
  const { targetId } = await send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
  const s = (m, p) => send(m, p, sessionId);
  await s("Page.enable");
  await s("Emulation.setDeviceMetricsOverride", { ...VIEWPORT, deviceScaleFactor: SCALE, mobile: false });
  await s("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "reduce" }] });
  const loaded = event("Page.loadEventFired", sessionId);
  await s("Page.navigate", { url: pathToFileURL(join(root, "docs", "design", file)).href });
  await loaded;
  await s("Runtime.evaluate", { expression: "document.fonts.ready", awaitPromise: true });
  return s;
}

mkdirSync(out, { recursive: true });
const pages = new Map();
let failed = false;
try {
  for (const [name, shot] of Object.entries(SHOTS)) {
    if (!pages.has(shot.page)) pages.set(shot.page, await openPage(shot.page));
    const s = pages.get(shot.page);
    const { result } = await s("Runtime.evaluate", {
      returnByValue: true,
      expression: `(() => {
        const els = [...document.querySelectorAll(${JSON.stringify(shot.selector)})].slice(0, ${shot.take ?? 1e9});
        if (!els.length) return null;
        const r = els.map(e => e.getBoundingClientRect());
        const x = Math.min(...r.map(b => b.left)) + scrollX, y = Math.min(...r.map(b => b.top)) + scrollY;
        return { x, y, w: Math.max(...r.map(b => b.right)) + scrollX - x, h: Math.max(...r.map(b => b.bottom)) + scrollY - y, n: els.length };
      })()`,
    });
    const box = result.value;
    if (!box) {
      console.error(`${name}: nothing matches "${shot.selector}" in ${shot.page}`);
      failed = true;
      continue;
    }
    const pad = shot.pad ?? 0;
    const { data } = await s("Page.captureScreenshot", {
      format: "png",
      captureBeyondViewport: true,
      clip: { x: box.x - pad, y: box.y - pad, width: box.w + 2 * pad, height: box.h + 2 * pad, scale: 1 },
    });
    writeFileSync(join(out, `${name}.png`), Buffer.from(data, "base64"));
    console.log(`${name}.png  ${Math.round((box.w + 2 * pad) * SCALE)}x${Math.round((box.h + 2 * pad) * SCALE)}  (${box.n} element${box.n > 1 ? "s" : ""})`);
  }
} finally {
  ws.close();
  browser.kill();
  // The browser can hold its profile folder for a moment after exiting.
  setTimeout(() => rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }), 500);
}
if (failed) process.exitCode = 1;
