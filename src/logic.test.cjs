const test = require("node:test");
const assert = require("node:assert/strict");
const {
  generateId,
  filterAndSortDeps,
  formatReportMarkdown,
  sortIndicator,
  projectDisplayName,
  projectHealthBadge,
  reportFileName,
  clampSidebarWidth,
  debounce,
  normalizeError,
  runScan,
  cancelScan,
  scanAll,
  createRemoteStorage,
  PROJECTS_KEY,
  loadProjects,
  findProject,
  findExisting,
  upsertProject,
  removeProject,
} = require("./logic.cjs");

function makeStorage(initial = {}) {
  const store = { ...initial };
  return {
    getItem: (key) => (key in store ? store[key] : null),
    setItem: (key, value) => {
      store[key] = value;
    },
  };
}

const deps = [
  { name: "zod", status: "uptodate" },
  { name: "react", status: "major" },
  { name: "axios", status: "outdated" },
  { name: "internal-lib", status: "unknown" },
  { name: "react-dom", status: "major" },
];

function names(result) {
  return result.map((d) => d.name);
}

test("default status-severity sort (asc)", () => {
  assert.deepEqual(names(filterAndSortDeps(deps)), ["react", "react-dom", "axios", "internal-lib", "zod"]);
});

test("status-severity sort desc reverses ties too", () => {
  assert.deepEqual(
    names(filterAndSortDeps(deps, { sortDirection: "desc" })),
    ["zod", "internal-lib", "axios", "react-dom", "react"]
  );
});

test("name sort asc", () => {
  assert.deepEqual(
    names(filterAndSortDeps(deps, { sortColumn: "name" })),
    ["axios", "internal-lib", "react", "react-dom", "zod"]
  );
});

test("name sort desc", () => {
  assert.deepEqual(
    names(filterAndSortDeps(deps, { sortColumn: "name", sortDirection: "desc" })),
    ["zod", "react-dom", "react", "internal-lib", "axios"]
  );
});

test("name filter is case-insensitive substring match", () => {
  assert.deepEqual(names(filterAndSortDeps(deps, { nameFilter: "REACT" })), ["react", "react-dom"]);
});

test("status filter with a single value", () => {
  assert.deepEqual(names(filterAndSortDeps(deps, { statusFilterSet: new Set(["major"]) })), ["react", "react-dom"]);
});

test("status filter with multiple values is a union", () => {
  assert.deepEqual(
    names(filterAndSortDeps(deps, { statusFilterSet: new Set(["major", "unknown"]) })),
    ["react", "react-dom", "internal-lib"]
  );
});

test("name filter and status filter combine", () => {
  assert.deepEqual(
    names(filterAndSortDeps(deps, { statusFilterSet: new Set(["major"]), nameFilter: "dom" })),
    ["react-dom"]
  );
});

test("empty status filter set means no filtering", () => {
  assert.equal(filterAndSortDeps(deps, { statusFilterSet: new Set() }).length, 5);
});

test("vulnOnly keeps only packages with a non-empty vulnerabilities list", () => {
  const withVulns = [
    { name: "zod", status: "uptodate", vulnerabilities: [] },
    { name: "react", status: "major", vulnerabilities: ["GHSA-xxxx"] },
    { name: "axios", status: "outdated" }, // no vulnerabilities field at all
  ];
  assert.deepEqual(names(filterAndSortDeps(withVulns, { vulnOnly: true })), ["react"]);
});

test("vulnOnly combines with the other filters", () => {
  const withVulns = [
    { name: "react", status: "major", vulnerabilities: ["GHSA-xxxx"] },
    { name: "react-dom", status: "major", vulnerabilities: [] },
  ];
  assert.deepEqual(
    names(filterAndSortDeps(withVulns, { vulnOnly: true, statusFilterSet: new Set(["major"]) })),
    ["react"]
  );
});

test("generateId returns unique, non-empty string ids", () => {
  const a = generateId();
  const b = generateId();
  assert.equal(typeof a, "string");
  assert.ok(a.length > 0);
  assert.notEqual(a, b);
});

test("sortIndicator shows an ascending arrow for the active column", () => {
  assert.equal(sortIndicator("name", "name", "asc"), " ▲");
});

test("sortIndicator shows a descending arrow for the active column", () => {
  assert.equal(sortIndicator("name", "name", "desc"), " ▼");
});

test("sortIndicator is empty for a non-active column", () => {
  assert.equal(sortIndicator("name", "status", "asc"), "");
});

test("loadProjects returns an empty array when storage is empty", () => {
  assert.deepEqual(loadProjects(makeStorage()), []);
});

test("loadProjects is resilient to malformed JSON", () => {
  const storage = makeStorage({ depradar_saved_projects: "not json" });
  assert.deepEqual(loadProjects(storage), []);
});

test("loadProjects is resilient to a non-array stored value", () => {
  const storage = makeStorage({ depradar_saved_projects: JSON.stringify({ not: "an array" }) });
  assert.deepEqual(loadProjects(storage), []);
});

test("upsertProject adds a new project and persistProjects/loadProjects round-trip", () => {
  const storage = makeStorage();
  upsertProject({ id: "a1", type: "local", value: "/tmp/foo" }, storage);
  assert.deepEqual(loadProjects(storage), [{ id: "a1", type: "local", value: "/tmp/foo" }]);
});

test("upsertProject updates an existing project by id instead of duplicating it", () => {
  const storage = makeStorage();
  upsertProject({ id: "a1", type: "local", value: "/tmp/foo", lastScannedAt: 1 }, storage);
  upsertProject({ id: "a1", type: "local", value: "/tmp/foo", lastScannedAt: 2 }, storage);
  const list = loadProjects(storage);
  assert.equal(list.length, 1);
  assert.equal(list[0].lastScannedAt, 2);
});

test("findProject finds by id, and returns null when absent", () => {
  const storage = makeStorage();
  upsertProject({ id: "a1", type: "local", value: "/tmp/foo" }, storage);
  assert.equal(findProject("a1", storage).value, "/tmp/foo");
  assert.equal(findProject("missing", storage), null);
});

test("findExisting matches by (type, value) pair", () => {
  const storage = makeStorage();
  upsertProject({ id: "a1", type: "local", value: "/tmp/foo" }, storage);
  assert.equal(findExisting("local", "/tmp/foo", storage).id, "a1");
  assert.equal(findExisting("github", "/tmp/foo", storage), null);
  assert.equal(findExisting("local", "/tmp/other", storage), null);
});

test("formatReportMarkdown builds one table per manifest with a title and source line", () => {
  const project = { type: "local", value: "/tmp/my-app" };
  const report = {
    warnings: [],
    manifests: [
      {
        manifest_path: "package.json",
        dependencies: [
          {
            name: "react",
            declared: "^18.0.0",
            installed: "18.2.0",
            latest: "18.3.1",
            status: "outdated",
            vulnerabilities: [],
          },
          {
            name: "lodash",
            declared: "^4.17.19",
            installed: "4.17.19",
            latest: "4.17.21",
            status: "outdated",
            vulnerabilities: ["GHSA-29mw-wpgm-hmr9"],
          },
        ],
      },
    ],
  };
  const md = formatReportMarkdown(project, report);
  assert.match(md, /^# DepRadar report — \/tmp\/my-app/);
  assert.match(md, /Source: Local/);
  assert.match(md, /## package\.json/);
  assert.match(md, /\| react \| \^18\.0\.0 \| 18\.2\.0 \| 18\.3\.1 \| Outdated \| — \|/);
  assert.match(md, /\| lodash \| \^4\.17\.19 \| 4\.17\.19 \| 4\.17\.21 \| Outdated \| GHSA-29mw-wpgm-hmr9 \|/);
});

test("formatReportMarkdown uses supplied status labels and falls back to '—' for missing values", () => {
  const project = { type: "github", value: "owner/repo" };
  const report = {
    warnings: [],
    manifests: [
      {
        manifest_path: "requirements.txt",
        dependencies: [{ name: "requests", declared: "", installed: null, latest: null, status: "unknown" }],
      },
    ],
  };
  const md = formatReportMarkdown(project, report, { status: { unknown: "Inconnu" } });
  assert.match(md, /Source: GitHub/);
  assert.match(md, /\| requests \| — \| — \| — \| Inconnu \| — \|/);
});

test("formatReportMarkdown includes warnings as blockquotes", () => {
  const project = { type: "local", value: "/tmp/my-app" };
  const report = {
    warnings: [{ code: "lockfile_v1_unsupported", path: "package-lock.json", detail: null }],
    manifests: [],
  };
  const md = formatReportMarkdown(project, report);
  assert.match(md, /> ⚠️ lockfile_v1_unsupported: package-lock\.json/);
  const translated = formatReportMarkdown(project, report, { warning: (w) => `Avertissement ${w.path}` });
  assert.match(translated, /> ⚠️ Avertissement package-lock\.json/);
});

test("removeProject removes only the matching project", () => {
  const storage = makeStorage();
  upsertProject({ id: "a1", type: "local", value: "/tmp/foo" }, storage);
  upsertProject({ id: "a2", type: "local", value: "/tmp/bar" }, storage);
  removeProject("a1", storage);
  const list = loadProjects(storage);
  assert.equal(list.length, 1);
  assert.equal(list[0].id, "a2");
});

test("projectDisplayName keeps only the last path segment", () => {
  assert.equal(projectDisplayName("/home/me/dev/depradar"), "depradar");
  assert.equal(projectDisplayName("/home/me/dev/depradar/"), "depradar");
  assert.equal(projectDisplayName("C:\\Users\\me\\app"), "app");
  assert.equal(projectDisplayName("https://github.com/owner/repo"), "repo");
  assert.equal(projectDisplayName("https://github.com/owner/repo.git"), "repo");
  assert.equal(projectDisplayName("owner/repo"), "repo");
  assert.equal(projectDisplayName("repo"), "repo");
});

test("formatReportMarkdown escapes pipes so OR-ranges keep the table aligned", () => {
  const project = { type: "local", value: "/tmp/app" };
  const report = {
    warnings: [],
    manifests: [
      {
        manifest_path: "package.json",
        dependencies: [{ name: "a", declared: "^1.0.0 || ^2.0.0", installed: null, latest: "2.1.0", status: "uptodate" }],
      },
    ],
  };
  const row = formatReportMarkdown(project, report).split("\n").find((l) => l.startsWith("| a |"));
  assert.equal(row, "| a | ^1.0.0 \\|\\| ^2.0.0 | — | 2.1.0 | Up to date | — |");
  // Unescaped cells in this row: 6 columns => 7 unescaped pipes.
  assert.equal(row.replace(/\\\|/g, "").split("|").length - 1, 7);
});

test("formatReportMarkdown uses translated headings and labels", () => {
  const project = { type: "github", value: "o/r" };
  const labels = {
    title: (n) => `Rapport — ${n}`,
    source: "Origine",
    generated: "Généré le",
    sourceGithub: "GitHub",
    columns: { package: "Paquet", vulnerabilities: "Vulnérabilités" },
  };
  const md = formatReportMarkdown(project, { warnings: [], manifests: [{ manifest_path: "a", dependencies: [] }] }, labels);
  assert.match(md, /^# Rapport — o\/r/);
  assert.match(md, /Origine: GitHub/);
  assert.match(md, /Généré le: /);
  assert.match(md, /\| Paquet \| Declared \| Installed \| Latest \| Status \| Vulnérabilités \|/);
});

test("reportFileName never contains characters Windows forbids", () => {
  assert.equal(reportFileName({ value: "/home/me/app" }), "app-depradar-report.md");
  assert.equal(reportFileName({ value: "C:\\Users\\me\\proj" }), "proj-depradar-report.md");
  assert.equal(reportFileName({ value: "owner/repo" }), "repo-depradar-report.md");
  assert.equal(reportFileName({ value: "a:b" }), "a-b-depradar-report.md");
  assert.doesNotMatch(reportFileName({ value: 'we?ird*"name' }), /[<>:"/\\|?*]/);
});

test("projectHealthBadge counts non-up-to-date dependencies", () => {
  const label = (s) => s.toUpperCase();
  assert.equal(projectHealthBadge({ lastReport: null }, label), null);
  const clean = { lastReport: { manifests: [{ dependencies: [{ status: "uptodate" }] }] } };
  assert.equal(projectHealthBadge(clean, label), null);
  const mixed = {
    lastReport: {
      manifests: [
        { dependencies: [{ status: "major" }, { status: "outdated" }, { status: "uptodate" }] },
        { dependencies: [{ status: "outdated" }, { status: "unknown" }] },
      ],
    },
  };
  assert.deepEqual(projectHealthBadge(mixed, label), {
    count: 4,
    cls: "danger",
    title: "1 MAJOR, 2 OUTDATED, 1 UNKNOWN",
  });
  const warnOnly = { lastReport: { manifests: [{ dependencies: [{ status: "outdated" }] }] } };
  assert.equal(projectHealthBadge(warnOnly, label).cls, "warn");
});

test("clampSidebarWidth keeps room for the main content", () => {
  assert.equal(clampSidebarWidth(100), 200);
  assert.equal(clampSidebarWidth(900), 600);
  assert.equal(clampSidebarWidth(350.4), 350);
  assert.equal(clampSidebarWidth(500, 800), 480);
  assert.equal(clampSidebarWidth(500, 400), 200);
});

test("debounce runs once with the last arguments, and can be cancelled", () => {
  const pending = [];
  const timers = {
    set: (fn) => pending.push(fn) && pending.length,
    clear: (handle) => {
      pending[handle - 1] = null;
    },
  };
  const calls = [];
  const d = debounce((x) => calls.push(x), 100, timers);
  const fire = () => pending.splice(0).filter(Boolean).forEach((fn) => fn());
  d(1);
  d(2);
  d(3);
  fire();
  assert.deepEqual(calls, [3]);
  d(4);
  d.cancel();
  fire();
  assert.deepEqual(calls, [3]);
  d.cancel(); // no-op when idle
});

test("loadProjects drops the transient `scanning` flag older versions persisted", () => {
  const storage = makeStorage({ [PROJECTS_KEY]: JSON.stringify([{ id: "a", scanning: true }]) });
  assert.deepEqual(loadProjects(storage), [{ id: "a" }]);
});

test("loadProjects reuses the parse while the stored text is unchanged", () => {
  const storage = makeStorage();
  upsertProject({ id: "a", type: "local", value: "/x" }, storage);
  assert.equal(loadProjects(storage), loadProjects(storage));
  storage.setItem(PROJECTS_KEY, "[]");
  assert.deepEqual(loadProjects(storage), []);
});

test("normalizeError keeps structured errors and maps anything else to unknown", () => {
  assert.deepEqual(normalizeError({ code: "network_error", detail: "x" }), { code: "network_error", detail: "x" });
  assert.deepEqual(normalizeError("boom"), { code: "unknown" });
  assert.deepEqual(normalizeError(null), { code: "unknown" });
  assert.deepEqual(normalizeError({ message: "no code" }), { code: "unknown" });
});

/* --- runScan / scanAll --- */

const report = { source: "s", manifests: [], warnings: [] };

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

test("runScan stores the report and uses the right command per project type", async () => {
  const storage = makeStorage();
  upsertProject({ id: "l", type: "local", value: "/tmp/p" }, storage);
  upsertProject({ id: "g", type: "github", value: "o/r" }, storage);
  const calls = [];
  const invoke = async (command, args) => {
    calls.push([command, args]);
    return report;
  };
  const scanning = new Set();
  let changes = 0;
  const deps = { invoke, storage, scanning, onChange: () => changes++, now: () => 42 };
  assert.deepEqual(await runScan("l", deps), { outcome: "done" });
  await runScan("g", deps);
  assert.deepEqual(calls, [
    ["scan_local", { path: "/tmp/p", scanId: "l" }],
    ["scan_github", { repo: "o/r", scanId: "g" }],
  ]);
  const saved = findProject("l", storage);
  assert.deepEqual(saved.lastReport, report);
  assert.equal(saved.lastScannedAt, 42);
  assert.equal(saved.lastError, null);
  assert.equal(scanning.size, 0);
  assert.equal(changes, 4); // start + end, twice
});

test("runScan records a structured error and keeps the previous report", async () => {
  const storage = makeStorage();
  upsertProject({ id: "l", type: "local", value: "/tmp/p", lastReport: report }, storage);
  const invoke = async () => {
    throw { code: "local_dir_not_found", detail: "/tmp/p" };
  };
  await runScan("l", { invoke, storage, scanning: new Set() });
  const saved = findProject("l", storage);
  assert.deepEqual(saved.lastError, { code: "local_dir_not_found", detail: "/tmp/p" });
  assert.deepEqual(saved.lastReport, report);
});

test("runScan refuses a second concurrent scan of the same project", async () => {
  const storage = makeStorage();
  upsertProject({ id: "l", type: "local", value: "/tmp/p" }, storage);
  const gate = deferred();
  let invocations = 0;
  const invoke = () => {
    invocations++;
    return gate.promise;
  };
  const scanning = new Set();
  const first = runScan("l", { invoke, storage, scanning });
  assert.ok(scanning.has("l"));
  assert.deepEqual(await runScan("l", { invoke, storage, scanning }), { outcome: "busy" });
  gate.resolve(report);
  assert.deepEqual(await first, { outcome: "done" });
  assert.equal(invocations, 1);
});

test("runScan does not resurrect a project deleted during its scan", async () => {
  const storage = makeStorage();
  upsertProject({ id: "l", type: "local", value: "/tmp/p" }, storage);
  const gate = deferred();
  const scanning = new Set();
  const pending = runScan("l", { invoke: () => gate.promise, storage, scanning });
  removeProject("l", storage);
  gate.resolve(report);
  assert.deepEqual(await pending, { outcome: "missing" });
  assert.deepEqual(loadProjects(storage), []);
  assert.equal(scanning.size, 0);
});

test("runScan on an unknown project is a no-op", async () => {
  const invoke = () => assert.fail("must not scan");
  assert.deepEqual(await runScan("nope", { invoke, storage: makeStorage(), scanning: new Set() }), { outcome: "missing" });
});

test("runScan reports a persistence failure instead of throwing", async () => {
  const storage = makeStorage();
  upsertProject({ id: "l", type: "local", value: "/tmp/p" }, storage);
  const failing = { getItem: storage.getItem, setItem: () => { throw new Error("quota"); } };
  const result = await runScan("l", { invoke: async () => report, storage: failing, scanning: new Set() });
  assert.equal(result.outcome, "persist_failed");
  assert.match(String(result.persistError), /quota/);
});

test("scanAll scans every project in order and reports progress", async () => {
  const storage = makeStorage();
  upsertProject({ id: "a", type: "local", value: "/a" }, storage);
  upsertProject({ id: "b", type: "local", value: "/b" }, storage);
  const order = [];
  const progress = [];
  const count = await scanAll(
    { invoke: async (_c, args) => order.push(args.path) && report, storage, scanning: new Set() },
    (current, total) => progress.push([current, total])
  );
  assert.equal(count, 2);
  assert.deepEqual(order, ["/a", "/b"]);
  assert.deepEqual(progress, [[1, 2], [2, 2]]);
  assert.equal(await scanAll({ invoke: async () => report, storage: makeStorage(), scanning: new Set() }), 0);
});

/* --- createRemoteStorage --- */

function fakeTimers() {
  const queue = [];
  return {
    timers: {
      set: (fn) => queue.push(fn) && queue.length,
      clear: (h) => {
        queue[h - 1] = null;
      },
    },
    run: async () => {
      for (const fn of queue.splice(0)) if (fn) await fn();
    },
  };
}

test("createRemoteStorage loads the saved state and writes changes back, debounced", async () => {
  const saved = [];
  const invoke = async (command, args) => {
    if (command === "load_state") return JSON.stringify([{ id: "a", type: "local", value: "/a" }]);
    saved.push(args.json);
  };
  const { timers, run } = fakeTimers();
  const { storage } = await createRemoteStorage({ invoke, timers });
  assert.equal(loadProjects(storage)[0].id, "a");
  upsertProject({ id: "b", type: "local", value: "/b" }, storage);
  upsertProject({ id: "c", type: "local", value: "/c" }, storage);
  assert.deepEqual(saved, []);
  await run();
  assert.equal(saved.length, 1, "one coalesced write");
  assert.equal(JSON.parse(saved[0]).length, 3);
  assert.equal(storage.getItem("other"), null);
  storage.setItem("other", "x"); // unrelated keys are ignored
  await run();
  assert.equal(saved.length, 1);
});

test("createRemoteStorage migrates projects saved in localStorage by older versions", async () => {
  const legacy = makeStorage({ [PROJECTS_KEY]: JSON.stringify([{ id: "old", type: "local", value: "/old" }]) });
  let removed = false;
  legacy.removeItem = () => {
    removed = true;
  };
  const saved = [];
  const invoke = async (command, args) => {
    if (command === "load_state") return "";
    saved.push(args.json);
  };
  const { storage } = await createRemoteStorage({ invoke, legacy, timers: fakeTimers().timers });
  assert.equal(loadProjects(storage)[0].id, "old");
  assert.equal(saved.length, 1, "migration is written immediately");
  assert.ok(removed, "the legacy copy is removed once saved");
});

test("createRemoteStorage keeps the legacy copy if the migration write fails, and reports errors", async () => {
  const legacy = makeStorage({ [PROJECTS_KEY]: "[]" });
  let removed = false;
  legacy.removeItem = () => {
    removed = true;
  };
  const errors = [];
  const invoke = async (command) => {
    if (command === "load_state") throw { code: "state_io_failed", detail: "x" };
    throw { code: "state_io_failed", detail: "disk full" };
  };
  await createRemoteStorage({ invoke, legacy, onError: (e) => errors.push(e.detail), timers: fakeTimers().timers });
  assert.deepEqual(errors, ["x", "disk full"]);
  assert.equal(removed, false);
});

test("createRemoteStorage flush writes pending changes immediately", async () => {
  const saved = [];
  const invoke = async (command, args) => {
    if (command === "load_state") return "";
    saved.push(args.json);
  };
  const { storage, flush } = await createRemoteStorage({ invoke, timers: fakeTimers().timers });
  await flush();
  assert.equal(saved.length, 0, "nothing to flush");
  upsertProject({ id: "a", type: "local", value: "/a" }, storage);
  await flush();
  assert.equal(saved.length, 1);
});

test("createRemoteStorage tolerates an unreadable legacy storage", async () => {
  const legacy = {
    getItem: () => {
      throw new Error("denied");
    },
  };
  const { storage } = await createRemoteStorage({ invoke: async () => "", legacy, timers: fakeTimers().timers });
  assert.deepEqual(loadProjects(storage), []);
});

test("a cancelled scan leaves the previous result untouched", async () => {
  const storage = makeStorage();
  upsertProject({ id: "l", type: "local", value: "/tmp/p", lastReport: report, lastError: null }, storage);
  const invoke = async () => {
    throw { code: "scan_cancelled" };
  };
  assert.deepEqual(await runScan("l", { invoke, storage, scanning: new Set() }), { outcome: "done" });
  const saved = findProject("l", storage);
  assert.deepEqual(saved.lastReport, report);
  assert.equal(saved.lastError, null);
});

test("cancelScan asks Rust to abort that scan id", async () => {
  const calls = [];
  await cancelScan("p1", { invoke: async (c, a) => calls.push([c, a]) });
  assert.deepEqual(calls, [["cancel_scan", { scanId: "p1" }]]);
});

test("debounce's default timers work in an engine that checks the receiver of setTimeout", async () => {
  const realSet = globalThis.setTimeout;
  // Like a browser: setTimeout only works when called without a foreign `this`.
  globalThis.setTimeout = function (fn, ms) {
    if (this !== undefined && this !== globalThis) throw new TypeError("Illegal invocation");
    return realSet(fn, ms);
  };
  try {
    const calls = [];
    const d = debounce((x) => calls.push(x), 5);
    d(1);
    d(2);
    await new Promise((resolve) => realSet(resolve, 30));
    assert.deepEqual(calls, [2]);
  } finally {
    globalThis.setTimeout = realSet;
  }
});
