// The app's version lives in Cargo.toml (tauri.conf.json inherits it);
// package.json must not drift from it.
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.join(__dirname, "..");

test("package.json and Cargo.toml declare the same version", () => {
  const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
  const cargo = fs.readFileSync(path.join(root, "src-tauri", "Cargo.toml"), "utf8");
  const cargoVersion = /^version = "([^"]+)"/m.exec(cargo)[1];
  assert.equal(pkg.version, cargoVersion);
});

test("tauri.conf.json does not carry a third copy of the version", () => {
  const conf = JSON.parse(fs.readFileSync(path.join(root, "src-tauri", "tauri.conf.json"), "utf8"));
  assert.equal(conf.version, undefined);
});

test("the macOS bundle name matches the name used by the uninstall script", () => {
  const conf = JSON.parse(fs.readFileSync(path.join(root, "src-tauri", "tauri.conf.json"), "utf8"));
  const uninstall = fs.readFileSync(path.join(root, "docs", "uninstall.sh"), "utf8");
  assert.ok(uninstall.includes(`/Applications/${conf.productName}.app`));
});

// `tauri build` refuses to run when an npm plugin and its Rust crate differ in
// major.minor (this broke the first 1.0.0 release build).
test("npm Tauri packages match the Rust crates on major.minor", () => {
  const lock = fs.readFileSync(path.join(root, "src-tauri", "Cargo.lock"), "utf8");
  const crateVersion = (name) => new RegExp(`name = "${name}"\\nversion = "([^"]+)"`).exec(lock)[1];
  const npmVersion = (pkg) =>
    JSON.parse(fs.readFileSync(path.join(root, "node_modules", pkg, "package.json"), "utf8")).version;
  const majorMinor = (v) => v.split(".").slice(0, 2).join(".");
  assert.equal(majorMinor(npmVersion("@tauri-apps/plugin-dialog")), majorMinor(crateVersion("tauri-plugin-dialog")));
  assert.equal(majorMinor(npmVersion("@tauri-apps/cli")), majorMinor(crateVersion("tauri")));
});
