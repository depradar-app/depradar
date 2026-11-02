// The landing page (docs/index.html + landing.js) run in jsdom.

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { JSDOM } = require("jsdom");

const read = (file) => fs.readFileSync(path.join(__dirname, file), "utf8");
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const RELEASE = {
  assets: [
    { name: "depradar_1.0.0_amd64.deb", browser_download_url: "https://dl/deb" },
    { name: "depradar_1.0.0_amd64.deb.sig", browser_download_url: "https://dl/deb.sig" },
    { name: "depradar_1.0.0_amd64.AppImage", browser_download_url: "https://dl/appimage" },
    { name: "depradar_1.0.0_amd64.AppImage.sig", browser_download_url: "https://dl/appimage.sig" },
    { name: "depradar_1.0.0_x64-setup.exe", browser_download_url: "https://dl/exe" },
    { name: "depradar_1.0.0_universal.dmg", browser_download_url: "https://dl/dmg" },
  ],
};

async function load({ fetchImpl = async () => ({ json: async () => RELEASE }), clipboard } = {}) {
  const html = read("index.html").replace(/<script[^>]*><\/script>/g, "");
  const dom = new JSDOM(html, { url: "http://localhost/", runScripts: "outside-only" });
  const { window } = dom;
  window.fetch = fetchImpl;
  if (clipboard) Object.defineProperty(window.navigator, "clipboard", { value: clipboard, configurable: true });
  new vm.Script(read("landing.js"), { filename: path.join(__dirname, "landing.js") }).runInContext(
    dom.getInternalVMContext()
  );
  await sleep(10);
  return window;
}

test("direct download links resolve to the exact asset, never the .sig file", async () => {
  const window = await load();
  const href = (asset) => window.document.querySelector(`a.direct[data-asset="${asset}"]`).href;
  assert.equal(href("_amd64.deb"), "https://dl/deb");
  assert.equal(href(".AppImage"), "https://dl/appimage");
  assert.equal(href("x64-setup.exe"), "https://dl/exe");
  assert.equal(href(".dmg"), "https://dl/dmg");
});

test("links keep their releases-page fallback when the API call fails or has no match", async () => {
  for (const fetchImpl of [
    async () => {
      throw new Error("offline");
    },
    async () => ({ json: async () => ({ message: "rate limited" }) }),
  ]) {
    const window = await load({ fetchImpl });
    for (const link of window.document.querySelectorAll("a.direct")) {
      assert.equal(link.href, "https://github.com/depradar-app/depradar/releases/latest");
    }
  }
});

test("the language switcher translates the page and remembers the choice", async () => {
  const window = await load();
  const select = window.document.getElementById("lang-select");
  assert.equal(select.options.length, 8);
  const tagline = () => window.document.querySelector("[data-i18n='landing.tagline']").textContent;
  assert.match(tagline(), /^Detects outdated/);
  select.value = "fr";
  select.dispatchEvent(new window.Event("change"));
  assert.match(tagline(), /^Détecte/);
  assert.equal(window.localStorage.getItem("depradar_lang"), "fr");
  assert.equal(window.document.documentElement.lang, "fr");
});

test("every landing locale defines the same keys as English", async () => {
  const window = await load();
  const ctx = window;
  const strings = vm.runInContext("STRINGS", ctx);
  const english = Object.keys(strings.en).sort();
  for (const code of Object.keys(strings)) {
    assert.deepEqual(Object.keys(strings[code]).sort(), english, code);
  }
});

test("the copy buttons copy the command, with a fallback when the clipboard API is missing", async () => {
  const copied = [];
  const window = await load({ clipboard: { writeText: async (text) => copied.push(text) } });
  const button = window.document.querySelector(".copy-btn");
  button.click();
  await sleep(10);
  assert.equal(copied.length, 1);
  assert.match(copied[0], /install\.sh/);
  assert.equal(button.textContent, "✓");

  const noClipboard = await load();
  let execCalls = 0;
  noClipboard.document.execCommand = () => {
    execCalls++;
    return true;
  };
  noClipboard.document.querySelector(".copy-btn").click();
  assert.equal(execCalls, 1);

  const failing = await load({
    clipboard: {
      writeText: async () => {
        throw new Error("denied");
      },
    },
  });
  let fallbackCalls = 0;
  failing.document.execCommand = () => {
    fallbackCalls++;
    return true;
  };
  failing.document.querySelector(".copy-btn").click();
  await sleep(10);
  assert.equal(fallbackCalls, 1);
});

test("the OS tabs switch between the macOS/Linux and Windows commands", async () => {
  const window = await load();
  const tabs = [...window.document.querySelectorAll("#install-tabs button")];
  assert.ok(tabs.length >= 2);
  const windowsTab = tabs.find((b) => b.dataset.os === "win" || /win/i.test(b.dataset.os || ""));
  assert.ok(windowsTab, "a Windows tab exists");
  windowsTab.click();
  assert.ok(!window.document.getElementById("install-win").classList.contains("hidden"));
  assert.ok(window.document.getElementById("install-mac").classList.contains("hidden"));
});

test("the page loads no third-party resources", () => {
  const html = read("index.html");
  assert.doesNotMatch(html, /fonts\.(googleapis|gstatic)\.com/);
  const external = [...html.matchAll(/\b(?:src|href)="(https?:[^"]*)"/g)].map((m) => m[1]);
  for (const url of external) assert.match(url, /^https:\/\/github\.com\/depradar-app\//, url);
  assert.ok(fs.existsSync(path.join(__dirname, "fonts", "Outfit.woff2")));
  assert.ok(fs.existsSync(path.join(__dirname, "fonts", "JetBrainsMono.woff2")));
});
