// Pure logic (dependency sort/filter, id generation, saved-project
// persistence, the scan/rescan state machine, report export) — no dependency
// on the DOM or Tauri: everything that touches the outside world (Tauri's
// `invoke`, the storage) is passed in. Loaded both as a plain script in the
// app (see index.html) and as a CommonJS module in tests (see
// logic.test.cjs). This file must contain no ES import/export syntax so it
// stays loadable as-is in the webview.

const STATUS_SORT_ORDER = { major: 0, outdated: 1, unknown: 2, uptodate: 3 };

function generateId() {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

// Filters `deps` by name (substring, case-insensitive), by status (an empty
// set means no filtering), and optionally to only known-vulnerable packages,
// then sorts the result by name or by status severity, ascending or
// descending.
function filterAndSortDeps(deps, options = {}) {
  const {
    nameFilter = "",
    statusFilterSet = new Set(),
    vulnOnly = false,
    sortColumn = "status",
    sortDirection = "asc",
  } = options;

  let result = deps;

  if (nameFilter) {
    const needle = nameFilter.toLowerCase();
    result = result.filter((d) => d.name.toLowerCase().includes(needle));
  }

  if (statusFilterSet.size > 0) {
    result = result.filter((d) => statusFilterSet.has(d.status));
  }

  if (vulnOnly) {
    result = result.filter((d) => d.vulnerabilities && d.vulnerabilities.length > 0);
  }

  result = [...result].sort((a, b) => {
    let diff;
    if (sortColumn === "name") {
      diff = a.name.localeCompare(b.name);
    } else {
      diff = (STATUS_SORT_ORDER[a.status] ?? 4) - (STATUS_SORT_ORDER[b.status] ?? 4);
      if (diff === 0) diff = a.name.localeCompare(b.name);
    }
    return sortDirection === "asc" ? diff : -diff;
  });

  return result;
}

const DEFAULT_REPORT_LABELS = {
  title: (name) => `DepRadar report — ${name}`,
  source: "Source",
  generated: "Generated",
  sourceLocal: "Local",
  sourceGithub: "GitHub",
  columns: {
    package: "Package",
    declared: "Declared",
    installed: "Installed",
    latest: "Latest",
    status: "Status",
    vulnerabilities: "Vulnerabilities",
  },
  status: {
    uptodate: "Up to date",
    outdated: "Outdated",
    major: "Breaking",
    unknown: "Unknown",
  },
  warning: (w) => [w.code, w.path, w.detail].filter(Boolean).join(": "),
};

// A Markdown table cell: `|` would start a new column and a newline would
// end the row.
function mdCell(value) {
  return String(value).replaceAll("\\", "\\\\").replaceAll("|", "\\|").replace(/\r?\n/g, " ");
}

// Builds a Markdown report for one project's last scan, one table per
// manifest. `labels` lets the caller supply translated text (see i18n.cjs);
// it defaults to English so this stays testable/usable standalone.
function formatReportMarkdown(project, report, labels = DEFAULT_REPORT_LABELS, now = new Date()) {
  const l = {
    ...DEFAULT_REPORT_LABELS,
    ...labels,
    columns: { ...DEFAULT_REPORT_LABELS.columns, ...labels.columns },
    status: { ...DEFAULT_REPORT_LABELS.status, ...labels.status },
  };
  const sourceName = project.type === "local" ? l.sourceLocal : l.sourceGithub;
  const lines = [`# ${l.title(project.value)}`, ""];
  lines.push(`${l.source}: ${sourceName}  `);
  lines.push(`${l.generated}: ${now.toISOString()}`);
  lines.push("");

  if (report.warnings && report.warnings.length) {
    for (const warning of report.warnings) {
      lines.push(`> ⚠️ ${l.warning(warning)}`);
    }
    lines.push("");
  }

  const c = l.columns;
  for (const manifest of report.manifests) {
    lines.push(`## ${manifest.manifest_path}`, "");
    lines.push(
      `| ${c.package} | ${c.declared} | ${c.installed} | ${c.latest} | ${c.status} | ${c.vulnerabilities} |`
    );
    lines.push("|---|---|---|---|---|---|");
    for (const dep of manifest.dependencies) {
      const status = l.status[dep.status] || dep.status;
      const vulns = dep.vulnerabilities && dep.vulnerabilities.length ? dep.vulnerabilities.join(", ") : "—";
      lines.push(
        `| ${mdCell(dep.name)} | ${mdCell(dep.declared || "—")} | ${mdCell(dep.installed || "—")} | ${mdCell(dep.latest || "—")} | ${mdCell(status)} | ${mdCell(vulns)} |`
      );
    }
    lines.push("");
  }

  return lines.join("\n");
}

// A file name that is valid on every OS for a project's exported report:
// the project's short name with every character Windows forbids removed.
function reportFileName(project) {
  const name = Array.from(projectDisplayName(project.value), (ch) =>
    '<>:"/\\|?*'.includes(ch) || ch.charCodeAt(0) < 32 ? "-" : ch
  ).join("");
  return `${name || "project"}-depradar-report.md`;
}

// Sort-direction arrow for a table header, given the currently active
// column/direction.
function sortIndicator(column, sortColumn, sortDirection) {
  if (sortColumn !== column) return "";
  return sortDirection === "asc" ? " ▲" : " ▼";
}

// Last path segment of a local folder or GitHub URL/"owner/repo" — the
// project's short name for the sidebar.
function projectDisplayName(value) {
  const parts = String(value).trim().split(/[\\/]+/).filter(Boolean);
  const last = parts.length ? parts[parts.length - 1] : String(value);
  return last.replace(/\.git$/, "") || last;
}

// Tallies non-up-to-date dependencies across every manifest of a project's
// last scan, for the small sidebar count badge. `null` when there's nothing
// to show (unscanned, or everything's up to date). `label(status)` supplies
// the (translated) status names for the tooltip.
function projectHealthBadge(project, label) {
  const manifests = project.lastReport && project.lastReport.manifests;
  if (!manifests) return null;

  const counts = { major: 0, outdated: 0, unknown: 0 };
  for (const manifest of manifests) {
    for (const dep of manifest.dependencies) {
      if (dep.status in counts) counts[dep.status]++;
    }
  }

  const count = counts.major + counts.outdated + counts.unknown;
  if (count === 0) return null;

  const parts = [];
  for (const status of ["major", "outdated", "unknown"]) {
    if (counts[status]) parts.push(`${counts[status]} ${label(status)}`);
  }
  return { count, cls: counts.major > 0 ? "danger" : "warn", title: parts.join(", ") };
}

// Sidebar width limits: never wider than 600px, and always leaving room for
// the main content in a narrow window.
const SIDEBAR_MIN = 200;
const SIDEBAR_MAX = 600;
const MAIN_MIN_WIDTH = 320;

function clampSidebarWidth(width, windowWidth = Infinity) {
  const max = Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, windowWidth - MAIN_MIN_WIDTH));
  return Math.min(max, Math.max(SIDEBAR_MIN, Math.round(width)));
}

// The default timers are wrapped in arrow functions: storing the bare
// `setTimeout` in an object and calling it as `timers.set(...)` makes browsers
// throw "Can only call Window.setTimeout on instances of Window".
const DEFAULT_TIMERS = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: (handle) => clearTimeout(handle),
};

function debounce(fn, delayMs, timers = DEFAULT_TIMERS) {
  let handle = null;
  const debounced = (...args) => {
    if (handle !== null) timers.clear(handle);
    handle = timers.set(() => {
      handle = null;
      fn(...args);
    }, delayMs);
  };
  debounced.cancel = () => {
    if (handle !== null) timers.clear(handle);
    handle = null;
  };
  return debounced;
}

/* --- Saved projects ---------------------------------------------------- */

const PROJECTS_KEY = "depradar_saved_projects";

// The storage the app uses: the one installed at startup (see
// createRemoteStorage), or the webview's localStorage.
function defaultStorage() {
  return globalThis.__depradarStorage || localStorage;
}

// Parsing the whole list on every lookup is wasteful once reports are
// stored: remember the parse per storage and raw string.
const parseCache = new WeakMap();

function loadProjects(storage = defaultStorage()) {
  try {
    const raw = storage.getItem(PROJECTS_KEY) || "";
    const cached = parseCache.get(storage);
    if (cached && cached.raw === raw) return cached.list;

    const parsed = raw ? JSON.parse(raw) : [];
    const list = Array.isArray(parsed) ? parsed : [];
    // `scanning` is transient UI state: older versions persisted it, which
    // left a project stuck on "Scanning…" after an interrupted scan.
    for (const project of list) delete project.scanning;
    parseCache.set(storage, { raw, list });
    return list;
  } catch {
    return [];
  }
}

function persistProjects(list, storage = defaultStorage()) {
  const raw = JSON.stringify(list);
  storage.setItem(PROJECTS_KEY, raw);
  parseCache.set(storage, { raw, list });
}

function findProject(id, storage = defaultStorage()) {
  return loadProjects(storage).find((p) => p.id === id) || null;
}

function findExisting(type, value, storage = defaultStorage()) {
  return loadProjects(storage).find((p) => p.type === type && p.value === value) || null;
}

function upsertProject(project, storage = defaultStorage()) {
  const list = [...loadProjects(storage)];
  const idx = list.findIndex((p) => p.id === project.id);
  if (idx >= 0) {
    list[idx] = project;
  } else {
    list.push(project);
  }
  persistProjects(list, storage);
}

function removeProject(id, storage = defaultStorage()) {
  persistProjects(
    loadProjects(storage).filter((p) => p.id !== id),
    storage
  );
}

/* --- Scanning ----------------------------------------------------------- */

// Rust commands reject with a structured { code, detail } object (see
// AppError in src-tauri/src/error.rs); anything else is an unknown error.
function normalizeError(err) {
  return err && typeof err === "object" && err.code ? { code: err.code, detail: err.detail } : { code: "unknown" };
}

// Scans one project and stores the outcome. `scanning` is an in-memory Set of
// project ids (never persisted: see loadProjects). Re-reads the project after
// the scan, so one deleted meanwhile is not resurrected, and refuses to start
// a second scan of a project already being scanned.
//
// Returns "done", "busy" (already scanning), "missing" (no such project, or
// deleted during the scan) or "persist_failed" (scanned but could not be
// saved; the error is in `persistError`).
async function runScan(id, deps) {
  const { invoke, storage = defaultStorage(), scanning, onChange = () => {}, now = Date.now } = deps;

  const project = findProject(id, storage);
  if (!project) return { outcome: "missing" };
  if (scanning.has(id)) return { outcome: "busy" };

  scanning.add(id);
  onChange();

  // `scanId` lets the Rust side cancel this very scan (see cancelScan).
  const [command, args] =
    project.type === "local"
      ? ["scan_local", { path: project.value, scanId: id }]
      : ["scan_github", { repo: project.value, scanId: id }];

  let report = null;
  let error = null;
  try {
    report = await invoke(command, args);
  } catch (err) {
    error = normalizeError(err);
  }
  scanning.delete(id);

  const current = findProject(id, storage);
  if (!current) {
    onChange();
    return { outcome: "missing" };
  }
  if (report) {
    current.lastReport = report;
    current.lastError = null;
    current.lastScannedAt = now();
  } else if (error.code !== "scan_cancelled") {
    // A cancelled scan leaves the previous result untouched.
    current.lastError = error;
  }

  let result = { outcome: "done" };
  try {
    upsertProject(current, storage);
  } catch (persistError) {
    result = { outcome: "persist_failed", persistError };
  }
  onChange();
  return result;
}

// Asks the Rust side to abort the scan of project `id`.
function cancelScan(id, { invoke }) {
  return invoke("cancel_scan", { scanId: id });
}

// Rescans every saved project, one at a time. Sequential, not parallel: a
// GitHub-type project already runs several requests of its own.
async function scanAll(deps, onProgress = () => {}) {
  const storage = deps.storage || defaultStorage();
  const ids = loadProjects(storage).map((p) => p.id);
  for (let i = 0; i < ids.length; i++) {
    onProgress(i + 1, ids.length);
    await runScan(ids[i], deps);
  }
  return ids.length;
}

/* --- Storage in a file owned by the Rust side --------------------------- */

// The saved projects (with their last reports) can easily exceed the ~5 MB
// localStorage quota, so they live in a file in the app's data directory
// (Rust commands `load_state` / `save_state`). The storage returned here is
// synchronous for the rest of the code (reads/writes an in-memory copy) and
// writes through to the file after a short debounce. Projects saved by older
// versions in localStorage are migrated on first run.
async function createRemoteStorage({ invoke, legacy = null, onError = () => {}, delayMs = 300, timers }) {
  let raw = "";
  try {
    raw = (await invoke("load_state")) || "";
  } catch (err) {
    onError(err);
  }

  let migrated = false;
  if (!raw && legacy) {
    try {
      raw = legacy.getItem(PROJECTS_KEY) || "";
      migrated = !!raw;
    } catch {
      raw = "";
    }
  }

  let pending = false;
  const write = async () => {
    pending = false;
    try {
      await invoke("save_state", { json: raw });
      if (migrated) {
        migrated = false;
        try {
          legacy.removeItem(PROJECTS_KEY);
        } catch {
          // Leaving the old copy behind is harmless.
        }
      }
    } catch (err) {
      onError(err);
    }
  };
  const schedule = debounce(write, delayMs, timers);

  const storage = {
    getItem: (key) => (key === PROJECTS_KEY ? raw : null),
    setItem: (key, value) => {
      if (key !== PROJECTS_KEY) return;
      raw = value;
      pending = true;
      schedule();
    },
  };
  const flush = async () => {
    schedule.cancel();
    if (pending || migrated) await write();
  };

  if (migrated) await flush();
  return { storage, flush };
}

if (typeof module !== "undefined" && module.exports) {
  module.exports = {
    STATUS_SORT_ORDER,
    DEFAULT_REPORT_LABELS,
    PROJECTS_KEY,
    generateId,
    filterAndSortDeps,
    formatReportMarkdown,
    reportFileName,
    sortIndicator,
    projectDisplayName,
    projectHealthBadge,
    clampSidebarWidth,
    debounce,
    loadProjects,
    persistProjects,
    findProject,
    findExisting,
    upsertProject,
    removeProject,
    normalizeError,
    runScan,
    cancelScan,
    scanAll,
    createRemoteStorage,
  };
}
