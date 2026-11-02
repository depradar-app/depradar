const { invoke } = window.__TAURI__.core;

// From logic.cjs: STATUS_SORT_ORDER, generateId(), filterAndSortDeps(),
// formatReportMarkdown(), reportFileName(), sortIndicator(),
// projectDisplayName(), projectHealthBadge(), clampSidebarWidth(),
// debounce(), the saved-project functions (loadProjects/findProject/
// findExisting/upsertProject/removeProject), runScan(), cancelScan(), scanAll() and
// createRemoteStorage(). From i18n.cjs: LOCALES, t(), getLocale(),
// setLocale(), initLocale() and applyTranslations(). Both are loaded before
// this script (see index.html).
// Everything shown is built with DOM nodes and textContent — never HTML
// strings — so data coming from scanned (untrusted) repositories can't inject
// markup.

const LEGACY_TOKEN_KEY = "depradar_github_token";
const THEME_KEY = "depradar_theme";
const SIDEBAR_WIDTH_KEY = "depradar_sidebar_width";

// Applied immediately (not deferred to DOMContentLoaded) since the DOM
// already exists by the time this `defer` script runs, and doing it as
// early as possible keeps the light/dark flash on launch to a minimum.
function applyTheme() {
  const theme = localStorage.getItem(THEME_KEY) || "system";
  if (theme === "light" || theme === "dark") {
    document.documentElement.dataset.theme = theme;
  } else {
    delete document.documentElement.dataset.theme;
  }
}
applyTheme();

function effectiveTheme() {
  const theme = localStorage.getItem(THEME_KEY);
  if (theme === "light" || theme === "dark") return theme;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

const STATUS_CLASS = { uptodate: "ok", outdated: "warn", major: "danger", unknown: "muted" };

let selectedProjectId = null;
let sortColumn = "status"; // "status" | "name"
let sortDirection = "asc";
let nameFilter = "";
let statusFilterSet = new Set();
let vulnOnly = false;
let appVersion = null;
let updateInProgress = false;
let tokenSet = false;
let scanAllProgress = null; // { current, total } while "Scan all" runs
const scanning = new Set(); // ids of projects being scanned (never persisted)

function $(selector) {
  return document.querySelector(selector);
}

// Tiny DOM builder: el("a", { href, class }, "text", childNode, ...).
function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === null || value === undefined || value === false) continue;
    if (key === "class") node.className = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else node.setAttribute(key, value === true ? "" : value);
  }
  for (const child of children.flat()) {
    if (child === null || child === undefined || child === false) continue;
    node.append(child.nodeType ? child : document.createTextNode(String(child)));
  }
  return node;
}

function statusLabel(status) {
  return { label: t(`status.${status}`), cls: STATUS_CLASS[status] || "muted" };
}

function manifestFilename(path) {
  return path.slice(path.lastIndexOf("/") + 1);
}

const ERROR_CODES = new Set(
  Object.keys(window.i18n.STRINGS.en)
    .filter((key) => key.startsWith("error."))
    .map((key) => key.slice("error.".length))
);

// Rust commands reject with a structured { code, detail } object (see
// AppError in src-tauri/src/error.rs) rather than a finished sentence, so
// the message can be resolved in whichever language is currently selected.
function errorMessage(err) {
  if (err && typeof err === "object" && ERROR_CODES.has(err.code)) {
    return t(`error.${err.code}`, { detail: err.detail });
  }
  return t("error.unknown");
}

// Non-fatal scan warnings come as { code, path, detail } as well.
function warningMessage(warning) {
  const key = `warning.${warning.code}`;
  if (key in window.i18n.STRINGS.en) {
    return t(key, { path: warning.path, detail: warning.detail });
  }
  return [warning.code, warning.path, warning.detail].filter(Boolean).join(": ");
}

function formatDate(timestamp) {
  return new Date(timestamp).toLocaleString(getLocale());
}

/* --- App-wide notice --- */

function setNotice(message, kind) {
  const box = $("#app-notice");
  box.textContent = message || "";
  box.className = `app-notice ${kind || ""}${message ? "" : " hidden"}`;
}

/* --- Modal stack: focus trapping, Escape, focus restoration --- */

const modalStack = [];

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), summary, [tabindex]:not([tabindex="-1"])';

function showModal(overlay, onClose, initialFocus) {
  modalStack.push({ overlay, onClose, returnFocus: document.activeElement });
  overlay.classList.remove("hidden");
  const target = initialFocus || overlay.querySelector(FOCUSABLE);
  if (target) target.focus();
}

function hideModal(overlay) {
  const index = modalStack.findIndex((m) => m.overlay === overlay);
  overlay.classList.add("hidden");
  if (index === -1) return;
  const [{ returnFocus }] = modalStack.splice(index, 1);
  if (returnFocus && returnFocus.focus) returnFocus.focus();
}

function initModalKeys() {
  document.addEventListener("keydown", (e) => {
    const top = modalStack[modalStack.length - 1];
    if (!top) return;
    if (e.key === "Escape") {
      top.onClose();
      return;
    }
    if (e.key !== "Tab") return;
    const items = [...top.overlay.querySelectorAll(FOCUSABLE)].filter((n) => n.offsetParent !== null);
    if (!items.length) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  });
}

/* --- Sidebar --- */

function setSidebarWidth(width) {
  const clamped = clampSidebarWidth(width, window.innerWidth);
  document.documentElement.style.setProperty("--sidebar-width", `${clamped}px`);
  const resizer = $("#sidebar-resizer");
  resizer.setAttribute("aria-valuenow", String(clamped));
  return clamped;
}

function initSidebarResizer() {
  const resizer = $("#sidebar-resizer");
  resizer.setAttribute("aria-valuemin", "200");
  resizer.setAttribute("aria-valuemax", "600");
  const saved = Number(localStorage.getItem(SIDEBAR_WIDTH_KEY));
  let current = setSidebarWidth(saved || 260);

  const save = (width) => {
    current = setSidebarWidth(width);
    localStorage.setItem(SIDEBAR_WIDTH_KEY, String(current));
  };

  resizer.addEventListener("pointerdown", (e) => {
    e.preventDefault();
    resizer.setPointerCapture(e.pointerId);
    resizer.classList.add("dragging");
  });
  resizer.addEventListener("pointermove", (e) => {
    if (resizer.classList.contains("dragging")) current = setSidebarWidth(e.clientX);
  });
  const stopDragging = () => {
    if (!resizer.classList.contains("dragging")) return;
    resizer.classList.remove("dragging");
    save(current);
  };
  resizer.addEventListener("pointerup", stopDragging);
  resizer.addEventListener("pointercancel", stopDragging);
  resizer.addEventListener("lostpointercapture", stopDragging);

  resizer.addEventListener("keydown", (e) => {
    const step = e.shiftKey ? 48 : 16;
    if (e.key === "ArrowLeft") save(current - step);
    else if (e.key === "ArrowRight") save(current + step);
    else if (e.key === "Home") save(0);
    else if (e.key === "End") save(Infinity);
    else return;
    e.preventDefault();
  });

  window.addEventListener("resize", () => {
    current = setSidebarWidth(current);
  });
}

function initThemeToggle() {
  const btn = $("#theme-btn");
  const render = () => {
    btn.textContent = effectiveTheme() === "dark" ? "☀" : "☾";
  };
  btn.addEventListener("click", () => {
    localStorage.setItem(THEME_KEY, effectiveTheme() === "dark" ? "light" : "dark");
    applyTheme();
    render();
  });
  // Follow the OS when no explicit choice has been made.
  window
    .matchMedia("(prefers-color-scheme: dark)")
    .addEventListener("change", () => render());
  render();
}

function renderSidebar() {
  const container = $("#project-list");
  container.replaceChildren();

  for (const project of loadProjects()) {
    const health = projectHealthBadge(project, (status) => t(`status.${status}`));
    const item = el(
      "button",
      {
        type: "button",
        class: "project-item" + (project.id === selectedProjectId ? " active" : ""),
        onclick: () => selectProject(project.id),
      },
      el("span", { class: `type-badge ${project.type}` }, t(project.type === "local" ? "sidebar.typeLocal" : "sidebar.typeGithub")),
      el("span", { class: "project-item-label", title: project.value }, projectDisplayName(project.value)),
      health ? el("span", { class: `health-badge ${health.cls}`, title: health.title }, health.count) : null
    );
    container.append(item);
  }
}

// The webview's native menu (Back/Forward/Stop/Reload) means nothing here.
function initProjectListContextMenu() {
  $("#project-list").addEventListener("contextmenu", (e) => e.preventDefault());
}

/* --- Main content --- */

function selectProject(id) {
  selectedProjectId = id;
  resetFilters();
  renderSidebar();
  renderMain();
}

function resetFilters() {
  sortColumn = "status";
  sortDirection = "asc";
  nameFilter = "";
  statusFilterSet = new Set();
  vulnOnly = false;
  $("#search-input").value = "";
  document.querySelectorAll(".filter-pill").forEach((p) => p.classList.remove("active"));
}

function setStatus(message, kind) {
  const box = $("#status");
  if (!message) {
    box.classList.add("hidden");
    box.textContent = "";
    return;
  }
  box.className = `status ${kind || ""}`;
  box.textContent = message;
}

function renderMain() {
  const project = selectedProjectId ? findProject(selectedProjectId) : null;

  $("#empty-state").classList.toggle("hidden", !!project);
  $("#project-view").classList.toggle("hidden", !project);

  if (!project) {
    return;
  }

  $("#project-badge").textContent = t(project.type === "local" ? "sidebar.typeLocal" : "sidebar.typeGithub");
  $("#project-badge").className = `saved-badge ${project.type}`;
  $("#project-title").textContent = project.value;

  if (scanning.has(project.id)) {
    setStatus(t("status.scanning"));
  } else if (project.lastError) {
    setStatus(errorMessage(project.lastError), "danger");
  } else if (project.lastScannedAt) {
    setStatus(t("status.lastScanned", { date: formatDate(project.lastScannedAt) }), "ok");
  } else {
    setStatus("");
  }
  // While a scan runs, the refresh button becomes a cancel button.
  const isScanning = scanning.has(project.id);
  const refresh = $("#refresh-btn");
  refresh.textContent = isScanning ? "✕" : "↻";
  refresh.title = t(isScanning ? "project.cancelButton" : "project.refreshButton");
  refresh.dataset.i18nTitle = isScanning ? "project.cancelButton" : "project.refreshButton";

  const hasResults = !!(project.lastReport && project.lastReport.manifests.length);
  $("#filters-bar").classList.toggle("hidden", !hasResults);

  renderResults(project);
}

function vulnerabilityBlock(dep) {
  const ids = dep.vulnerabilities || [];
  if (!ids.length) return null;
  const basis = t(dep.vuln_basis === "declared" ? "vuln.basisDeclared" : "vuln.basisInstalled");
  return el(
    "details",
    { class: "vuln-details" },
    el("summary", { class: "pill danger vuln-badge", title: basis }, t("vuln.badge", { count: ids.length })),
    el(
      "ul",
      { class: "vuln-list", "aria-label": t("vuln.listTitle") },
      ids.map((id) =>
        el(
          "li",
          {},
          el(
            "a",
            {
              href: `https://osv.dev/vulnerability/${encodeURIComponent(id)}`,
              target: "_blank",
              rel: "noopener noreferrer",
            },
            id
          )
        )
      )
    )
  );
}

function dependencyRow(dep) {
  const info = statusLabel(dep.status);
  const reasonText = dep.reason ? t(`reason.${dep.reason}`) : null;
  return el(
    "tr",
    {},
    el(
      "td",
      {},
      dep.name,
      dep.package ? el("span", { class: "alias", title: dep.package }, ` → ${dep.package}`) : null,
      vulnerabilityBlock(dep)
    ),
    el("td", { class: "mono" }, dep.declared),
    el("td", { class: "mono" }, dep.installed || "—"),
    el("td", { class: "mono" }, dep.latest || "—"),
    el("td", {}, el("span", { class: `pill ${info.cls}`, title: reasonText }, info.label))
  );
}

function manifestCard(manifest) {
  const card = el(
    "article",
    { class: "manifest-card" },
    el(
      "h2",
      {},
      manifestFilename(manifest.manifest_path),
      " ",
      el("span", { class: "ecosystem-badge" }, t(`ecosystem.${manifest.ecosystem}`)),
      " ",
      el("span", { class: "manifest-path" }, manifest.manifest_path)
    )
  );

  const visible = filterAndSortDeps(manifest.dependencies, {
    nameFilter,
    statusFilterSet,
    vulnOnly,
    sortColumn,
    sortDirection,
  });

  if (!visible.length) {
    card.append(el("p", { class: "empty" }, t("results.noMatches")));
    return card;
  }

  const sortableHeader = (column, label) =>
    el(
      "th",
      {
        class: "sortable",
        tabindex: "0",
        onclick: () => toggleSort(column),
        onkeydown: (e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            toggleSort(column);
          }
        },
      },
      label + sortIndicator(column, sortColumn, sortDirection)
    );

  card.append(
    el(
      "table",
      {},
      el(
        "thead",
        {},
        el(
          "tr",
          {},
          sortableHeader("name", t("table.package")),
          el("th", {}, t("table.declared")),
          el("th", {}, t("table.installed")),
          el("th", {}, t("table.latest")),
          sortableHeader("status", t("table.status"))
        )
      ),
      el("tbody", {}, visible.map(dependencyRow))
    )
  );
  return card;
}

function toggleSort(column) {
  if (sortColumn === column) {
    sortDirection = sortDirection === "asc" ? "desc" : "asc";
  } else {
    sortColumn = column;
    sortDirection = "asc";
  }
  const project = selectedProjectId ? findProject(selectedProjectId) : null;
  if (project) renderResults(project);
}

function renderResults(project) {
  const container = $("#results");
  container.replaceChildren();
  const report = project.lastReport;
  if (!report) return;

  if (report.warnings && report.warnings.length) {
    container.append(el("div", { class: "warnings" }, report.warnings.map((w) => el("p", {}, warningMessage(w)))));
  }

  if (!report.manifests.length) {
    container.append(el("p", { class: "empty" }, t("results.noManifests")));
    return;
  }

  for (const manifest of report.manifests) {
    container.append(manifestCard(manifest));
  }
}

/* --- Scanning --- */

const scanDeps = () => ({
  invoke,
  scanning,
  onChange: () => {
    renderSidebar();
    renderScanState();
  },
});

// Re-renders the current project's header (status line, refresh button)
// without rebuilding the results table.
function renderScanState() {
  const project = selectedProjectId ? findProject(selectedProjectId) : null;
  if (project) renderMain();
}

async function scanProject(id) {
  const { outcome, persistError } = await runScan(id, scanDeps());
  if (outcome === "persist_failed") {
    setNotice(errorMessage({ code: "state_io_failed", detail: String(persistError) }), "danger");
  }
}

function renderScanAllButton() {
  const btn = $("#scan-all-btn");
  btn.disabled = scanAllProgress !== null;
  btn.textContent = scanAllProgress
    ? t("sidebar.scanningAll", scanAllProgress)
    : t("sidebar.scanAll");
}

async function scanAllProjects() {
  if (scanAllProgress !== null || !loadProjects().length) return;
  scanAllProgress = { current: 0, total: loadProjects().length };
  renderScanAllButton();
  try {
    await scanAll(scanDeps(), (current, total) => {
      scanAllProgress = { current, total };
      renderScanAllButton();
    });
  } finally {
    scanAllProgress = null;
    renderScanAllButton();
  }
}

function initScanAll() {
  $("#scan-all-btn").addEventListener("click", scanAllProjects);
}

function addProject(type, value) {
  let project = findExisting(type, value);
  if (!project) {
    project = {
      id: generateId(),
      type,
      value,
      lastReport: null,
      lastError: null,
      lastScannedAt: null,
    };
    upsertProject(project);
  }
  closeAddModal();
  selectProject(project.id);
  scanProject(project.id);
}

/* --- GitHub token (kept by the Rust side, in the OS keychain) --- */

async function refreshTokenStatus() {
  try {
    tokenSet = await invoke("github_token_status");
  } catch {
    tokenSet = false;
  }
  renderTokenField();
}

function renderTokenField() {
  $("#github-token").placeholder = t(tokenSet ? "form.github.tokenSetPlaceholder" : "form.github.tokenPlaceholder");
  $("#github-token-remove").classList.toggle("hidden", !tokenSet);
}

// Saves (or, if empty, deletes) the token. A keychain failure is not fatal:
// the Rust side keeps the token for the session and the user is told.
async function saveToken(token) {
  try {
    await invoke("save_github_token", { token });
  } catch (err) {
    setNotice(errorMessage(err), "warn");
  }
  await refreshTokenStatus();
}

// Older versions kept the token in localStorage in clear text.
async function migrateLegacyToken() {
  const legacy = localStorage.getItem(LEGACY_TOKEN_KEY);
  if (legacy === null) return;
  if (legacy) await saveToken(legacy);
  localStorage.removeItem(LEGACY_TOKEN_KEY);
}

/* --- Add-project modal --- */

function activateTab(tab) {
  document.querySelectorAll(".tab-btn").forEach((b) => {
    const active = b.dataset.tab === tab;
    b.classList.toggle("active", active);
    b.setAttribute("aria-selected", String(active));
  });
  document.querySelectorAll(".tab-panel").forEach((panel) => {
    panel.classList.toggle("hidden", panel.dataset.panel !== tab);
  });
}

function openAddModal() {
  activateTab("local");
  $("#local-path").value = "";
  $("#github-repo").value = "";
  $("#github-token").value = "";
  renderTokenField();
  showModal($("#add-modal"), closeAddModal, $("#local-path"));
}

function closeAddModal() {
  hideModal($("#add-modal"));
}

function initAddModal() {
  $("#add-project-btn").addEventListener("click", openAddModal);
  $("#add-project-btn-main").addEventListener("click", openAddModal);
  $("#close-modal-btn").addEventListener("click", closeAddModal);

  $("#add-modal").addEventListener("click", (e) => {
    if (e.target.id === "add-modal") closeAddModal();
  });

  document.querySelectorAll(".tab-btn").forEach((btn) => {
    btn.addEventListener("click", () => activateTab(btn.dataset.tab));
  });
}

/* --- Confirm modal ---
 * window.confirm() isn't reliable in Tauri's webview (resolves silently,
 * with no dialog shown). We use a homemade popin instead. */

let confirmCallback = null;

function openConfirmModal({ message, title, okLabel, onConfirm }) {
  const modal = $("#confirm-modal");
  if (title) $("#confirm-modal-title").textContent = title;
  if (okLabel) $("#confirm-ok-btn").textContent = okLabel;
  $("#confirm-modal-message").textContent = message;
  confirmCallback = onConfirm;
  showModal(modal, closeConfirmModal, $("#confirm-cancel-btn"));
}

function closeConfirmModal(confirmed = false) {
  const callback = confirmed === true ? confirmCallback : null;
  const cancelCallback = confirmed === true ? null : confirmCancelCallback;
  confirmCallback = null;
  confirmCancelCallback = null;
  hideModal($("#confirm-modal"));
  // Restore the default (deletion) texts.
  applyTranslations($("#confirm-modal"));
  if (callback) callback();
  if (cancelCallback) cancelCallback();
}

let confirmCancelCallback = null;

function initConfirmModal() {
  $("#confirm-cancel-btn").addEventListener("click", () => closeConfirmModal());
  $("#confirm-modal-close-btn").addEventListener("click", () => closeConfirmModal());
  $("#confirm-ok-btn").addEventListener("click", () => closeConfirmModal(true));
  $("#confirm-modal").addEventListener("click", (e) => {
    if (e.target.id === "confirm-modal") closeConfirmModal();
  });
}

/* --- About modal --- */

function closeAboutModal() {
  hideModal($("#about-modal"));
}

function initAboutModal() {
  $("#app-version-btn").addEventListener("click", () => {
    $("#about-version").textContent = t("about.version", { version: appVersion || "…" });
    showModal($("#about-modal"), closeAboutModal, $("#about-modal-close-btn"));
  });
  $("#about-modal-close-btn").addEventListener("click", closeAboutModal);
  $("#about-modal").addEventListener("click", (e) => {
    if (e.target.id === "about-modal") closeAboutModal();
  });
}

/* --- Forms --- */

function initForms() {
  $("#local-form").addEventListener("submit", (e) => {
    e.preventDefault();
    const path = $("#local-path").value.trim();
    if (!path) return;
    addProject("local", path);
  });

  $("#browse-btn").addEventListener("click", async () => {
    try {
      const selected = await window.__TAURI__.dialog.open({ directory: true, multiple: false });
      if (selected) {
        $("#local-path").value = selected;
      }
    } catch {
      // Folder picker unavailable; the user can still type the path manually.
    }
  });

  $("#github-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const repo = $("#github-repo").value.trim();
    const token = $("#github-token").value.trim();
    if (!repo) return;
    // An empty field keeps the token that is already saved.
    if (token) await saveToken(token);
    addProject("github", repo);
  });

  $("#github-token-remove").addEventListener("click", async () => {
    await saveToken("");
    $("#github-token").value = "";
  });
}

/* --- Project view actions --- */

async function exportReport() {
  const project = selectedProjectId ? findProject(selectedProjectId) : null;
  if (!project || !project.lastReport) return;

  const labels = {
    title: (name) => t("report.title", { name }),
    source: t("report.source"),
    generated: t("report.generated"),
    sourceLocal: t("sidebar.typeLocal"),
    sourceGithub: t("sidebar.typeGithub"),
    columns: {
      package: t("table.package"),
      declared: t("table.declared"),
      installed: t("table.installed"),
      latest: t("table.latest"),
      status: t("table.status"),
      vulnerabilities: t("table.vulnerabilities"),
    },
    status: {
      uptodate: t("status.uptodate"),
      outdated: t("status.outdated"),
      major: t("status.major"),
      unknown: t("status.unknown"),
    },
    warning: warningMessage,
  };
  const content = formatReportMarkdown(project, project.lastReport, labels);

  try {
    // The Rust side shows the save dialog and writes the file: the webview
    // never chooses (or learns) the destination path.
    await invoke("export_report", { content, defaultName: reportFileName(project) });
  } catch (err) {
    setStatus(errorMessage(err), "danger");
  }
}

function initProjectView() {
  $("#refresh-btn").addEventListener("click", () => {
    if (!selectedProjectId) return;
    if (scanning.has(selectedProjectId)) {
      cancelScan(selectedProjectId, { invoke });
    } else {
      scanProject(selectedProjectId);
    }
  });

  $("#export-btn").addEventListener("click", exportReport);

  $("#delete-project-btn").addEventListener("click", () => {
    if (!selectedProjectId) return;
    const project = findProject(selectedProjectId);
    openConfirmModal({
      message: t("confirm.deleteMessage", { name: project ? project.value : "" }),
      onConfirm: () => {
        removeProject(selectedProjectId);
        selectedProjectId = null;
        renderSidebar();
        renderMain();
      },
    });
  });
}

/* --- Filters --- */

function rerenderResults() {
  const project = selectedProjectId ? findProject(selectedProjectId) : null;
  if (project) renderResults(project);
}

function initFilters() {
  const applySearch = debounce(rerenderResults, 150);
  $("#search-input").addEventListener("input", (e) => {
    nameFilter = e.target.value.trim();
    applySearch();
  });

  document.querySelectorAll(".filter-pill[data-status]").forEach((pill) => {
    pill.addEventListener("click", () => {
      const status = pill.dataset.status;
      if (statusFilterSet.has(status)) {
        statusFilterSet.delete(status);
        pill.classList.remove("active");
      } else {
        statusFilterSet.add(status);
        pill.classList.add("active");
      }
      rerenderResults();
    });
  });

  $("#vuln-filter-pill").addEventListener("click", () => {
    vulnOnly = !vulnOnly;
    $("#vuln-filter-pill").classList.toggle("active", vulnOnly);
    rerenderResults();
  });
}

/* --- Language switcher + app version --- */

function initSidebarFooter() {
  const select = $("#lang-select");
  for (const { code, name } of LOCALES) {
    const opt = document.createElement("option");
    opt.value = code;
    opt.textContent = name;
    select.append(opt);
  }
  select.value = getLocale();
  select.addEventListener("change", () => setLocale(select.value));

  window.__TAURI__.app
    .getVersion()
    .then((version) => {
      appVersion = version;
      $("#app-version-btn").textContent = `v${version}`;
    })
    .catch(() => {
      $("#app-version-btn").textContent = "";
    });
}

/* --- Auto-update --- */

function setUpdateStatus(message, kind) {
  const box = $("#update-status");
  if (!message) {
    box.classList.add("hidden");
    box.textContent = "";
    return;
  }
  box.className = `update-status ${kind || ""}`;
  box.textContent = message;
}

async function installUpdate(update) {
  const btn = $("#check-update-btn");
  btn.disabled = true;
  try {
    setUpdateStatus(t("update.installing", { version: update.version }));
    await update.downloadAndInstall();
    setUpdateStatus(t("update.relaunching"));
    await window.__TAURI__.process.relaunch();
  } catch {
    setUpdateStatus(t("update.error"), "danger");
  } finally {
    updateInProgress = false;
    btn.disabled = false;
  }
}

async function checkForUpdate() {
  if (updateInProgress) return;
  updateInProgress = true;
  const btn = $("#check-update-btn");
  btn.disabled = true;
  setUpdateStatus(t("update.checking"));

  let update = null;
  try {
    update = await window.__TAURI__.updater.check();
  } catch {
    setUpdateStatus(t("update.error"), "danger");
    updateInProgress = false;
    btn.disabled = false;
    return;
  }

  if (!update) {
    setUpdateStatus(t("update.upToDate"), "ok");
    setTimeout(() => setUpdateStatus(""), 4000);
    updateInProgress = false;
    btn.disabled = false;
    return;
  }

  // Installing restarts the app, interrupting any running scan: ask first.
  setUpdateStatus("");
  btn.disabled = false;
  confirmCancelCallback = () => {
    updateInProgress = false;
  };
  openConfirmModal({
    title: t("update.confirmTitle"),
    message: t("update.confirmMessage", { version: update.version }),
    okLabel: t("update.installButton"),
    onConfirm: () => installUpdate(update),
  });
}

function initUpdater() {
  $("#check-update-btn").addEventListener("click", checkForUpdate);
}

/* --- Locale changes --- */

function initI18n() {
  initLocale();
  applyTranslations(document);
  document.addEventListener("depradar:locale-changed", () => {
    applyTranslations(document);
    renderTokenField();
    renderScanAllButton();
    renderSidebar();
    renderMain();
  });
}

/* --- Startup --- */

// Saved projects live in a file managed by the Rust side (see
// createRemoteStorage); projects saved by older versions in localStorage are
// migrated on the first run.
async function initStorage() {
  const { storage, flush } = await createRemoteStorage({
    invoke,
    legacy: localStorage,
    onError: (err) => setNotice(errorMessage(err), "danger"),
  });
  globalThis.__depradarStorage = storage;
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") flush();
  });
  window.addEventListener("beforeunload", flush);
}

window.addEventListener("DOMContentLoaded", async () => {
  initI18n();
  initModalKeys();
  initAddModal();
  initConfirmModal();
  initAboutModal();
  initProjectListContextMenu();
  initThemeToggle();
  initSidebarResizer();
  initForms();
  initProjectView();
  initFilters();
  initSidebarFooter();
  initScanAll();
  initUpdater();

  await initStorage();
  await migrateLegacyToken();
  await refreshTokenStatus();
  renderSidebar();
  renderMain();
});
