const test = require("node:test");
const assert = require("node:assert/strict");
const { LOCALES, DEFAULT_LOCALE, STRINGS, t, getLocale, setLocale, initLocale, applyTranslations } = require("./i18n.cjs");

test("LOCALES includes English as the default and 7 other locales", () => {
  assert.equal(DEFAULT_LOCALE, "en");
  assert.ok(LOCALES.some((l) => l.code === "en"));
  assert.equal(LOCALES.length, 8);
});

test("every locale dictionary has the exact same key set as English", () => {
  const englishKeys = Object.keys(STRINGS.en).sort();
  for (const { code } of LOCALES) {
    const keys = Object.keys(STRINGS[code]).sort();
    assert.deepEqual(keys, englishKeys, `locale "${code}" has a different key set than "en"`);
  }
});

test("t() returns the English string for the default locale", () => {
  assert.equal(t("table.package"), "Package");
});

test("t() falls back to the raw key when it doesn't exist anywhere", () => {
  assert.equal(t("this.key.does.not.exist"), "this.key.does.not.exist");
});

test("t() interpolates {{vars}} into the resolved string", () => {
  assert.equal(
    t("error.local_dir_not_found", { detail: "/tmp/missing" }),
    'Directory "/tmp/missing" not found.'
  );
});

test("t() without vars leaves a template with no placeholders untouched", () => {
  assert.equal(t("update.checking"), "Checking for updates…");
});

test("t() does not interpret replacement patterns in interpolated values", () => {
  assert.equal(t("error.network_error", { detail: "cost $& now $1" }), "Network error: cost $& now $1.");
});

test("every locale defines every warning and reason code the backend can emit", () => {
  const needed = [
    "warning.read_failed",
    "warning.lockfile_empty",
    "warning.lockfile_unparseable",
    "warning.lockfile_v1_unsupported",
    "warning.tree_truncated",
    "warning.osv_unavailable",
    "warning.registry_unreachable",
    "reason.non_registry",
    "reason.not_found",
    "reason.network_error",
    "reason.unparseable",
    "error.github_unauthorized",
    "error.github_forbidden",
    "error.github_tree_truncated",
    "error.token_storage_failed",
    "error.state_io_failed",
  ];
  for (const { code } of LOCALES) {
    for (const key of needed) assert.ok(STRINGS[code][key], `${code} is missing ${key}`);
  }
});

test("every translation keeps the same {{placeholders}} as English", () => {
  const placeholders = (s) => (s.match(/\{\{\w+\}\}/g) || []).sort().join(",");
  for (const { code } of LOCALES) {
    for (const [key, value] of Object.entries(STRINGS[code])) {
      assert.equal(placeholders(value), placeholders(STRINGS.en[key]), `${code}:${key}`);
    }
  }
});

test("setLocale ignores unknown codes and getLocale reflects the change", () => {
  setLocale("klingon");
  assert.equal(getLocale(), "en");
  setLocale("fr");
  assert.equal(getLocale(), "fr");
  assert.equal(t("status.uptodate"), "À jour");
  setLocale("en");
  assert.equal(getLocale(), "en");
});

test("initLocale and applyTranslations work against a minimal DOM", () => {
  const attrs = {};
  const makeEl = (dataset) => ({
    dataset,
    setAttribute: (k, v) => {
      attrs[k] = v;
    },
  });
  const nodes = {
    "[data-i18n]": [makeEl({ i18n: "status.major" })],
    "[data-i18n-placeholder]": [makeEl({ i18nPlaceholder: "filters.searchPlaceholder" })],
    "[data-i18n-title]": [makeEl({ i18nTitle: "project.refreshButton" })],
    "[data-i18n-aria-label]": [makeEl({ i18nAriaLabel: "sidebar.languageLabel" })],
  };
  const root = { querySelectorAll: (sel) => nodes[sel] };
  applyTranslations(root);
  assert.equal(nodes["[data-i18n]"][0].textContent, "Breaking");
  assert.equal(nodes["[data-i18n-placeholder]"][0].placeholder, "Search a package…");
  assert.equal(nodes["[data-i18n-title]"][0].title, "Refresh");
  assert.equal(attrs["aria-label"], "Language");

  global.document = { documentElement: {}, dispatchEvent: () => {} };
  global.CustomEvent = class {};
  initLocale();
  assert.equal(document.documentElement.lang, "en");
  setLocale("de");
  assert.equal(document.documentElement.lang, "de");
  setLocale("en");
  delete global.document;
  delete global.CustomEvent;
});
