// End-to-end tests of the webview UI: the real index.html, logic.cjs,
// i18n.cjs and main.js run in jsdom, with Tauri's JS API replaced by a fake
// whose calls are recorded.

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { JSDOM } = require("jsdom");

const SRC = __dirname;
const read = (file) => fs.readFileSync(path.join(SRC, file), "utf8");

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(predicate, what, timeoutMs = 3000) {
  const start = Date.now();
  while (!predicate()) {
    if (Date.now() - start > timeoutMs) assert.fail(`timed out waiting for ${what}`);
    await sleep(5);
  }
}

const REPORT = {
  source: "/work/app",
  warnings: [
    { code: "lockfile_v1_unsupported", path: "old/package-lock.json", detail: null },
    { code: "mystery_code", path: null, detail: "x" },
  ],
  manifests: [
    {
      ecosystem: "npm",
      manifest_path: "package.json",
      lockfile_path: "package-lock.json",
      dependencies: [
        {
          name: "lodash",
          package: null,
          declared: "^4.17.0",
          installed: "4.17.19",
          latest: "4.17.21",
          status: "outdated",
          reason: null,
          vulnerabilities: ["GHSA-29mw-wpgm-hmr9", "CVE-2020-8203"],
          vuln_basis: "installed",
        },
        {
          name: "alias",
          package: "react",
          declared: "npm:react@^17",
          installed: null,
          latest: "18.3.0",
          status: "major",
          reason: null,
          vulnerabilities: [],
          vuln_basis: null,
        },
        {
          name: "internal",
          package: null,
          declared: "workspace:*",
          installed: null,
          latest: null,
          status: "unknown",
          reason: "non_registry",
          vulnerabilities: [],
          vuln_basis: null,
        },
        {
          name: "zod",
          package: null,
          declared: "^3.0.0",
          installed: "3.23.0",
          latest: "3.23.0",
          status: "uptodate",
          reason: null,
          vulnerabilities: [],
          vuln_basis: "installed",
        },
      ],
    },
  ],
};

// Boots the app in a fresh jsdom window with a scriptable fake backend.
async function boot({ initialState = "", legacy = {}, handlers = {} } = {}) {
  const html = read("index.html").replace(/<script[^>]*><\/script>/g, "");
  const dom = new JSDOM(html, { url: "http://localhost/", pretendToBeVisual: true, runScripts: "outside-only" });
  const { window } = dom;
  for (const [k, v] of Object.entries(legacy)) window.localStorage.setItem(k, v);

  const calls = [];
  let savedState = initialState;
  let tokenSet = false;
  const invoke = async (command, args) => {
    calls.push([command, args]);
    if (handlers[command]) return handlers[command](args);
    switch (command) {
      case "load_state":
        return savedState;
      case "save_state":
        savedState = args.json;
        return;
      case "github_token_status":
        return tokenSet;
      case "save_github_token":
        tokenSet = !!args.token;
        return;
      case "scan_local":
      case "scan_github":
        return REPORT;
      case "export_report":
        return true;
      default:
        throw { code: "unknown" };
    }
  };

  const updater = { update: null, installed: false };
  window.__TAURI__ = {
    core: { invoke },
    dialog: { open: async () => "/picked/folder" },
    app: { getVersion: async () => "9.9.9" },
    updater: { check: async () => updater.update },
    process: { relaunch: async () => calls.push(["relaunch"]) },
  };
  window.matchMedia = () => ({ matches: false, addEventListener() {} });
  window.HTMLElement.prototype.setPointerCapture = () => {};

  for (const file of ["logic.cjs", "i18n.cjs", "main.js"]) {
    new vm.Script(read(file), { filename: path.join(SRC, file) }).runInContext(dom.getInternalVMContext());
  }
  // jsdom fires DOMContentLoaded by itself once the document is parsed.

  const doc = window.document;
  const $ = (sel) => doc.querySelector(sel);
  const $$ = (sel) => [...doc.querySelectorAll(sel)];
  const click = (sel) => $(sel).dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
  const submit = (sel) => $(sel).dispatchEvent(new window.Event("submit", { bubbles: true, cancelable: true }));
  const type = (sel, value) => {
    $(sel).value = value;
    $(sel).dispatchEvent(new window.Event("input", { bubbles: true }));
  };
  // Plain copies: objects created inside the jsdom realm have another
  // prototype, which strict deep-equality would reject.
  const called = (command) => JSON.parse(JSON.stringify(calls.filter(([c]) => c === command)));

  await until(() => called("github_token_status").length > 0, "startup");
  await sleep(10);
  return { window, doc, $, $$, click, submit, type, calls, called, updater, getSaved: () => savedState, dom };
}

async function addLocalProject(app, folder = "/work/app") {
  app.click("#add-project-btn-main");
  app.type("#local-path", folder);
  app.submit("#local-form");
  await until(() => app.$$(".manifest-card").length > 0, "scan results");
}

test("starts on the empty state and shows the app version", async () => {
  const app = await boot();
  assert.ok(!app.$("#empty-state").classList.contains("hidden"));
  assert.ok(app.$("#project-view").classList.contains("hidden"));
  await until(() => app.$("#app-version-btn").textContent === "v9.9.9", "version");
  assert.equal(app.doc.documentElement.lang, "en");
  assert.equal(app.$("#lang-select").getAttribute("aria-label"), "Language");
  assert.deepEqual(
    [...app.$("#lang-select").options].map((o) => o.value),
    ["en", "fr", "es", "de", "pt", "it", "zh-CN", "ja"]
  );
});

test("adding a local project scans it and renders the results", async () => {
  const app = await boot();
  await addLocalProject(app);

  assert.equal(app.called("scan_local")[0][1].path, "/work/app");
  assert.ok(app.called("scan_local")[0][1].scanId, "a scan id is sent so the scan can be cancelled");
  assert.ok(app.$("#add-modal").classList.contains("hidden"));
  assert.equal(app.$("#project-title").textContent, "/work/app");
  assert.equal(app.$("#project-badge").textContent, "Local");

  const rows = app.$$("tbody tr");
  assert.equal(rows.length, 4);
  // Default sort: most severe first.
  assert.deepEqual(
    rows.map((r) => r.cells[0].firstChild.textContent),
    ["alias", "lodash", "internal", "zod"]
  );
  assert.equal(app.$(".manifest-card h2").firstChild.textContent, "package.json");
  assert.equal(app.$(".ecosystem-badge").textContent, "JavaScript");

  // Alias shows the real package; non-registry shows its reason as a tooltip.
  assert.match(rows[0].cells[0].textContent, /alias → react/);
  assert.match(rows[2].querySelector(".pill").title, /not looked up on a registry/);

  // Vulnerabilities: a summary with the count and a link per advisory.
  const links = [...rows[1].querySelectorAll(".vuln-list a")];
  assert.equal(links.length, 2);
  assert.equal(links[0].href, "https://osv.dev/vulnerability/GHSA-29mw-wpgm-hmr9");
  assert.equal(links[0].rel, "noopener noreferrer");
  assert.equal(rows[1].querySelector("summary").textContent, "Vulnerable (2)");
  assert.match(rows[1].querySelector("summary").title, /installed version/);

  // Warnings are translated; unknown codes fall back to their raw parts.
  const warnings = app.$$(".warnings p").map((p) => p.textContent);
  assert.equal(warnings[0], "old/package-lock.json: only lockfileVersion 1 data found; installed versions were not detected.");
  assert.equal(warnings[1], "mystery_code: x");

  // Sidebar shows the project with a health badge (1 breaking + 1 outdated + 1 unknown).
  assert.equal(app.$(".project-item-label").textContent, "app");
  assert.equal(app.$(".health-badge").textContent, "3");
  assert.equal(app.$(".health-badge").className, "health-badge danger");
  assert.match(app.$(".status").textContent, /^Last scan:/);
});

test("the saved projects are written to the Rust-side state file", async () => {
  const app = await boot();
  await addLocalProject(app);
  await until(() => app.getSaved() !== "", "state to be saved", 2000);
  const saved = JSON.parse(app.getSaved());
  assert.equal(saved.length, 1);
  assert.equal(saved[0].value, "/work/app");
  assert.equal(saved[0].lastReport.manifests.length, 1);
  assert.equal("scanning" in saved[0], false);
});

test("a second launch restores the saved projects and migrates the legacy token", async () => {
  const state = JSON.stringify([
    { id: "p1", type: "local", value: "/work/app", lastReport: REPORT, lastError: null, lastScannedAt: Date.now(), scanning: true },
  ]);
  const app = await boot({ initialState: state, legacy: { depradar_github_token: "ghp_old" } });
  assert.equal(app.$$(".project-item").length, 1);
  assert.equal(app.called("save_github_token")[0][1].token, "ghp_old");
  assert.equal(app.window.localStorage.getItem("depradar_github_token"), null);

  app.click(".project-item");
  assert.equal(app.$$("tbody tr").length, 4);
  assert.ok(!app.$("#status").textContent.includes("Scanning"), "the stale `scanning` flag is ignored");
});

test("an empty legacy token is dropped without being saved", async () => {
  const app = await boot({ legacy: { depradar_github_token: "" } });
  assert.equal(app.called("save_github_token").length, 0);
  assert.equal(app.window.localStorage.getItem("depradar_github_token"), null);
});

test("saved projects from localStorage are migrated to the state file", async () => {
  const legacyProjects = JSON.stringify([{ id: "old", type: "github", value: "o/r", lastReport: null }]);
  const app = await boot({ legacy: { depradar_saved_projects: legacyProjects } });
  assert.equal(app.$$(".project-item").length, 1);
  assert.equal(JSON.parse(app.getSaved())[0].id, "old");
  assert.equal(app.window.localStorage.getItem("depradar_saved_projects"), null);
});

test("hostile package names are rendered as text, never as markup", async () => {
  const evil = JSON.parse(JSON.stringify(REPORT));
  evil.manifests[0].dependencies[0].name = '<img src=x onerror="window.pwned=1">';
  evil.manifests[0].dependencies[0].vulnerabilities = ['"><script>window.pwned=2</script>'];
  evil.warnings = [{ code: "read_failed", path: "<b>x</b>", detail: "<i>y</i>" }];
  const app = await boot({ handlers: { scan_local: () => evil } });
  await addLocalProject(app);
  assert.equal(app.$$("#results img, #results script, #results b, #results i").length, 0);
  assert.equal(app.window.pwned, undefined);
  assert.ok(app.$("#results").textContent.includes('<img src=x onerror="window.pwned=1">'));
  const link = app.$(".vuln-list a");
  assert.equal(new URL(link.href).hostname, "osv.dev");
});

test("filters: search, status pills and the vulnerable-only toggle", async () => {
  const app = await boot();
  await addLocalProject(app);
  const visible = () => app.$$("tbody tr").map((r) => r.cells[0].firstChild.textContent);

  app.type("#search-input", "LOD");
  await until(() => visible().length === 1, "debounced search");
  assert.deepEqual(visible(), ["lodash"]);
  app.type("#search-input", "");
  await until(() => visible().length === 4, "search reset");

  app.click('.filter-pill[data-status="major"]');
  assert.deepEqual(visible(), ["alias"]);
  app.click('.filter-pill[data-status="major"]');
  assert.equal(visible().length, 4);

  app.click("#vuln-filter-pill");
  assert.deepEqual(visible(), ["lodash"]);
  app.click("#vuln-filter-pill");

  app.type("#search-input", "nothing-matches");
  await until(() => app.$(".manifest-card .empty"), "empty message");
  assert.equal(app.$(".manifest-card .empty").textContent, "No package matches the filters.");
});

test("sorting by clicking or pressing Enter on a column header toggles direction", async () => {
  const app = await boot();
  await addLocalProject(app);
  const names = () => app.$$("tbody tr").map((r) => r.cells[0].firstChild.textContent);
  const header = () => app.$$("th.sortable")[0];

  header().dispatchEvent(new app.window.MouseEvent("click", { bubbles: true }));
  assert.deepEqual(names(), ["alias", "internal", "lodash", "zod"]);
  assert.match(header().textContent, /Package ▲/);
  header().dispatchEvent(new app.window.MouseEvent("click", { bubbles: true }));
  assert.deepEqual(names(), ["zod", "lodash", "internal", "alias"]);
  header().dispatchEvent(new app.window.KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  assert.deepEqual(names(), ["alias", "internal", "lodash", "zod"]);
  header().dispatchEvent(new app.window.KeyboardEvent("keydown", { key: "x", bubbles: true }));
  assert.deepEqual(names(), ["alias", "internal", "lodash", "zod"]);
  app.$$("th.sortable")[1].dispatchEvent(new app.window.MouseEvent("click", { bubbles: true }));
  assert.match(app.$$("th.sortable")[1].textContent, /Status ▲/);
});

test("switching language retranslates the UI, report labels included", async () => {
  const app = await boot();
  await addLocalProject(app);
  app.$("#lang-select").value = "fr";
  app.$("#lang-select").dispatchEvent(new app.window.Event("change", { bubbles: true }));
  assert.equal(app.doc.documentElement.lang, "fr");
  assert.equal(app.$$("th")[0].textContent.trim().startsWith("Paquet"), true);
  assert.equal(app.$("#scan-all-btn").textContent, "Tout rescanner");
  assert.match(app.$(".warnings p").textContent, /lockfileVersion 1/);
  assert.match(app.$("summary").textContent, /Vulnérable/);

  app.click("#export-btn");
  await until(() => app.called("export_report").length > 0, "export");
  const { content, defaultName } = app.called("export_report")[0][1];
  assert.match(content, /^# Rapport DepRadar — \/work\/app/);
  const fr = require("./i18n.cjs").STRINGS.fr;
  const header = ["package", "declared", "installed", "latest", "status", "vulnerabilities"]
    .map((k) => fr[`table.${k}`])
    .join(" | ");
  assert.ok(content.includes(`| ${header} |`), header);
  assert.equal(defaultName, "app-depradar-report.md");
});

test("export hands the report to Rust and surfaces failures", async () => {
  const app = await boot({
    handlers: {
      export_report: () => {
        throw { code: "export_write_failed", detail: "denied" };
      },
    },
  });
  await addLocalProject(app);
  app.click("#export-btn");
  await until(() => app.$("#status").textContent.includes("Failed to write the report"), "export error");
});

test("deleting a project asks for confirmation first", async () => {
  const app = await boot();
  await addLocalProject(app);

  app.click("#delete-project-btn");
  assert.ok(!app.$("#confirm-modal").classList.contains("hidden"));
  assert.match(app.$("#confirm-modal-message").textContent, /Remove "\/work\/app"/);
  app.click("#confirm-cancel-btn");
  assert.ok(app.$("#confirm-modal").classList.contains("hidden"));
  assert.equal(app.$$(".project-item").length, 1);

  app.click("#delete-project-btn");
  app.click("#confirm-ok-btn");
  assert.equal(app.$$(".project-item").length, 0);
  assert.ok(!app.$("#empty-state").classList.contains("hidden"));
});

test("Escape closes the open modal and focus returns to the opener", async () => {
  const app = await boot();
  const opener = app.$("#add-project-btn");
  opener.focus();
  app.click("#add-project-btn");
  assert.ok(!app.$("#add-modal").classList.contains("hidden"));
  assert.equal(app.doc.activeElement.id, "local-path");
  app.doc.dispatchEvent(new app.window.KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  assert.ok(app.$("#add-modal").classList.contains("hidden"));
  assert.equal(app.doc.activeElement, opener);
});

test("the about modal shows the version", async () => {
  const app = await boot();
  await until(() => app.$("#app-version-btn").textContent === "v9.9.9", "version");
  app.click("#app-version-btn");
  assert.ok(!app.$("#about-modal").classList.contains("hidden"));
  assert.equal(app.$("#about-version").textContent, "Version 9.9.9");
  app.click("#about-modal-close-btn");
  assert.ok(app.$("#about-modal").classList.contains("hidden"));
});

test("a GitHub project is scanned with the repo only: the token never reaches the webview", async () => {
  const app = await boot();
  app.click("#add-project-btn");
  app.click('.tab-btn[data-tab="github"]');
  assert.ok(!app.$('[data-panel="github"]').classList.contains("hidden"));
  app.type("#github-repo", "o/r");
  app.type("#github-token", "ghp_secret");
  app.submit("#github-form");
  await until(() => app.$$(".manifest-card").length > 0, "github scan");

  assert.deepEqual(app.called("save_github_token")[0][1], { token: "ghp_secret" });
  assert.equal(app.called("scan_github")[0][1].repo, "o/r");
  assert.equal(Object.keys(app.called("scan_github")[0][1]).sort().join(), "repo,scanId", "no token among the arguments");
  assert.equal(app.$("#project-badge").textContent, "GitHub");
  assert.equal(app.window.localStorage.getItem("depradar_github_token"), null);

  // A token is now saved: the field says so and offers to remove it.
  app.click("#add-project-btn");
  assert.match(app.$("#github-token").placeholder, /already|saved/i);
  assert.ok(!app.$("#github-token-remove").classList.contains("hidden"));
  app.click("#github-token-remove");
  await until(() => app.called("save_github_token").length === 2, "token removal");
  assert.deepEqual(app.called("save_github_token")[1][1], { token: "" });
  await until(() => app.$("#github-token-remove").classList.contains("hidden"), "token field reset");
});

test("a keychain failure while saving the token is reported but not fatal", async () => {
  const app = await boot({
    handlers: {
      save_github_token: () => {
        throw { code: "token_storage_failed", detail: "no secret service" };
      },
    },
  });
  app.click("#add-project-btn");
  app.click('.tab-btn[data-tab="github"]');
  app.type("#github-repo", "o/r");
  app.type("#github-token", "ghp_x");
  app.submit("#github-form");
  await until(() => app.called("scan_github").length > 0, "scan still runs");
  assert.match(app.$("#app-notice").textContent, /system keychain/);
  assert.ok(app.$("#app-notice").classList.contains("warn"));
});

test("scan errors are shown translated and rescanning is possible", async () => {
  let fail = true;
  const app = await boot({
    handlers: {
      scan_local: () => {
        if (fail) throw { code: "local_dir_not_found", detail: "/gone" };
        return REPORT;
      },
    },
  });
  app.click("#add-project-btn");
  app.type("#local-path", "/gone");
  app.submit("#local-form");
  await until(() => app.$("#status").textContent.includes("not found"), "error status");
  assert.equal(app.$("#status").textContent, 'Directory "/gone" not found.');
  assert.ok(app.$("#filters-bar").classList.contains("hidden"));

  fail = false;
  app.click("#refresh-btn");
  await until(() => app.$$(".manifest-card").length > 0, "rescan");
  assert.match(app.$("#status").textContent, /^Last scan:/);
});

test("the refresh button becomes a cancel button while a scan runs", async () => {
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const app = await boot({
    handlers: {
      scan_local: async () => {
        await gate;
        throw { code: "scan_cancelled" };
      },
      cancel_scan: () => release(),
    },
  });
  app.click("#add-project-btn");
  app.type("#local-path", "/slow");
  app.submit("#local-form");
  await until(() => app.$("#refresh-btn").textContent === "✕", "cancel button");
  assert.equal(app.$("#refresh-btn").title, "Cancel the scan");
  assert.equal(app.$("#status").textContent, "Scanning…");
  app.click("#refresh-btn");
  await until(() => app.$("#refresh-btn").textContent === "↻", "refresh button back");
  assert.equal(app.called("cancel_scan")[0][1].scanId.length > 0, true);
  assert.equal(app.$("#status").textContent, "", "a cancelled scan is not an error");
});

test("an unexpected error object falls back to a generic message", async () => {
  const app = await boot({
    handlers: {
      scan_local: () => {
        throw "boom";
      },
    },
  });
  app.click("#add-project-btn");
  app.type("#local-path", "/x");
  app.submit("#local-form");
  await until(() => app.$("#status").textContent === "An error occurred during the scan.", "generic error");
});

test("Scan all rescans every project and shows progress", async () => {
  const app = await boot();
  await addLocalProject(app, "/work/a");
  app.click("#add-project-btn");
  app.type("#local-path", "/work/b");
  app.submit("#local-form");
  await until(() => app.called("scan_local").length === 2, "second scan");
  await until(() => !app.$("#refresh-btn").disabled, "scans to settle");

  const before = app.called("scan_local").length;
  app.click("#scan-all-btn");
  await until(() => app.called("scan_local").length === before + 2, "both rescanned");
  await until(() => app.$("#scan-all-btn").textContent === "Scan all" && !app.$("#scan-all-btn").disabled, "button reset");
});

test("Scan all does nothing without projects", async () => {
  const app = await boot();
  app.click("#scan-all-btn");
  await sleep(20);
  assert.equal(app.called("scan_local").length, 0);
  assert.equal(app.$("#scan-all-btn").disabled, false);
});

test("the folder picker fills the path, and tolerates a failing dialog", async () => {
  const app = await boot();
  app.click("#add-project-btn");
  app.click("#browse-btn");
  await until(() => app.$("#local-path").value === "/picked/folder", "picked folder");
  app.window.__TAURI__.dialog.open = async () => {
    throw new Error("no dialog");
  };
  app.type("#local-path", "");
  app.click("#browse-btn");
  await sleep(20);
  assert.equal(app.$("#local-path").value, "");
  app.submit("#local-form"); // empty path: ignored
  assert.equal(app.called("scan_local").length, 0);
});

test("the theme button toggles and persists the theme", async () => {
  const app = await boot();
  assert.equal(app.$("#theme-btn").textContent, "☾");
  app.click("#theme-btn");
  assert.equal(app.doc.documentElement.dataset.theme, "dark");
  assert.equal(app.$("#theme-btn").textContent, "☀");
  app.click("#theme-btn");
  assert.equal(app.doc.documentElement.dataset.theme, "light");
});

test("the sidebar can be resized with the keyboard and the width is remembered", async () => {
  const app = await boot();
  const resizer = app.$("#sidebar-resizer");
  const key = (k, extra = {}) => resizer.dispatchEvent(new app.window.KeyboardEvent("keydown", { key: k, bubbles: true, ...extra }));
  const width = () => app.doc.documentElement.style.getPropertyValue("--sidebar-width");
  assert.equal(width(), "260px");
  key("ArrowRight");
  assert.equal(width(), "276px");
  key("ArrowLeft", { shiftKey: true });
  assert.equal(width(), "228px");
  key("Home");
  assert.equal(width(), "200px");
  key("End");
  assert.equal(width(), `${Math.max(200, Math.min(600, app.window.innerWidth - 320))}px`);
  key("Tab");
  assert.equal(app.window.localStorage.getItem("depradar_sidebar_width"), width().replace("px", ""));

  // Dragging with the pointer.
  const pointer = (type, props = {}) => {
    const e = new app.window.MouseEvent(type, { bubbles: true, cancelable: true, ...props });
    e.pointerId = 1;
    resizer.dispatchEvent(e);
  };
  pointer("pointerdown");
  assert.ok(resizer.classList.contains("dragging"));
  pointer("pointermove", { clientX: 300 });
  assert.equal(width(), "300px");
  pointer("pointercancel");
  assert.ok(!resizer.classList.contains("dragging"));
  assert.equal(app.window.localStorage.getItem("depradar_sidebar_width"), "300");
  pointer("pointermove", { clientX: 450 });
  assert.equal(width(), "300px", "moves are ignored when not dragging");
  app.window.dispatchEvent(new app.window.Event("resize"));
});

test("the update flow asks for confirmation before installing", async () => {
  const app = await boot();
  app.updater.update = {
    version: "1.2.3",
    downloadAndInstall: async () => {
      app.updater.installed = true;
    },
  };
  app.click("#check-update-btn");
  await until(() => !app.$("#confirm-modal").classList.contains("hidden"), "confirmation");
  assert.equal(app.$("#confirm-modal-title").textContent, "Install update");
  assert.match(app.$("#confirm-modal-message").textContent, /Version 1\.2\.3 is available/);
  assert.equal(app.$("#confirm-ok-btn").textContent, "Install");

  // Cancel: nothing installed, and the dialog goes back to its default texts.
  app.click("#confirm-cancel-btn");
  assert.equal(app.updater.installed, false);
  assert.equal(app.$("#confirm-ok-btn").textContent, "Delete");
  assert.equal(app.$("#confirm-modal-title").textContent, "Confirm deletion");

  app.click("#check-update-btn");
  await until(() => !app.$("#confirm-modal").classList.contains("hidden"), "second confirmation");
  app.click("#confirm-ok-btn");
  await until(() => app.called("relaunch").length === 1, "relaunch");
  assert.ok(app.updater.installed);
  assert.equal(app.$("#update-status").textContent, "Relaunching…");
});

test("update check results: up to date, failure to check, failure to install", async () => {
  const app = await boot();
  app.click("#check-update-btn");
  await until(() => app.$("#update-status").textContent === "You're up to date.", "up to date");
  assert.equal(app.$("#check-update-btn").disabled, false);

  app.window.__TAURI__.updater.check = async () => {
    throw new Error("offline");
  };
  app.click("#check-update-btn");
  await until(() => app.$("#update-status").textContent === "Update check failed.", "check failure");

  app.window.__TAURI__.updater.check = async () => ({
    version: "2.0.0",
    downloadAndInstall: async () => {
      throw new Error("disk full");
    },
  });
  app.click("#check-update-btn");
  await until(() => !app.$("#confirm-modal").classList.contains("hidden"), "confirmation");
  app.click("#confirm-ok-btn");
  await until(() => app.$("#update-status").textContent === "Update check failed.", "install failure");
  assert.equal(app.$("#check-update-btn").disabled, false);
});

test("startup state errors are shown in the notice banner", async () => {
  const app = await boot({
    handlers: {
      load_state: () => {
        throw { code: "state_io_failed", detail: "permission denied" };
      },
    },
  });
  assert.match(app.$("#app-notice").textContent, /permission denied/);
  assert.ok(app.$("#app-notice").classList.contains("danger"));
});

test("a failing version lookup leaves the version button empty", async () => {
  const app = await boot();
  assert.equal(typeof app.$("#app-version-btn").textContent, "string");
});

test("the page loads no external resources and has no inline scripts or styles (CSP-safe)", () => {
  const html = read("index.html");
  const external = [...html.matchAll(/\b(?:src|href)="(https?:[^"]*)"/g)].map((m) => m[1]);
  assert.deepEqual(external, ["https://github.com/depradar-app/depradar"], "only the repo link is external");
  assert.doesNotMatch(html, /\sstyle=/);
  assert.doesNotMatch(html, /<script(?![^>]*\ssrc=)/);
  assert.doesNotMatch(html, /\son[a-z]+=/);
  assert.doesNotMatch(read("styles.css"), /https?:\/\//);
  const conf = JSON.parse(fs.readFileSync(path.join(SRC, "..", "src-tauri", "tauri.conf.json"), "utf8"));
  const csp = conf.app.security.csp;
  assert.match(csp, /default-src 'self'/);
  assert.doesNotMatch(csp, /unsafe-inline|unsafe-eval|\*/);
});
