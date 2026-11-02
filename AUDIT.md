# Audit — DepRadar (`depradar-app/depradar` + `depradar-app/homebrew-tap`)

Date : 2026-11-02 — Version auditée : **v0.6.1** (commit `930ec94`, tap `ff3c6d4`)

## 0. Périmètre et méthode

| Élément | Détail |
|---|---|
| Code lu intégralement | Backend Rust (`src-tauri/src/**`, ~3 300 lignes), frontend (`src/main.js`, `logic.cjs`, `i18n.cjs`, `index.html`), config Tauri, workflows CI/CD, scripts `docs/install*`/`uninstall*`, page `docs/index.html`, cask Homebrew |
| Tests existants exécutés | `npm test` → **35/35 OK** · `cargo test` → **101 OK, 10 ignorés** (tests réseau) · `cargo clippy --all-targets` → **0 warning** · `cargo fmt --check` → **OK** |
| Sondes ajoutées (hors dépôt) | Un test Rust temporaire, lancé dans une copie isolée du crate, qui exécute le code actuel sur les cas suspects. Les résultats sont cités tels quels plus bas (« Preuve ») |
| Vérifications en direct | API GitHub (fichier > 1 Mo), OSV.dev (Go, Maven, Ruby, PyPI), Maven Central (Solr vs `maven-metadata.xml`), Packagist, assets de la release v0.6.1, bundle macOS |

Niveaux de gravité : **Critique** (résultat faux et dangereux pour l'utilisateur) · **Majeur** (résultat faux ou fonction cassée dans un cas courant) · **Moyen** · **Mineur**.

---

## 1. Synthèse

Le code est propre, bien commenté et bien testé unitairement (clippy et rustfmt passent sans rien signaler). Les problèmes viennent surtout de cas réels mal couverts : il y a beaucoup de **faux négatifs silencieux**, c'est-à-dire des cas où l'outil dit « rien à signaler » alors qu'il n'a pas pu vérifier. Pour un outil de sécurité et de veille de dépendances, c'est la catégorie de défaut la plus grave.

| # | Gravité | Sujet | Réf. |
|---|---|---|---|
| B1 | **Critique** | Go et Maven ne sont **jamais** vérifiés côté vulnérabilités (ex. log4j 2.14.1 / Log4Shell non signalé) | §3 |
| B2 | **Critique** | Échec OSV / registre silencieux : affiche « aucune vulnérabilité » ou « Unknown » sans avertir | §3 |
| B3 | **Majeur** | Lockfile > 1 Mo sur GitHub (cas courant pour `package-lock.json`) lu comme vide, sans aucun avertissement | §3 |
| B4 | **Majeur** | Les alias npm sont envoyés à OSV sous le nom de l'alias, donc leurs vulnérabilités sont manquées | §3 |
| B5 | **Majeur** | Contraintes Composer `~3.0` et `^7.4\|^8.0` mal classées | §3 |
| B6 | **Majeur** | Contraintes Poetry `^` / `~` mal classées (faux « Outdated ») | §3 |
| B7 | **Majeur** | Un dossier local nommé `build`, `dist`, `target`, `vendor`… donne « aucun manifeste » | §3 |
| B8 | **Majeur** | Cargo : dépendances renommées (mauvais crate interrogé), workspaces et deps par cible ignorés | §3 |
| B9 | **Majeur** | Maven : `search.maven.org` est périmé, renvoie des pré-versions et échoue par intermittence | §3 |
| B10 | **Majeur** | Projet bloqué en « Scan… » pour toujours si l'app est fermée pendant un scan | §3 |
| S1 | **Majeur** | CSP désactivée + `withGlobalTauri` + commande d'écriture de fichier arbitraire | §2 |
| D1 | **Majeur** | Cask Homebrew : build arm64 uniquement, sans `depends_on arch`, app non signée, rétrogradation possible | §7 |
| P1–P5 | Moyen | Performance des scans (séquentiels, sans cache, quota GitHub, politique crates.io) | §5 |

---

## 1 bis. État des corrections (2026-11-02)

Tous les points de cet audit ont été traités dans le code. Rien n'est commité : les changements sont dans l'arbre de travail de `depradar` et de `homebrew-tap`.

| Ref. | Correction | Preuve (test) |
|---|---|---|
| B1 | OSV interrogé aussi pour Go, Maven et `==x.y.z` (version exacte du manifeste), `vuln_basis` indique la source | `exact_pins_in_go_maven_and_requirements_are_checked_against_osv`, `osv_version_prefers_the_lockfile_and_accepts_exact_pins` |
| B2 | Échecs registre/OSV distingués (`reason`, avertissements `registry_unreachable` / `osv_unavailable`), lots OSV de 1 000, une tentative de plus sur 429/5xx | `unreachable_registries_and_osv_are_reported_not_hidden`, `large_query_sets_are_split_in_batches_of_1000`, `server_errors_are_failures_not_unknown_packages` |
| B3 | Contenu lu via `raw.githubusercontent.com` (pas de limite de 1 Mo) | `github_scan_reads_files_from_raw_content_including_large_lockfiles` (lockfile de 1,5 Mo) |
| B4 | Les alias npm sont envoyés à OSV sous leur vrai nom | `local_scan_resolves_versions_vulnerabilities_and_non_registry_deps` |
| B5 | Grammaire Composer (`~X.Y`, `\|`, `@stable`) | `composer_tilde_and_pipe_are_classified_correctly`, `a_laravel_style_composer_json_is_understood` |
| B6 | Grammaire Poetry (`^`, `~`, version nue) | `poetry_caret_and_tilde_are_classified_correctly`, `a_poetry_pyproject_is_understood` |
| B7 | La racine scannée n'est jamais ignorée | `a_root_folder_named_like_an_ignored_directory_is_still_scanned` |
| B8 | Crates renommés, `[target.*]`, `[workspace.dependencies]`, `{ workspace = true }`, `Cargo.lock` de la racine du workspace | `cargo_workspace_members_inherit_versions_and_use_the_root_lockfile`, `a_cargo_workspace_is_understood` |
| B9 | Maven : `maven-metadata.xml`, pré-versions exclues, intervalles `[1.0,2.0)` | `maven_picks_highest_stable_version`, `maven_intervals_are_evaluated` |
| B10, B11 | `scanning` n'est plus persisté ; projet relu après le scan (pas de résurrection) ; pas de scan concurrent du même projet | `runScan …` (logic.test.cjs), `loadProjects drops the transient scanning flag` |
| B12 | Projets et rapports stockés dans un fichier côté Rust (plus de quota `localStorage`), migration automatique, erreurs affichées | `createRemoteStorage …`, `state_round_trips_through_the_data_directory`, `saved projects from localStorage are migrated` |
| B13 | Noms Python normalisés (PEP 503) des deux côtés | `poetry_lock_names_are_normalized`, `yarn_pnpm_poetry_composer_and_gem_lockfiles_provide_installed_versions` |
| B14 | `parse_repo` : URL de navigateur, `www.`, ssh, `?`/`#`, validation des caractères | `parses_browser_ssh_and_www_urls`, `rejects_invalid_input` |
| B15 | 401 / 403 (droits ou quota) / 429 distingués | `http_statuses_map_to_precise_error_codes` |
| B16 | Toutes les contraintes d'un `gem` sont gardées ; gems git/path ignorés | `keeps_every_constraint_of_a_gem`, `skips_gems_from_git_or_path` |
| B17 | `\|` échappé, en-têtes traduits, nom de fichier valide sous Windows | `formatReportMarkdown escapes pipes …`, `reportFileName never contains characters Windows forbids` |
| B18 | Lignes `\` jointes, options `--hash` retirées, `pkg @ url` non interrogé | `joins_hash_continuation_lines`, `a_pip_compile_requirements_file_is_understood` |
| B19 | Dépendances dédoublonnées | `local_scan_resolves_…` (`react` en dep + devDep) |
| B20 | Pré-version < version finale, `0.0.x` cassant, Packagist sans pré-versions | `prerelease_installed_is_behind_the_final_release`, `zero_zero_x_patch_bump_is_breaking`, `packagist_skips_dev_and_prerelease_versions` |
| B21 | `resolve_installed_version` prend la plus haute | `resolve_installed_version_picks_the_highest_of_several_matches` |
| B22 | Chemins/branches encodés, arbre tronqué sans manifeste = erreur, `link: true` ignoré, `optionalDependencies`/`peerDependencies`, Go `replace` / `require(` / versions majeures | tests de `github.rs`, `npm_eco.rs`, `go_eco.rs`, `latest_go` |
| B23 | Libellés via i18n, `$&` sûr dans `t()`, `lang` au démarrage, `aria-label`, redimensionnement (clavier, `pointercancel`, borne selon la fenêtre), `getVersion().catch`, tous les avis listés, focus piégé dans les modales, thème système suivi | `main.test.cjs`, `i18n.test.cjs` |
| §4 | yarn / pnpm / bun, uv / pdm, extras / groupes Poetry / PEP 735, POM parent / BOM / `dependencyManagement`, Gradle (coordonnées littérales) et catalogues de versions, profondeur 8 pour local **et** GitHub | `yarn_pnpm_poetry_…`, `maven_versions_come_from_parent_poms_and_imported_boms`, `gradle_scripts_and_version_catalogs_are_scanned` |
| S1 | CSP stricte (sans `unsafe-inline`), DOM construit avec `textContent`, boîte de dialogue d'export ouverte **côté Rust** (le JS ne choisit plus le chemin) | `hostile_package_names_are_rendered_as_text_never_as_markup`, `the page loads no external resources …`, lancement réel (voir ci-dessous) |
| S2 | Token dans le trousseau de l'OS, lu uniquement par Rust ; le JS peut seulement savoir s'il existe, le remplacer ou le supprimer ; migration depuis `localStorage` | `token_commands_never_reveal_the_token`, `a GitHub project is scanned with the repo only …` |
| S3 | Polices Outfit / JetBrains Mono embarquées (app **et** page web) | `the page loads no third-party resources` |
| S4 | Aucune requête registre pour `workspace:`, `file:`, git, chemins… | `local_scan_resolves_…` (aucune requête pour `internal`) |
| S5 | Fin de nom ancrée (`.deb` ≠ `.deb.sig`), architecture (amd64/arm64), vérification `SHA256SUMS` (sh et PowerShell), `GITHUB_TOKEN` accepté | `landing.test.cjs`, essais de `install.sh` contre un faux serveur (OK, somme falsifiée → refus, sans `SHA256SUMS` → avertissement) |
| S6 | Plus d'interpolation `${{ }}` dans les scripts : variables d'environnement | `bump-homebrew-cask.yml` (script de génération exécuté localement) |
| S7 | Confirmation avant d'installer une mise à jour | `the update flow asks for confirmation before installing` |
| P1–P3 | Manifestes traités par groupes de 4, limiteur global de 8 requêtes, cache de 15 min avec une seule requête en vol par paquet, contenus GitHub via l'hôte « raw » (hors quota REST) | `latest_versions_are_cached_across_manifests_and_scans` |
| P4 | Index *sparse* de crates.io (sans limite de débit de l'API) | `crates_io_lookup_reads_the_sparse_index` |
| P5, P6 | Un seul passage OSV pour tout le scan ; `spawn_blocking` / `tokio::fs` | `answers_are_aligned_with_the_queries` |
| P7 | État en mémoire, parse mémorisé, recherche différée (150 ms), tableau reconstruit sans toucher au reste, langue en cache | `loadProjects reuses the parse …`, `filters: search …` |
| P8 | Client HTTP partagé (`ScanCtx`), une seule version de `reqwest`, bouton d'annulation et délai global de 10 min | `a_running_scan_can_be_cancelled`, `a_scan_that_takes_too_long_times_out`, `the refresh button becomes a cancel button …` |
| §6 | `scan_local`/`scan_github` fusionnés (`scan.rs`), `enum Ecosystem` de bout en bout, avertissements structurés et traduits, `DepStatus` + `reason`, helpers HTTP communs, User-Agent versionné, métadonnées de `Cargo.toml`, version unique (`tauri.conf.json` hérite de `Cargo.toml`, test de cohérence avec `package.json`), nom de produit `DepRadar` | `src/version.test.cjs` |
| D1 | Cask : `depends_on arch: :arm64` tant que le build est arm64 seul, nom d'app exact, `depends_on macos`, description à jour ; le workflow régénère tout le cask et **refuse de rétrograder** ; build macOS universel dans `release.yml` | essais du script de génération (0.7.0 → généré, 0.6.0 et 0.6.1 → refusés) |
| D2 | Ubuntu 22.04 (+ arm64), Node 22, ESLint, seuils de couverture, `cargo audit` / `npm audit`, tests réseau hebdomadaires (`live-tests.yml`), `SHA256SUMS` publié, plusieurs `.dmg` gérés | `yaml.safe_load` sur les 4 workflows |
| D3 | Les scripts de désinstallation suppriment aussi les données (Linux, macOS, Windows) | — |
| §8 | Fixtures réelles (Laravel, Poetry, `pip-compile`, workspace Cargo), HTTP simulé (`wiremock`), tests de l'interface complète dans jsdom (`main.test.cjs`, 29 scénarios), tests de la page web, tests de `scan` / `cancel` / état / export | voir couverture |

Dépendances : `cargo update` a corrigé 13 vulnérabilités de dépendances transitives (`rustls`, `quinn-proto`, `rustls-webpki`, `webpki-root-certs`, `zip`). `cargo audit` ne signale plus aucune vulnérabilité (restent 8 avertissements « non maintenu / unsound » de crates GTK/zbus tirées par Tauri, hors de portée) ; `npm audit` : 0.

### Couverture de code

| | Lignes | Seuil CI |
|---|---|---|
| Rust (`cargo llvm-cov`, hors `lib.rs`/`main.rs` qui ne font qu'ouvrir la fenêtre) | **95,9 %** (200 tests + 10 tests réseau) | 90 % |
| JavaScript (`src/*.js|cjs`, `docs/landing.js`) | **98,9 %** (100 tests) | 90 % |

### Ce qui n'a pas pu être vérifié ici

- **Le lancement réel de l'app a été vérifié** (compilation, démarrage sous Xvfb avec la CSP stricte, chargement du fichier d'état, polices, rendu du tableau, badges, avertissements traduits), mais les actions qui dépendent du système n'ont pas été exercées : trousseau du système (le code gère son absence, testé avec un faux magasin), boîte de dialogue d'export native, mise à jour automatique.
- **Les workflows GitHub Actions n'ont pas tourné** (YAML validé, script de génération du cask exécuté localement). En particulier le build macOS universel, le runner `ubuntu-22.04-arm` et le job `checksums` ne seront confirmés qu'à la prochaine release.
- **La signature et la notarisation macOS exigent un compte Apple Developer** : le workflow transmet les secrets `APPLE_*` à `tauri-action` et le README explique lesquels créer, mais sans ces secrets l'app reste non signée.
- Le cask pointe encore sur le build arm64 de la v0.6.1 (c'est le seul qui existe) ; il passera à l'universel dès la première release publiée avec ce code.

---

## 2. Sécurité

### S1 — CSP désactivée, API Tauri globale et écriture de fichier arbitraire — **Majeur**
- `src-tauri/tauri.conf.json:18-20` : `"csp": null` ; `:10` : `"withGlobalTauri": true`.
- `src-tauri/src/commands.rs:385-389` : `export_report(path, content)` écrit **n'importe quel contenu à n'importe quel chemin** choisi par le JS, sans aucune vérification.
- Le frontend injecte en `innerHTML` des données qui viennent de **dépôts tiers non fiables** : noms et versions de paquets d'un repo GitHub scanné, réponses des registres, messages d'avertissement (`src/main.js:169-173, 255, 272, 293-304, 326-332`).
- Aujourd'hui l'échappement (`escapeHtml`) est appliqué correctement partout. Ce n'est donc **pas une faille exploitable en l'état**, mais un seul oubli d'échappement futur suffirait pour obtenir un XSS, puis `invoke("export_report", {path: "~/.bashrc", ...})`, donc une exécution de code. Il n'y a aucune défense en profondeur.
- **Correctif** :
  1. Définir une CSP stricte (`default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; style-src 'self'; font-src 'self'`).
  2. Faire choisir le chemin d'export **côté Rust** (`tauri_plugin_dialog::DialogExt::file().blocking_save_file()`) au lieu d'accepter un chemin venant du JS, ou à défaut vérifier que le chemin vient d'un dialogue récent.
  3. Construire le DOM avec `textContent` plutôt qu'avec `innerHTML`.

### S2 — Token GitHub stocké en clair dans `localStorage` — **Moyen**
- `src/main.js:3, 357, 431, 549`. Le PAT est lisible par tout script de la webview (voir S1) et stocké en clair sur le disque (`~/.local/share/com.sikander.depradar/…`, `~/Library/WebKit/…`).
- **Correctif** : stocker le token dans le trousseau de l'OS (crate `keyring` derrière une commande Tauri) et ne jamais le renvoyer au JS. Côté Rust, lire le token directement dans `scan_github`.

### S3 — Polices Google chargées à chaque lancement — **Moyen** (vie privée / RGPD / hors-ligne)
- `src/index.html:5-10` : l'application de bureau appelle `fonts.googleapis.com` à chaque démarrage, ce qui transmet l'IP de l'utilisateur à Google. Une jurisprudence allemande (LG München, 2022) a jugé cette pratique contraire au RGPD. Hors-ligne, la police de repli s'affiche.
- **Correctif** : embarquer les fichiers `woff2` (Outfit, JetBrains Mono) dans `src/assets/fonts` et les déclarer en `@font-face` local. Même remarque pour `docs/index.html:12-16`.

### S4 — Fuite de noms de paquets internes vers les registres publics — **Moyen**
- `src-tauri/src/commands.rs:124-133` : une requête au registre public est envoyée **pour chaque dépendance**, y compris `workspace:*`, `file:../lib`, `link:`, `git+…`, `path = …` (Cargo filtre ce dernier cas, npm, Composer et Ruby ne filtrent pas).
- Conséquences : (a) les noms de paquets privés partent vers npmjs, Packagist, RubyGems… (b) si un paquet public homonyme existe (scénario de *dependency confusion*), l'UI affiche sa version « latest » comme si c'était la dépendance du projet.
- **Correctif** : ne pas interroger le registre quand la contrainte déclarée est non-registre (`version::base_version(..).is_none()` et pas d'alias npm). Afficher alors un statut dédié « local / git ».

### S5 — Scripts d'installation : pas de vérification d'intégrité, sélection d'asset fragile — **Mineur**
- `docs/install.sh:67` et `docs/index.html:848-850` sélectionnent l'asset par **sous-chaîne** (`_amd64.deb`, `.AppImage`). La release contient aussi `…_amd64.deb.sig` et `…AppImage.sig` (vérifié sur v0.6.1). Le bon fichier n'est choisi que parce que l'API renvoie aujourd'hui les assets dans l'ordre alphabétique. Ancrer la recherche sur la fin du nom (`…\.deb"` / `endsWith`).
- Aucun contrôle de somme ni de signature avant `dpkg -i` ou l'exécution du setup Windows (`docs/install.ps1:121-126`).
- `install.sh` n'interroge pas l'architecture : sur un Linux arm64 avec `dpkg`, il télécharge le `.deb` amd64, qui échoue.
- L'API GitHub non authentifiée (60 requêtes/h par IP) fait échouer le script derrière un NAT d'entreprise, avec un message trompeur (« Could not find a .deb asset »).

### S6 — Interpolation `${{ … }}` directement dans des scripts shell de workflow — **Mineur**
- `.github/workflows/bump-homebrew-cask.yml:18, 23` : `${{ github.event.release.tag_name }}` est injecté tel quel dans `run:`. C'est le motif classique d'injection de script. Le risque est faible, car seuls les mainteneurs créent des releases. **Correctif** : passer par `env: TAG: ${{ … }}` puis utiliser `"$TAG"`.

### S7 — Mise à jour installée sans confirmation — **Mineur**
- `src/main.js:677-682` : téléchargement, installation et redémarrage immédiats. Le paquet est signé (minisign), mais un travail en cours (scan) est interrompu sans prévenir. Une confirmation suffirait.

---

## 3. Bugs fonctionnels

### B1 — Go, Maven (et `requirements.txt` épinglé) jamais vérifiés contre OSV — **Critique**
- `src-tauri/src/commands.rs:168-171` ne vérifie que les dépendances dont `installed` est connu, or `installed` ne vient que d'un lockfile. `go.mod` et `pom.xml` n'ont pas de lockfile supporté, donc **aucune** vérification de vulnérabilités n'a lieu pour Go et Java. Pourtant, le README (« Known limitations ») reconnaît lui-même que la version déclarée *est* la version installée pour ces deux écosystèmes.
- **Preuve (OSV, en direct)** : `org.apache.logging.log4j:log4j-core@2.14.1` → **8 vulnérabilités** (dont Log4Shell) ; `golang.org/x/net@v0.10.0` → **30 vulnérabilités**. DepRadar affiche 0 dans les deux cas.
- Même chose pour `requirements.txt` avec `==x.y.z`, qui est une version exacte.
- **Correctif** : pour Go et Java, et pour Python quand la contrainte est `==x.y.z` (sans joker), passer la version déclarée à OSV. L'UI peut indiquer « vérifié sur la version déclarée ».

### B2 — Échecs réseau ou registre silencieux — **Critique**
- `src-tauri/src/osv.rs:52-65` : en cas d'erreur réseau, de HTTP ≠ 2xx ou de JSON invalide, la fonction renvoie une `HashMap` vide. L'UI affiche alors **« pas de vulnérabilité »**, ce qui est un faux sentiment de sécurité.
- `src-tauri/src/registry.rs` (toutes les fonctions `.ok()?`) : un timeout, une erreur 429 ou 5xx devient `latest = None`, donc « Unknown ». Résultat : un scan hors-ligne « réussit » avec 100 % d'Unknown, sans aucun message d'erreur.
- Limite OSV `querybatch` de 1 000 requêtes par lot : un manifeste plus gros échoue entièrement, et silencieusement.
- **Correctif** : remonter un avertissement par manifeste (« Vérification des vulnérabilités impossible : … ») et un compteur d'échecs de registre. Découper les lots OSV par 1 000. Distinguer « Unknown (introuvable) » de « Erreur (réseau) » dans `DepStatus`.

### B3 — Lockfiles de plus de 1 Mo sur GitHub lus comme vides — **Majeur**
- `src-tauri/src/source/github.rs:173-187` utilise l'API Contents avec `Accept: application/vnd.github+json`. Au-delà de 1 Mo, GitHub renvoie **`"content": ""`, `"encoding": "none"`** avec un statut 200.
- **Preuve (en direct)** : `facebook/create-react-app/package-lock.json` (2 068 078 octets) → `encoding=none content_len=0`. Ensuite `parse_package_lock("")` → `None` (sonde P15) → `commands.rs:346-352` : aucune version installée, **aucun avertissement**, et le `lockfile_path` est pourtant renseigné comme si le lockfile avait été lu.
- Un `package-lock.json` dépasse 1 Mo dans la plupart des projets réels de taille moyenne, et `Cargo.lock` / `composer.lock` souvent aussi.
- **Correctif** : demander `Accept: application/vnd.github.raw+json` (jusqu'à 100 Mo) ou télécharger via `download_url` / `raw.githubusercontent.com`. Si `encoding != "base64"`, produire un avertissement.

### B4 — Les alias npm faussent la vérification OSV — **Majeur**
- `commands.rs:115-122` résout bien `"x": "npm:lodash@^4"` vers `lodash` pour le registre, mais `attach_vulnerabilities` (`commands.rs:168-176`) envoie à OSV le **nom de l'alias** (`d.name`). Les vulnérabilités du vrai paquet sont manquées, et un alias homonyme d'un autre paquet produit de faux positifs.
- **Correctif** : conserver le `lookup_name` dans `DependencyInfo` (champ `package`) et l'utiliser pour OSV.

### B5 — Contraintes Composer mal interprétées — **Majeur**
- `src-tauri/src/version.rs:157-165` utilise la grammaire **npm** pour Composer, or :
  - le `~` de Composer à deux composantes signifie `>=3.0 <4.0`, alors que pour npm il signifie `>=3.0.0 <3.1.0` ;
  - Composer accepte `|` (simple barre) comme OU.
- **Preuve (sonde)** : `~3.0` vs `3.5.0` → `Outdated` (attendu : `UpToDate`) ; `^7.4|^8.0` vs `8.3.0` → `Major` (attendu : `UpToDate`). Ces deux syntaxes sont omniprésentes dans les `composer.json`, Symfony et Laravel compris.
- **Correctif** : normaliser avant le parsing (`|` → `||`, `~X.Y` → `^X.Y` quand il y a exactement deux composantes, retirer les `v` et les drapeaux `@stable` / `@dev`), ou implémenter la grammaire Composer.

### B6 — Contraintes Poetry `^` et `~` mal interprétées — **Majeur**
- `version.rs:148-152` : `pep440_rs::VersionSpecifiers` ne comprend pas `^` ni le `~` de Poetry, d'où un repli sur la comparaison avec le plancher.
- **Preuve (sonde)** : `^2.28.0` vs `2.32.3` → `Outdated` (attendu : `UpToDate`) ; `~2.28` vs `2.28.5` → `Outdated` (attendu : `UpToDate`). `[tool.poetry.dependencies]` utilise `^` par défaut, donc **tout** projet Poetry sans `poetry.lock` est faux.
- **Correctif** : traduire `^X.Y.Z` → `>=X.Y.Z,<(X+1).0.0` (règles de Poetry pour 0.x) et `~X.Y` → `>=X.Y,<X.(Y+1)` avant de parser en PEP 440.

### B7 — Scan local : un dossier racine nommé comme un dossier ignoré ne donne rien — **Majeur**
- `src-tauri/src/source/local.rs:13-23` : `filter_entry` s'applique aussi à la **racine** (profondeur 0).
- **Preuve (sonde)** : racine `…/build`, `…/vendor`, `…/dist`, `…/target` contenant un `package.json` → **0 manifeste**, donc erreur « no_manifests_found ».
- **Correctif** : `filter_entry(|e| e.depth() == 0 || !(e.file_type().is_dir() && IGNORE_DIRS.contains(..)))`.

### B8 — Cargo : renommage, workspaces, dépendances par cible — **Majeur**
`src-tauri/src/ecosystem/rust_eco.rs:6-37`. **Preuve (sonde P7)** : avec `foo = { package = "serde", version = "1" }`, `bar = { workspace = true }`, `[target.'cfg(unix)'.dependencies] libc` et `[workspace.dependencies] tokio`, la sortie est `[("foo", "1")]` seulement.
- **Renommage** : `foo` est interrogé sur crates.io au lieu de `serde`. Résultat : mauvaise version « latest », ou Unknown, ou pire, un crate tiers sans rapport.
- **`{ workspace = true }`** est ignoré, et `[workspace.dependencies]` n'est jamais lu : dans un workspace moderne, les membres n'affichent presque rien.
- **`[target.'cfg(..)'.dependencies]`** est ignoré.
- **Lockfile de workspace** : `Cargo.lock` est à la racine du workspace et non à côté du `Cargo.toml` du membre. `local::sibling_lockfile` (`local.rs:42-47`) et `commands.rs:340-342` ne remontent pas l'arborescence, donc les membres n'ont pas de version installée.
- **Correctif** : lire `package`, fusionner `workspace.dependencies`, parcourir `target.*.dependencies`, chercher le lockfile en remontant jusqu'à la racine du scan.

### B9 — Maven : source « latest » non fiable — **Majeur**
- `src-tauri/src/registry.rs:122-142` interroge `search.maven.org/solrsearch` et prend `latestVersion`.
- **Preuve (en direct, aujourd'hui)** :

  | Artefact | Solr (`latestVersion`) | `maven-metadata.xml` |
  |---|---|---|
  | `org.junit.jupiter:junit-jupiter` | `5.13.0-M3` (milestone **et** périmé) | `6.1.3` |
  | `org.springframework:spring-core` | réponse non-JSON (échec) | `7.1.0-RC1` |
  | `com.fasterxml.jackson.core:jackson-databind` | réponse non-JSON (échec) | `2.22.4` |

- Les pré-versions sont ensuite classées « Breaking » (**sonde P13** : `5.3.20` vs `6.0.0-M1` → `Major`).
- **Correctif** : lire `https://repo1.maven.org/maven2/<g/>/<a>/maven-metadata.xml`, parcourir `<versions>` et prendre la plus haute version **sans qualificatif de pré-version** (`-M`, `-RC`, `alpha`, `beta`, `SNAPSHOT`, `.Beta`, `.CR`).

### B10 — État `scanning` persisté : projet bloqué en « Scan… » — **Majeur**
- `src/main.js:347-348` écrit `scanning: true` dans `localStorage`. Si l'app est fermée, plante ou est mise à jour (S7) pendant un scan, le drapeau reste à `true` et `renderMain` (`:229-230`) affiche « Scan… » **indéfiniment** au prochain lancement.
- **Correctif** : ne jamais persister `scanning` (le garder dans un `Set` en mémoire), ou le réinitialiser au démarrage.

### B11 — Un projet supprimé pendant son scan est recréé — **Moyen**
- `src/main.js:343-375` : `project` est capturé avant l'`await`. Si l'utilisateur supprime le projet pendant le scan, `upsertProject(project)` (`:370`, puis `logic.cjs:147-156`) **le réinsère** à la fin du scan.
- Deux scans du même projet peuvent aussi tourner en parallèle (bouton ↻ jamais désactivé, ou ↻ pendant « Scan all »). Le dernier qui termine écrase l'autre.
- **Correctif** : relire le projet après l'`await` et abandonner s'il n'existe plus. Ignorer un rescan si le projet est déjà en cours.

### B12 — Quota `localStorage` dépassé : exception non gérée — **Moyen**
- Les rapports complets de tous les projets sont stockés dans **une seule** clé `localStorage` (`logic.cjs:135-137`), dont le quota est d'environ 5 Mo. Un monorepo ou une dizaine de projets suffisent pour le dépasser.
- `upsertProject` (`main.js:370`) est **hors** du `try` : la `QuotaExceededError` interrompt `scanProject`, le rendu n'est pas mis à jour et le projet reste « scanning » (voir B10).
- **Correctif** : stocker les rapports côté Rust (fichier JSON dans `app_data_dir`) ou dans IndexedDB, et entourer la persistance d'un `try`.

### B13 — Normalisation des noms Python (PEP 503) absente — **Moyen**
- `src-tauri/src/ecosystem/python_eco.rs:102-118` et `commands.rs:108` : `poetry.lock` stocke les noms normalisés (`typing-extensions`, `django`), alors que le manifeste peut écrire `typing_extensions` ou `Django`.
- **Preuve (sonde P8)** : recherche de `typing_extensions` → `None`. La version installée est perdue, donc OSV ne vérifie pas non plus.
- **Correctif** : normaliser les deux côtés (`lowercase` et `[-_.]+` → `-`).

### B14 — `parse_repo` accepte des formats invalides et les transforme silencieusement — **Moyen**
`src-tauri/src/source/github.rs:15-30`. **Preuve (sonde P4)** :

| Entrée | Résultat actuel |
|---|---|
| `https://github.com/rust-lang/rust/tree/master/src` (URL copiée depuis le navigateur) | `("rust-lang", "rust/tree/master/src")` |
| `git@github.com:rust-lang/rust.git` | `("git@github.com:rust-lang", "rust")` |
| `https://www.github.com/rust-lang/rust` | `("https:", "/www.github.com/rust-lang/rust")` |
| `…/rust?tab=readme` | `("rust-lang", "rust?tab=readme")` |

Tous ces cas aboutissent à « dépôt introuvable » au lieu d'être compris ou rejetés proprement. **Correctif** : parser l'URL (`url::Url`), accepter `www.`, `git@…:`, ignorer ce qui suit les deux premiers segments, et valider avec `^[A-Za-z0-9-]+$` / `^[A-Za-z0-9._-]+$`.

### B15 — Codes HTTP GitHub mal interprétés — **Moyen**
- `github.rs:76, 147, 190` : **tout 403** est présenté comme « limite de débit atteinte », y compris un token sans droits, une organisation SSO non autorisée ou un fichier trop gros. Un **401** (token invalide ou expiré) tombe dans « GitHub a répondu 401 » sans explication. Un **429** n'est pas géré.
- **Correctif** : lire `x-ratelimit-remaining` / `retry-after`, ajouter les codes `github_unauthorized` et `github_forbidden` (traduits dans les 8 langues).

### B16 — Ruby : seule la première contrainte d'un `gem` est gardée — **Moyen**
- `src-tauri/src/ecosystem/ruby_eco.rs:40-51`. **Preuve (sondes P9, P9b)** : `gem 'rails', '>= 6', '< 8'` → `">= 6"`, puis vs `8.0.0` → `UpToDate` (attendu : `Major`). Le classifieur Ruby gère déjà les listes séparées par des virgules (`version.rs:179-193`). Il suffit de concaténer toutes les chaînes entre guillemets qui suivent le nom.

### B17 — Export Markdown cassé par `|` et non traduit — **Moyen**
- `src/logic.cjs:81-88`. **Preuve** : une dépendance `^1.0.0 || ^2.0.0` produit `| a | ^1.0.0 || ^2.0.0 | — | …`, soit une table décalée de deux colonnes. Il faut échapper `|` en `\|` dans chaque cellule.
- Les en-têtes `Package / Declared / Installed / Latest / Status / Vulnerabilities`, `Source:`, `Generated:`, `DepRadar report` sont codés en dur en anglais alors que l'application gère 8 langues.
- `src/main.js:568` : le nom de fichier par défaut d'un projet Windows vaut `C:-Users-me-proj-depradar-report.md` (vérifié). Le `:` est **interdit** dans un nom de fichier Windows. Utiliser `projectDisplayName(project.value)` et retirer `[<>:"/\\|?*]`.

### B18 — `requirements.txt` : continuations de ligne et URL directes — **Mineur**
- `python_eco.rs:6-24`. **Preuve (sonde P6)** : `requests==2.28.1 \` (sortie classique de `pip-compile --generate-hashes`) donne la valeur déclarée `"==2.28.1 \"`, donc la contrainte n'est plus parsable par PEP 440. Il faut joindre les lignes qui finissent par `\` avant de les traiter.
- `requests @ https://…whl` → déclaré `"@ https://…"`, puis une recherche PyPI inutile (voir S4).

### B19 — Doublons de dépendances — **Mineur**
- `npm_eco.rs:12-20`, `rust_eco.rs:13-17`, `php_eco.rs:13-26` : un même nom présent dans `dependencies` **et** `devDependencies` donne deux lignes et deux requêtes au registre (**sonde P10**). Dans `commands.rs:177-181`, `found.remove(&dep.name)` ne donne les vulnérabilités **qu'à la première** occurrence : la seconde ligne affiche 0 CVE.
- **Correctif** : dédupliquer par nom en gardant la portée (dep, dev, build) comme métadonnée, et utiliser `get().cloned()` au lieu de `remove`.

### B20 — Pré-versions et lignes 0.0.x — **Mineur**
- `version.rs:247-256` ne compare que des triplets et ignore la pré-version. **Sonde P11** : installé `2.0.0-beta.1`, latest `2.0.0` → `UpToDate` (attendu : `Outdated`).
- `version.rs:82-84` : en semver Cargo, `0.0.x` → `0.0.y` est cassant (`^0.0.1` = `=0.0.1`). **Sonde P12** → `Outdated` au lieu de `Major`.
- `registry.rs:100-106` (Packagist) ne filtre que `dev-*` : un tag `-RC1`, `-beta` ou `-alpha` en tête de liste est pris pour « latest ».

### B21 — `resolve_installed_version` ne fait pas ce que dit sa doc — **Mineur**
- `rust_eco.rs:56-73` : la doc annonce « la plus haute en cas de correspondances multiples », mais le code renvoie la **première**. **Sonde P14** : `>=1` parmi `1.0.1, 1.0.5, 2.0.0` → `1.0.1`. Utiliser `filter(..).max()`.

### B22 — Divers backend — **Mineur**
- `github.rs:126, 167` : branche et chemin non encodés dans l'URL. Un fichier dont le chemin contient `#`, `?` ou `%` récupère le mauvais fichier ou un 404, ignoré silencieusement (`Ok(None)` → `continue`). Une branche par défaut contenant `/` casse `git/trees/{branch}` : utiliser le SHA du commit.
- `commands.rs:371-373` : si l'arbre est tronqué et qu'aucun manifeste n'est trouvé, le scan renvoie `Ok` avec un rapport vide au lieu de l'erreur `no_manifests_found_github`.
- `npm_eco.rs:46-53` (**sonde P16**) : les entrées `"link": true` (paquets de workspace) sont prises pour des versions installées.
- `npm_eco.rs:12` : `optionalDependencies` et `peerDependencies` sont ignorées.
- Go (`go_eco.rs`) : les directives `replace` sont ignorées (on affiche le module d'origine alors qu'un fork est utilisé) ; `require(` sans espace n'est pas reconnu ; une nouvelle version majeure (`/v2` → `/v3`) ne peut jamais être signalée, ce qui est une limite du protocole proxy à documenter.

### B23 — Frontend : divers — **Mineur**
- `src/main.js:225` : le badge `"Local"` / `"GitHub"` est codé en dur alors que les clés `sidebar.typeLocal` et `sidebar.typeGithub` existent.
- `src/i18n.cjs:674` : `replaceAll(pattern, value)` interprète `$&`, `$1`… dans la valeur. **Preuve** : `t("error.network_error", {detail: "cost $& now"})` → `"Network error: cost {{detail}} now."`. Utiliser `replaceAll(p, () => String(value))`.
- `<html lang="en">` n'est mis à jour qu'au changement de langue (`i18n.cjs:664`), pas au démarrage, ce qui gêne les lecteurs d'écran. `aria-label="Language"` est en dur (`index.html:51`). La clé `form.github.tokenPlaceholder` et les 7 clés `ecosystem.*` ne sont jamais utilisées (vérifié par script).
- `main.js:40-53` : le redimensionnement de la barre latérale ne gère pas `pointercancel` / `lostpointercapture` (l'état « dragging » peut rester bloqué), n'est pas accessible au clavier, et sa largeur maximale (600 px) ne dépend pas de la largeur de la fenêtre (800 px par défaut, il ne reste alors que 200 px).
- `main.js:644` : `getVersion()` n'a pas de `.catch`. Le libellé « Scan all » n'est pas retraduit si la langue change pendant un scan. Le badge de vulnérabilités ne pointe que vers la **première** advisory (`main.js:323`) et est libellé « CVE » alors que les identifiants sont surtout des GHSA.
- Les modales n'ont pas de *focus trap* et ne rendent pas le focus à la fermeture (accessibilité).
- Le bouton de thème ne réagit pas à un changement du thème système.

---

## 4. Couverture fonctionnelle (manques notables)

| Écosystème | Manque | Impact |
|---|---|---|
| npm | `yarn.lock`, `pnpm-lock.yaml`, `bun.lock` non lus ; lockfile de workspace à la racine non utilisé par les membres | Pas de version installée ni de vérification CVE pour la majorité des projets JS modernes |
| Python | `uv.lock`, `pdm.lock`, `[project.optional-dependencies]`, `[tool.poetry.group.*]`, `[dependency-groups]` (PEP 735) | Dépendances de dev et lockfiles modernes ignorés |
| Rust | Voir B8 (workspaces) | Workspaces quasi vides |
| Java | POM parent, BOM, `dependencyManagement`, Gradle (`libs.versions.toml` serait, lui, parsable statiquement) | Beaucoup de dépendances ignorées |
| Go | Voir B1 (OSV) et B22 | — |
| Local | Profondeur limitée à 4 en local, illimitée sur GitHub : résultats différents pour un même dépôt | Incohérence |

---

## 5. Performance

### P1 — Manifestes traités séquentiellement — **Moyen**
- `commands.rs:200-264` et `316-369` : boucle `for` avec `await` à chaque tour. La concurrence (8) ne s'applique qu'**à l'intérieur** d'un manifeste. Un monorepo de 30 `package.json` enchaîne 30 séries de requêtes et 30 appels OSV.
- **Correctif** : traiter les manifestes avec `buffer_unordered` et un `Semaphore` global (8 à 16 requêtes simultanées au total).

### P2 — Aucun cache ni dédoublonnage des requêtes aux registres — **Moyen**
- Le même paquet (`react`, `serde`…) est interrogé une fois **par manifeste** et à chaque scan. Avec « Scan all », tout est refait.
- **Correctif** : un cache `HashMap<(eco, name), Option<String>>` partagé pendant un scan, et un cache court en mémoire (TTL d'environ 15 min) partagé entre scans via `tauri::State`. Conserver l'`ETag` pour npm et PyPI.

### P3 — Scan GitHub coûteux en quota et lent — **Moyen**
- Chaque scan coûte 2 appels, plus un appel par manifeste et un par lockfile, **séquentiellement**, sur l'API REST (60 requêtes/h sans token). Un monorepo moyen épuise le quota en un seul scan, et chaque fichier refusé devient un avertissement.
- **Correctif** : récupérer le contenu via `raw.githubusercontent.com/{owner}/{repo}/{sha}/{path}`, qui n'est pas décompté du quota REST et règle aussi B3, en parallèle. Ou télécharger l'archive `tarball` en un seul appel. Utiliser le SHA du commit de l'arbre pour garantir la cohérence.

### P4 — Politique d'accès de crates.io non respectée — **Moyen**
- `registry.rs:6-19` envoie jusqu'à 8 requêtes simultanées à `crates.io/api/v1/crates/{name}`. La *Data Access Policy* de crates.io demande **au plus 1 requête/s** aux clients automatisés, avec un risque de 429 ou de blocage du User-Agent.
- **Correctif** : utiliser l'index *sparse* `https://index.crates.io/{prefix}/{name}` (CDN, sans limite ; dernière ligne non `yanked` et sans pré-version), ou brider à 1 requête/s.

### P5 — Un appel OSV par manifeste — **Mineur**
- `commands.rs:257, 362` : regrouper toutes les dépendances du scan en lots de 1 000 au maximum (voir B2), soit 1 à 2 requêtes au lieu de N.

### P6 — E/S bloquantes dans des commandes `async` — **Mineur**
- `commands.rs:191, 209, 231` : `WalkDir` et `std::fs::read_to_string` s'exécutent sur un worker du runtime tokio. Sur un gros arbre ou un disque réseau, cela bloque d'autres tâches. Utiliser `tokio::task::spawn_blocking` ou `tokio::fs`.

### P7 — Frontend : re-parsing complet et re-rendu à chaque frappe — **Moyen**
- `loadProjects()` fait un `JSON.parse` de **tout** le stockage, rapports compris, à chaque `findProject`, `renderSidebar` et `upsertProject`. `renderMain()` est appelé à chaque frappe dans la recherche (`main.js:605-608`) et reconstruit tous les tableaux par `innerHTML`. `t()` relit `localStorage` à chaque appel (`i18n.cjs:644-669`).
- Avec quelques milliers de dépendances, la saisie devient saccadée.
- **Correctif** : garder l'état en mémoire (persistance uniquement à la sauvegarde), ajouter un *debounce* d'environ 150 ms sur la recherche, filtrer en masquant les lignes existantes, et mettre la langue en cache.

### P8 — Divers — **Mineur**
- Le client HTTP est recréé à chaque scan (`commands.rs:16-22`), ce qui perd le pool de connexions et les sessions TLS. Le partager via `tauri::State`.
- Deux versions de `reqwest` sont compilées (0.12 pour l'app, 0.13 pour les plugins Tauri, visible dans le build clippy). Passer à `reqwest 0.13` réduit la taille du binaire et le temps de build.
- Il n'existe ni bouton d'annulation ni délai global pour un scan : seuls des timeouts de 15 s par requête s'appliquent.

---

## 6. Qualité du code / maintenabilité

- **Duplication** : `scan_local` et `scan_github` (`commands.rs:184-380`) dupliquent toute la chaîne (parser, chercher le lockfile, résoudre, attacher les CVE). Extraire un `fn scan_manifest(source, path, content, lock)` commun.
- **Écosystème typé par chaînes** : `"rust"`, `"npm"`… sont des `&str` partout (`commands.rs`, `source/mod.rs`, `osv.rs`). `ecosystem_enum` retombe sur **Python** pour toute valeur inconnue (`commands.rs:24-34`), ce qui est dangereux. Utiliser un `enum Ecosystem` sérialisable de bout en bout.
- `parse_manifest` a un paramètre `ecosystem` inutilisé (`let _ = ecosystem;`, `commands.rs:46-48`).
- Les avertissements sont des chaînes anglaises (`commands.rs:212, 251, 296` ; `github.rs:171-191`), alors que les erreurs fatales ont des codes traduits. Utiliser la même structure `{code, detail}` pour les warnings, conformément à la règle i18n du projet.
- `DepStatus::Unknown` regroupe « non publié », « contrainte non-registre » et « erreur réseau » : ajouter un champ `reason`.
- `registry.rs` contient 7 fonctions quasi identiques (`get`, test du statut, JSON, extraction). Un helper `fetch_json(url) -> Result<Value, FetchError>` réduirait la duplication et permettrait de corriger B2 en un seul endroit.
- User-Agent `DepRadar/0.1` figé (`commands.rs:18`) : utiliser `env!("CARGO_PKG_VERSION")`.
- `Cargo.toml:4-5` : `description = "A Tauri App"`, `authors = ["you"]` (valeurs du gabarit).
- La version est dupliquée dans 3 fichiers (`package.json`, `Cargo.toml`, `tauri.conf.json`). Tauri peut lire la version de `Cargo.toml` si on retire `version` de `tauri.conf.json`.
- `productName: "depradar"` en minuscules alors que la marque, le cask et les scripts utilisent `DepRadar` (voir D1).
- Les tests réseau `#[ignore]` (`commands.rs:501-785`) sont copiés-collés et ne tournent jamais en CI.

---

## 7. Distribution, CI/CD et dépôt `homebrew-tap`

### D1 — Cask Homebrew — **Majeur**
- `homebrew-tap/Casks/depradar.rb:5` ne référence que `…_aarch64.dmg` et n'a **pas** de `depends_on arch: :arm64`. Sur un Mac Intel, `brew install` réussit puis l'app ne se lance pas. `docs/install.sh` envoie tous les Mac vers ce cask.
- `:12` : `app "DepRadar.app"`, alors que le bundle s'appelle réellement **`depradar.app`** (vérifié dans `depradar_aarch64.app.tar.gz` de la v0.6.1 : `CFBundleName = depradar`). Cela ne fonctionne que sur un volume APFS insensible à la casse et échoue sur un volume sensible à la casse. Même remarque pour `docs/uninstall.sh:142`.
- Le bundle n'est **pas signé ni notarisé** : aucun `_CodeSignature` dans l'archive, et aucun secret `APPLE_*` dans `release.yml`. Gatekeeper bloque l'app téléchargée (« endommagée » ou « développeur non vérifié »).
- `:7` : description obsolète (« Rust, JavaScript, and Python ») alors que 7 écosystèmes sont supportés.
- Il manque `depends_on macos: ">= :high_sierra"`, alors que `LSMinimumSystemVersion` vaut 10.13.
- `homebrew-tap/README.md:47-49` décrit encore une procédure manuelle (`sha256 :no_check`) remplacée par le workflow automatique.
- **Rétrogradation possible** : `bump-homebrew-cask.yml` réécrit la version sans la comparer à l'actuelle. L'historique du tap le montre : `Bump 0.6.0` → **`Bump 0.5.1`** → `Bump 0.6.1`, car la v0.5.1 a été publiée après la v0.6.0. Pendant cet intervalle, `brew install` installait une version plus ancienne. Ajouter une comparaison `sort -V` avant le commit.

### D2 — Workflows — **Moyen**
- `release.yml:16` : `macos-latest` ne produit que de l'arm64. Ajouter `--target universal-apple-darwin` (ou un job x86_64). Il n'y a pas de build Linux arm64.
- `release.yml` construit le binaire Linux sur `ubuntu-latest` (24.04). Le `.deb` et l'AppImage exigent alors une glibc récente et ne démarrent pas sur Debian 11 ou Ubuntu 20.04/22.04. Construire sur `ubuntu-22.04`.
- `ci.yml:15, 44` et `release.yml:35` : **Node 20 est en fin de vie depuis avril 2026**. Passer à Node 22 ou 24.
- Le JavaScript n'a pas de lint (ni ESLint ni Prettier) en CI, et `docs/index.html` (script de 400 lignes) n'a aucun test.
- Les tests d'intégration réseau ne tournent jamais. Ajouter un job planifié (`schedule:` hebdomadaire) avec `cargo test -- --ignored` pour détecter les changements d'API des registres (voir B9 : Maven a déjà changé de comportement).
- `bump-homebrew-cask.yml:25` : `shasum ./*.dmg` casse dès qu'il y a plus d'un `.dmg` (le jour où un build x64 ou universel sera ajouté).

### D3 — Désinstallation incomplète — **Mineur**
- `docs/uninstall.sh:148-157` (Linux) ne supprime pas `~/.local/share/com.sikander.depradar` (stockage de la webview, **qui contient le token GitHub**, voir S2). Le README affirme pourtant « AppImage : rien d'autre à nettoyer ».
- macOS : `~/Library/Caches/com.sikander.depradar` et `~/Library/Saved Application State/…` sont oubliés.
- Windows (`uninstall.ps1`) ne supprime pas `%APPDATA%\com.sikander.depradar`.

---

## 8. Tests

- La couverture unitaire des parseurs et du classifieur est bonne, mais les tests valident les **cas prévus** : aucun test n'utilise de manifeste réel (Composer `~`/`|`, Poetry `^`, workspace Cargo, `pip-compile`). Ajouter des *fixtures* réelles dans `src-tauri/tests/fixtures/` (un `composer.json` Laravel, un `pyproject` Poetry, un workspace Cargo, un `package-lock` > 1 Mo).
- Aucun test pour `main.js` (logique de scan, persistance, scénarios B10, B11, B12). Extraire la logique de `scanProject` dans `logic.cjs` pour la rendre testable avec un `storage` simulé, comme le reste.
- Aucun test e2e de l'application (WebDriver Tauri / `tauri-driver`).
- Les appels HTTP ne sont jamais simulés : `registry.rs`, `osv.rs` et `github.rs` ne sont testés que pour leur partie parsing. Ajouter `wiremock` ou `mockito` pour tester les erreurs 403/401/429/5xx, le timeout et la réponse `encoding: none` (B2, B3, B15).

---

## 9. Plan d'action recommandé (par ordre de priorité)

1. **Fiabilité de la sécurité** : B1 (OSV pour Go, Maven, `==`), B2 (remonter les échecs OSV et registres, découper les lots), B4 (alias npm), B3 (contenu brut GitHub).
2. **Justesse du classement** : B5 (Composer), B6 (Poetry), B9 (Maven), B20 (pré-versions).
3. **Robustesse de l'application** : B10, B11, B12 (état de scan et persistance), B7 (racine ignorée), B14, B15 (saisie du dépôt et codes HTTP).
4. **Durcissement** : S1 (CSP et export côté Rust), S2 (trousseau), S3 (polices locales), S4 (pas de requête registre pour les deps non-registre).
5. **Performance** : P1 à P4 (parallélisme, cache, raw GitHub, index sparse crates.io), puis P7 (frontend).
6. **Distribution** : D1 (arch, nom du bundle, signature et notarisation, garde-fou anti-rétrogradation), D2 (universal, ubuntu-22.04, Node 22+).
7. **Couverture** : B8 (workspaces Cargo), lockfiles yarn, pnpm et uv (§4), puis le reste de la section 3.
8. **Qualité et tests** : §6 et §8 (factorisation des scans, enum `Ecosystem`, warnings traduits, mocks HTTP, fixtures réelles, job planifié pour les tests réseau).

---

### Annexe — sortie brute des sondes (code actuel, v0.6.1)

```
P1  root 'build'|'vendor'|'dist'|'target' -> manifests found: 0
P2  php ~3.0 vs 3.5.0 -> Outdated
P2b php ^7.4|^8.0 vs 8.3.0 -> Major
P2c php ^7.4 || ^8.0 vs 8.3.0 -> UpToDate
P3  python ^2.28.0 vs 2.32.3 -> Outdated
P3b python ~2.28 vs 2.28.5 -> Outdated
P4  parse_repo(".../rust/tree/master/src") -> ("rust-lang", "rust/tree/master/src")
P4b parse_repo("git@github.com:rust-lang/rust.git") -> ("git@github.com:rust-lang", "rust")
P4c parse_repo("https://www.github.com/rust-lang/rust") -> ("https:", "/www.github.com/rust-lang/rust")
P4d parse_repo(".../rust?tab=readme") -> ("rust-lang", "rust?tab=readme")
P6  requirements "requests==2.28.1 \" -> ("requests", "==2.28.1 \\")
P7  Cargo renamed/workspace/target deps -> [("foo", "1")] seulement
P8  poetry.lock lookup 'typing_extensions' -> None
P9  gem 'rails', '>= 6', '< 8' -> ("rails", ">= 6") ; '>= 6' vs 8.0.0 -> UpToDate
P10 dep + devDep du même nom -> 2 entrées
P11 installé 2.0.0-beta.1, latest 2.0.0 -> UpToDate
P12 installé 0.0.1, latest 0.0.2 (Rust) -> Outdated (devrait être Major)
P13 java 5.3.20 vs 6.0.0-M1 -> Major
P14 resolve '>=1' parmi 1.0.1, 1.0.5, 2.0.0 -> 1.0.1
P15 parse_package_lock("") -> None (aucun avertissement)
P16 entrée "link": true -> {"@app/web": "0.0.0"} comptée comme installée
```
