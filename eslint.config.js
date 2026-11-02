import js from "@eslint/js";
import globals from "globals";

// logic.cjs and i18n.cjs are classic <script>s in the app and CommonJS
// modules in tests; main.js uses what they declare as globals.
const sharedFromScripts = Object.fromEntries(
  [
    "generateId",
    "filterAndSortDeps",
    "formatReportMarkdown",
    "reportFileName",
    "sortIndicator",
    "projectDisplayName",
    "projectHealthBadge",
    "clampSidebarWidth",
    "debounce",
    "loadProjects",
    "findProject",
    "findExisting",
    "upsertProject",
    "removeProject",
    "runScan",
    "cancelScan",
    "scanAll",
    "createRemoteStorage",
    "LOCALES",
    "t",
    "getLocale",
    "setLocale",
    "initLocale",
    "applyTranslations",
  ].map((name) => [name, "readonly"])
);

export default [
  { ignores: ["node_modules/**", "src-tauri/**", "coverage/**"] },
  js.configs.recommended,
  {
    files: ["src/main.js"],
    languageOptions: {
      sourceType: "script",
      globals: { ...globals.browser, ...sharedFromScripts },
    },
  },
  {
    files: ["src/logic.cjs", "src/i18n.cjs", "docs/landing.js"],
    languageOptions: {
      sourceType: "commonjs",
      globals: { ...globals.browser, ...globals.commonjs },
    },
  },
  {
    files: ["**/*.test.cjs"],
    languageOptions: {
      sourceType: "commonjs",
      globals: { ...globals.node, ...globals.browser },
    },
  },
  {
    files: ["eslint.config.js"],
    languageOptions: { sourceType: "module", globals: globals.node },
  },
];
