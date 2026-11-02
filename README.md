# DepRadar

Detects outdated dependencies in a Rust, JavaScript, Python, Go, PHP, Ruby, and/or Java project — locally or on a GitHub repository.

**[depradar-app.github.io/depradar](https://depradar-app.github.io/depradar/)**

- **Rust**: `Cargo.toml` (including `[target.*]` tables, renamed crates and `{ workspace = true }` members) → [crates.io](https://crates.io) sparse index
- **JavaScript**: `package.json` (dependencies, dev, optional and peer) → [registry.npmjs.org](https://registry.npmjs.org)
- **Python**: `requirements.txt` (incl. `pip-compile` hash files) / `pyproject.toml` (PEP 621 + optional dependencies + PEP 735 groups, and Poetry incl. groups) → [PyPI](https://pypi.org)
- **Go**: `go.mod` (with `replace` directives and newer major versions) → [proxy.golang.org](https://proxy.golang.org)
- **PHP**: `composer.json` → [Packagist](https://packagist.org)
- **Ruby**: `Gemfile` → [RubyGems](https://rubygems.org)
- **Java/Kotlin**: `pom.xml` (parent POMs, `<dependencyManagement>`, imported BOMs), `build.gradle(.kts)` with literal coordinates, and Gradle version catalogs (`libs.versions.toml`) → [Maven Central](https://repo1.maven.org/maven2)

A single project can mix several ecosystems (a mono-repo): every manifest found is analyzed.

Installed versions are read from the lockfile next to a manifest — or, for workspaces (Cargo, npm/yarn/pnpm/bun, uv), from the one at the workspace root: `Cargo.lock`, `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `bun.lock`, `poetry.lock`, `uv.lock`, `pdm.lock`, `composer.lock`, `Gemfile.lock`.

Every dependency with a version really in use is also checked against [OSV.dev](https://osv.dev) for known vulnerabilities (GHSA/CVE): the version pinned in a lockfile, or an exact pin written in the manifest itself (Go modules, Maven versions, Python `==x.y.z`). Affected packages get a badge listing every advisory with a link, and a "Vulnerable" filter isolates them. When a registry or OSV can't be reached, the scan says so in its warnings (and each affected package shows why it is "Unknown") instead of silently reporting nothing.

## Install

One command for your platform, or pick a package from the [latest release](https://github.com/depradar-app/depradar/releases/latest).

**macOS / Linux**

```bash
curl -fsSL https://depradar-app.github.io/depradar/install.sh | sh
```

**Windows** (PowerShell)

```powershell
irm https://depradar-app.github.io/depradar/install.ps1 | iex
```

The install scripts check the download against the release's `SHA256SUMS` file before installing it (set `GITHUB_TOKEN` if the GitHub API rate-limits you).

| Platform | Command |
|---|---|
| macOS (Homebrew) | `brew install depradar-app/tap/depradar` |
| Debian / Ubuntu (x86_64, arm64) | `sudo apt install ./depradar_*.deb` |
| Linux (AppImage) | `chmod +x depradar_*.AppImage && ./depradar_*.AppImage` |
| Windows | run the `.exe` installer and follow the setup wizard |

## Uninstall

**macOS / Linux**

```bash
curl -fsSL https://depradar-app.github.io/depradar/uninstall.sh | sh
```

**Windows** (PowerShell)

```powershell
irm https://depradar-app.github.io/depradar/uninstall.ps1 | iex
```

| Platform | Command |
|---|---|
| macOS (Homebrew) | `brew uninstall --cask depradar-app/tap/depradar` |
| macOS (dragged from the .dmg) | move `DepRadar.app` to the Trash |
| Debian / Ubuntu | `sudo apt remove depradar` |
| Linux (AppImage) | delete the `.AppImage` file |
| Windows | Settings → Apps → DepRadar → Uninstall |

The uninstall scripts also remove the data DepRadar keeps (saved projects, webview cache). By hand, that is `~/.local/share/com.sikander.depradar` (Linux), `~/Library/Application Support/com.sikander.depradar` (macOS) or `%APPDATA%\com.sikander.depradar` (Windows).

## Usage

- **Local project**: point at a folder (or use "Browse…"). Scans recursively up to 8 path levels deep, ignoring `node_modules`, `target`, `.git`, `venv`, `vendor`, and similar directories (a folder you pick is always scanned, even if its own name is one of those).
- **GitHub repository**: `owner/repo`, an https/ssh URL, or a URL copied from the browser (`…/tree/main/src` works). Scans the whole repository (same depth limit), including nested folders, via GitHub's API and `raw.githubusercontent.com` — no cloning involved, and files over 1 MB (large lockfiles) are read correctly. A personal token (optional, set in the add-project form) raises the GitHub API rate limit (60/h → 5000/h) and gives access to private repositories. It is stored in the system keychain and only ever read by the Rust side: the interface can say whether one is saved, replace it or remove it, never read it back.
- **Scan all** (sidebar) rescans every saved project, one at a time. While a project is being scanned, its refresh button becomes a cancel button; a scan that takes more than 10 minutes is stopped.
- **Export** (⬇ in a project's header) saves the current scan as a Markdown report (in the interface's language) through a native save dialog.
- Saved projects and their last reports are kept in a file in the app's data directory (not in the webview's 5 MB `localStorage`); projects saved by older versions are migrated automatically.

Each dependency is classified as:
- **Up to date** — the declared constraint already admits the latest published version (or, when a lockfile is found, what's actually installed matches it).
- **Outdated** — a newer minor/patch version is available and isn't yet admitted by the declared constraint (or installed version).
- **Breaking** — a newer major version is available (or a minor bump on a `0.x` line, considered breaking by semver convention).
- **Unknown** — the constraint can't be resolved to a concrete version (a local/git dependency, `*`, etc.) or the package can't be found on the registry.

The UI is available in English (default), French, Spanish, German, Portuguese, Italian, Simplified Chinese, and Japanese — switch languages from the selector at the bottom of the sidebar.

## Platforms

macOS (universal: Apple Silicon + Intel), Linux (x86_64 and arm64, built on Ubuntu 22.04 for wide glibc compatibility) and Windows (a Tauri application). Installers are built automatically by CI on every `v*` tag (see `.github/workflows/release.yml`). The app can also check for and install updates itself from the sidebar's "Check for update" button (it asks for confirmation first, since installing restarts the app).

### Signing the macOS build

Without an Apple Developer certificate the macOS app is unsigned and Gatekeeper warns on first launch. To sign and notarize it, add these repository secrets (the release workflow already passes them to the build): `APPLE_CERTIFICATE` (base64 of the `.p12` Developer ID Application certificate), `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password) and `APPLE_TEAM_ID`.

## Development

```bash
npm install
npm run tauri dev   # runs the app from source, without a prebuilt install
```

```bash
cd src-tauri
cargo check          # compile
cargo fmt -- --check # formatting
cargo clippy --all-targets -- -D warnings   # lints
cargo test           # unit + mocked-HTTP tests (fast, no network)
cargo llvm-cov --ignore-filename-regex '(lib|main)\.rs$' --fail-under-lines 90   # coverage
cargo test -- --ignored --nocapture   # + real integration tests (network)
```

(`libdbus-1-dev` is needed on Linux for the keychain support.)

```bash
npm run lint            # ESLint
npm test                # logic, i18n, the whole UI in jsdom, the landing page
npm run test:coverage   # same, fails under 90% of lines
```

CI (`.github/workflows/ci.yml`): lint, tests and coverage gates (JS and Rust, 90% of lines minimum), clippy/rustfmt, `npm audit` / `cargo audit`, and builds on ubuntu/macos/windows on every push/PR. A weekly workflow (`live-tests.yml`) runs the real-network integration tests to catch registry API changes.

## Known limitations

- GitHub scanning can miss manifests in extremely large repositories: the Git Trees API truncates its response past a certain repo size. This is surfaced as a warning (or as an error if no manifest was found in the part returned) rather than failing silently.
- `requirements.txt` has no universal lockfile format: a version is only known (and vulnerability-checked) for exact `==x.y.z` pins. `go.mod` and Maven have no lockfile either, but their declared version is the one in use, and is checked as such.
- Maven: dependencies whose version comes from a profile, or from a BOM that can't be fetched from Maven Central, are skipped rather than misreported. Gradle support is limited to literal coordinates (`"group:artifact:1.2.3"`, the map notation) and version catalogs: versions computed by Groovy/Kotlin code can't be read statically.
- Go: replaced modules are reported under the fork that is really built; modules replaced by a local directory are not checked.
- The npm lockfile formats supported are `package-lock.json` (lockfileVersion 2/3; a v1-only file is reported as an unsupported-format warning), `yarn.lock` (classic and berry), `pnpm-lock.yaml` and the text `bun.lock`.
- An Intel Mac can't use the v0.6.1 Homebrew build (Apple Silicon only); the next release is universal.
- Vulnerability checks only run against a version that is really in use (see above), never against the floor of a range.
