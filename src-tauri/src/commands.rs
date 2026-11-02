//! Tauri commands: thin wrappers around the scan pipeline plus the pieces of
//! app state that must not live in the webview (the GitHub token, saved
//! projects, report export).

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use crate::error::AppError;
use crate::models::ScanReport;
use crate::scan::{self, ScanCtx};

// ------------------------------------------------------------ GitHub token

/// Persistent storage for the GitHub token.
pub trait TokenStore: Send + Sync {
    fn get(&self) -> Result<Option<String>, String>;
    fn set(&self, token: &str) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

/// The OS credential store (Keychain / Credential Manager / Secret Service).
pub struct KeyringStore;

impl KeyringStore {
    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new("com.sikander.depradar", "github-token").map_err(|e| e.to_string())
    }
}

impl TokenStore for KeyringStore {
    fn get(&self) -> Result<Option<String>, String> {
        match Self::entry()?.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, token: &str) -> Result<(), String> {
        Self::entry()?
            .set_password(token)
            .map_err(|e| e.to_string())
    }

    fn delete(&self) -> Result<(), String> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Holds the GitHub token on the Rust side only: the webview can say whether
/// one is set and replace it, but can never read it back. If the OS store is
/// unavailable (no Secret Service on a bare Linux session...), the token is
/// still kept for the current session.
pub struct TokenManager {
    store: Box<dyn TokenStore>,
    memory: Mutex<Option<String>>,
}

impl TokenManager {
    pub fn new(store: Box<dyn TokenStore>) -> Self {
        Self {
            store,
            memory: Mutex::new(None),
        }
    }

    pub fn get(&self) -> Option<String> {
        let mut memory = self.memory.lock().unwrap_or_else(|e| e.into_inner());
        if memory.is_none() {
            *memory = self.store.get().ok().flatten();
        }
        memory.clone()
    }

    pub fn save(&self, token: &str) -> Result<(), AppError> {
        let token = token.trim();
        let mut memory = self.memory.lock().unwrap_or_else(|e| e.into_inner());
        if token.is_empty() {
            *memory = None;
            return self
                .store
                .delete()
                .map_err(|e| AppError::with_detail("token_storage_failed", e));
        }
        *memory = Some(token.to_string());
        self.store
            .set(token)
            .map_err(|e| AppError::with_detail("token_storage_failed", e))
    }
}

pub struct AppState {
    pub ctx: ScanCtx,
    pub tokens: TokenManager,
    /// Scans in progress, by id, so they can be cancelled.
    scans: Mutex<HashMap<String, Arc<tokio::sync::Notify>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self::with(
            ScanCtx::default(),
            TokenManager::new(Box::new(KeyringStore)),
        )
    }

    pub fn with(ctx: ScanCtx, tokens: TokenManager) -> Self {
        Self {
            ctx,
            tokens,
            scans: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

#[tauri::command]
pub fn github_token_status(state: State<'_, AppState>) -> bool {
    state.tokens.get().is_some()
}

#[tauri::command]
pub fn save_github_token(state: State<'_, AppState>, token: String) -> Result<(), AppError> {
    state.tokens.save(&token)
}

// -------------------------------------------------------------------- scans

/// A scan that takes longer than this is abandoned.
const SCAN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Runs a scan so it can be cancelled with [`cancel_scan`] and can't run
/// forever.
async fn run_cancellable<F>(
    state: &AppState,
    scan_id: String,
    timeout: Duration,
    scan: F,
) -> Result<ScanReport, AppError>
where
    F: Future<Output = Result<ScanReport, AppError>>,
{
    let cancel = Arc::new(tokio::sync::Notify::new());
    state
        .scans
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(scan_id.clone(), cancel.clone());

    let result = tokio::select! {
        r = tokio::time::timeout(timeout, scan) => {
            r.unwrap_or_else(|_| Err(AppError::new("scan_timeout")))
        }
        _ = cancel.notified() => Err(AppError::new("scan_cancelled")),
    };

    state
        .scans
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&scan_id);
    result
}

/// Aborts the scan registered under `scan_id`; `false` if none is running.
#[tauri::command]
pub fn cancel_scan(state: State<'_, AppState>, scan_id: String) -> bool {
    let scans = state.scans.lock().unwrap_or_else(|e| e.into_inner());
    match scans.get(&scan_id) {
        Some(cancel) => {
            cancel.notify_one();
            true
        }
        None => false,
    }
}

#[tauri::command]
pub async fn scan_local(
    state: State<'_, AppState>,
    path: String,
    scan_id: String,
) -> Result<ScanReport, AppError> {
    let scan = scan::scan_local_path(&state.ctx, &path);
    run_cancellable(&state, scan_id, SCAN_TIMEOUT, scan).await
}

#[tauri::command]
pub async fn scan_github(
    state: State<'_, AppState>,
    repo: String,
    scan_id: String,
) -> Result<ScanReport, AppError> {
    let token = state.tokens.get();
    let scan = scan::scan_github_repo(&state.ctx, &repo, token.as_deref());
    run_cancellable(&state, scan_id, SCAN_TIMEOUT, scan).await
}

// ------------------------------------------------------------------- export

pub fn write_report(path: &Path, content: &str) -> Result<(), AppError> {
    std::fs::write(path, content).map_err(|e| {
        AppError::with_detail("export_write_failed", format!("{}: {e}", path.display()))
    })
}

/// A bare file name (no directory part) usable as a save-dialog default.
pub fn safe_file_name(name: &str) -> String {
    // Split on both separators by hand: `Path::file_name` only knows the
    // current OS's, and a name typed on another platform must behave the same.
    let last = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned = last.replace(['<', '>', ':', '"', '|', '?', '*'], "-");
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        "depradar-report.md".to_string()
    } else {
        cleaned
    }
}

/// Asks the user where to save the report with a native dialog and writes it
/// there. The destination is chosen here, on the Rust side, so the webview
/// can never make the app write to an arbitrary path. Returns `false` if the
/// user cancelled.
#[tauri::command]
pub async fn export_report(
    app: AppHandle,
    content: String,
    default_name: String,
) -> Result<bool, AppError> {
    let name = safe_file_name(&default_name);
    let picked = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Markdown", &["md"])
            .set_file_name(&name)
            .blocking_save_file()
    })
    .await
    .map_err(|e| AppError::with_detail("export_write_failed", e.to_string()))?;

    let Some(file_path) = picked else {
        return Ok(false);
    };
    let path = file_path
        .into_path()
        .map_err(|e| AppError::with_detail("export_write_failed", e.to_string()))?;
    write_report(&path, &content)?;
    Ok(true)
}

// ------------------------------------------------------------ saved projects

const STATE_FILE: &str = "projects.json";

pub fn read_state(dir: &Path) -> Result<String, AppError> {
    match std::fs::read_to_string(dir.join(STATE_FILE)) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(AppError::with_detail("state_io_failed", e.to_string())),
    }
}

/// Atomically replaces the saved-projects file (write to a temp file, then
/// rename), so a crash mid-write can't leave a truncated state behind.
pub fn write_state(dir: &Path, json: &str) -> Result<(), AppError> {
    let io = |e: std::io::Error| AppError::with_detail("state_io_failed", e.to_string());
    std::fs::create_dir_all(dir).map_err(io)?;
    let tmp = dir.join(format!("{STATE_FILE}.tmp"));
    std::fs::write(&tmp, json).map_err(io)?;
    std::fs::rename(&tmp, dir.join(STATE_FILE)).map_err(io)
}

fn data_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, AppError> {
    app.path()
        .app_data_dir()
        .map_err(|e| AppError::with_detail("state_io_failed", e.to_string()))
}

/// Saved projects (with their last reports) live in a file under the app's
/// data directory instead of the webview's ~5 MB `localStorage` quota.
#[tauri::command]
pub fn load_state<R: Runtime>(app: AppHandle<R>) -> Result<String, AppError> {
    read_state(&data_dir(&app)?)
}

#[tauri::command]
pub fn save_state<R: Runtime>(app: AppHandle<R>, json: String) -> Result<(), AppError> {
    write_state(&data_dir(&app)?, &json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct FakeStore {
        value: Mutex<Option<String>>,
        fail: bool,
        deleted: Mutex<bool>,
    }

    impl TokenStore for Arc<FakeStore> {
        fn get(&self) -> Result<Option<String>, String> {
            if self.fail {
                return Err("no keyring".into());
            }
            Ok(self.value.lock().unwrap().clone())
        }
        fn set(&self, token: &str) -> Result<(), String> {
            if self.fail {
                return Err("no keyring".into());
            }
            *self.value.lock().unwrap() = Some(token.to_string());
            Ok(())
        }
        fn delete(&self) -> Result<(), String> {
            if self.fail {
                return Err("no keyring".into());
            }
            *self.value.lock().unwrap() = None;
            *self.deleted.lock().unwrap() = true;
            Ok(())
        }
    }

    #[test]
    fn token_is_saved_to_the_store_and_read_back() {
        let store = Arc::new(FakeStore::default());
        let mgr = TokenManager::new(Box::new(store.clone()));
        assert_eq!(mgr.get(), None);
        mgr.save("  ghp_abc  ").unwrap();
        assert_eq!(mgr.get(), Some("ghp_abc".to_string()));
        assert_eq!(store.value.lock().unwrap().as_deref(), Some("ghp_abc"));
    }

    #[test]
    fn token_already_in_the_store_is_loaded_lazily() {
        let store = Arc::new(FakeStore::default());
        *store.value.lock().unwrap() = Some("stored".into());
        let mgr = TokenManager::new(Box::new(store));
        assert_eq!(mgr.get(), Some("stored".to_string()));
    }

    #[test]
    fn saving_an_empty_token_deletes_it() {
        let store = Arc::new(FakeStore::default());
        let mgr = TokenManager::new(Box::new(store.clone()));
        mgr.save("tok").unwrap();
        mgr.save("   ").unwrap();
        assert_eq!(mgr.get(), None);
        assert!(*store.deleted.lock().unwrap());
    }

    #[test]
    fn an_unavailable_credential_store_still_keeps_the_token_for_the_session() {
        let store = Arc::new(FakeStore {
            fail: true,
            ..Default::default()
        });
        let mgr = TokenManager::new(Box::new(store));
        let err = mgr.save("tok").unwrap_err();
        assert_eq!(err.code, "token_storage_failed");
        assert_eq!(mgr.get(), Some("tok".to_string()));
        let err = mgr.save("").unwrap_err();
        assert_eq!(err.code, "token_storage_failed");
        assert_eq!(mgr.get(), None);
    }

    #[test]
    fn report_is_written_to_the_given_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.md");
        write_report(&path, "# Report\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Report\n");
    }

    #[test]
    fn unwritable_report_path_gives_a_structured_error() {
        let err =
            write_report(Path::new("/this/directory/does/not/exist/report.md"), "x").unwrap_err();
        assert_eq!(err.code, "export_write_failed");
    }

    #[test]
    fn default_report_names_are_sanitized() {
        assert_eq!(safe_file_name("proj-report.md"), "proj-report.md");
        assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_file_name("C:\\Users\\me.md"), "me.md");
        assert_eq!(safe_file_name("a:b?.md"), "a-b-.md");
        assert_eq!(safe_file_name(""), "depradar-report.md");
        assert_eq!(safe_file_name(".."), "depradar-report.md");
    }

    #[test]
    fn state_round_trips_through_the_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("app").join("data");
        assert_eq!(read_state(&nested).unwrap(), "");
        write_state(&nested, "[1,2,3]").unwrap();
        assert_eq!(read_state(&nested).unwrap(), "[1,2,3]");
        write_state(&nested, "[]").unwrap();
        assert_eq!(read_state(&nested).unwrap(), "[]");
        assert!(!nested.join("projects.json.tmp").exists());
    }

    #[test]
    fn unreadable_state_is_a_structured_error() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be.
        std::fs::create_dir_all(dir.path().join("projects.json")).unwrap();
        assert_eq!(read_state(dir.path()).unwrap_err().code, "state_io_failed");
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        assert_eq!(
            write_state(&blocker.join("sub"), "{}").unwrap_err().code,
            "state_io_failed"
        );
    }

    #[test]
    fn app_state_can_be_built_without_touching_the_keyring() {
        let state = AppState::default();
        assert!(state.ctx.ep.osv.starts_with("https://"));
    }

    #[test]
    fn app_errors_display_their_code_and_detail() {
        assert_eq!(AppError::new("a").to_string(), "a");
        assert_eq!(AppError::with_detail("a", "b").to_string(), "a: b");
    }

    // The mock runtime links Tauri's `test` feature, whose Windows test
    // executables fail to start (missing comctl32 v6 manifest): these tests
    // run on Linux and macOS only.
    #[cfg(not(windows))]
    mod mock_runtime {
        use super::*;

        use crate::registry::Endpoints;
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        fn mock_app_with(
            ep: Endpoints,
            store: Arc<FakeStore>,
        ) -> tauri::App<tauri::test::MockRuntime> {
            let app = tauri::test::mock_app();
            app.manage(AppState::with(
                ScanCtx::new(ep),
                TokenManager::new(Box::new(store)),
            ));
            app
        }

        #[tokio::test]
        async fn token_commands_never_reveal_the_token() {
            let store = Arc::new(FakeStore::default());
            let app = mock_app_with(Endpoints::default(), store.clone());
            assert!(!github_token_status(app.state()));
            save_github_token(app.state(), "ghp_secret".into()).unwrap();
            assert!(github_token_status(app.state()));
            assert_eq!(store.value.lock().unwrap().as_deref(), Some("ghp_secret"));
            save_github_token(app.state(), String::new()).unwrap();
            assert!(!github_token_status(app.state()));
        }

        #[tokio::test]
        async fn scan_commands_run_the_pipeline_with_the_stored_token() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/gh/repos/o/r"))
                .and(header("authorization", "Bearer ghp_secret"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"default_branch": "main"})),
                )
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/gh/repos/o/r/git/trees/main"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"tree": [{"path": "package.json", "type": "blob"}]}),
                ))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/raw/o/r/main/package.json"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_string(r#"{"dependencies": {"x": "workspace:*"}}"#),
                )
                .mount(&server)
                .await;

            let store = Arc::new(FakeStore::default());
            let app = mock_app_with(Endpoints::all_at(&server.uri()), store);
            save_github_token(app.state(), "ghp_secret".into()).unwrap();

            let report = scan_github(app.state(), "o/r".into(), "g1".into())
                .await
                .unwrap();
            assert_eq!(report.source, "o/r");
            assert_eq!(report.manifests.len(), 1);

            let dir = tempfile::tempdir().unwrap();
            std::fs::write(
                dir.path().join("package.json"),
                r#"{"dependencies": {"x": "workspace:*"}}"#,
            )
            .unwrap();
            let local = scan_local(
                app.state(),
                dir.path().to_string_lossy().to_string(),
                "l1".into(),
            )
            .await
            .unwrap();
            assert_eq!(local.manifests.len(), 1);

            let err = scan_local(app.state(), "/no/such/dir".into(), "l2".into())
                .await
                .unwrap_err();
            assert_eq!(err.code, "local_dir_not_found");
        }

        #[test]
        fn state_commands_use_the_app_data_directory() {
            let app = mock_app_with(Endpoints::default(), Arc::new(FakeStore::default()));
            let handle = app.handle().clone();
            save_state(handle.clone(), "[\"p\"]".into()).unwrap();
            assert_eq!(load_state(handle).unwrap(), "[\"p\"]");
        }

        fn empty_report() -> ScanReport {
            ScanReport {
                source: "s".into(),
                manifests: vec![],
                warnings: vec![],
            }
        }

        #[tokio::test]
        async fn a_running_scan_can_be_cancelled() {
            let app = mock_app_with(Endpoints::default(), Arc::new(FakeStore::default()));
            let state: State<'_, AppState> = app.state();
            assert!(!cancel_scan(app.state(), "nothing".into()));

            let slow = async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(empty_report())
            };
            let (result, _) = tokio::join!(
                run_cancellable(&state, "s1".into(), Duration::from_secs(60), slow),
                async {
                    // Wait until the scan has registered itself.
                    while !cancel_scan(app.state(), "s1".into()) {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                }
            );
            assert_eq!(result.unwrap_err().code, "scan_cancelled");
            // Once finished, the id is forgotten.
            assert!(!cancel_scan(app.state(), "s1".into()));
        }

        #[tokio::test]
        async fn a_scan_that_takes_too_long_times_out() {
            let app = mock_app_with(Endpoints::default(), Arc::new(FakeStore::default()));
            let state: State<'_, AppState> = app.state();
            let slow = async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(empty_report())
            };
            let result =
                run_cancellable(&state, "s2".into(), Duration::from_millis(20), slow).await;
            assert_eq!(result.unwrap_err().code, "scan_timeout");
        }

        #[tokio::test]
        async fn a_scan_that_finishes_returns_its_report() {
            let app = mock_app_with(Endpoints::default(), Arc::new(FakeStore::default()));
            let state: State<'_, AppState> = app.state();
            let report = run_cancellable(&state, "s3".into(), Duration::from_secs(5), async {
                Ok(empty_report())
            })
            .await
            .unwrap();
            assert_eq!(report.source, "s");
        }
    }
}
