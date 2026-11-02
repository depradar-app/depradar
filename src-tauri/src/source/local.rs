//! Walks a local folder looking for dependency manifests and their lockfiles.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::{FileSource, IGNORE_DIRS, LOCKFILE_FILES, MANIFEST_FILES, MAX_DEPTH};
use crate::error::AppError;

/// Every manifest and lockfile under `root` (up to [`MAX_DEPTH`] levels),
/// skipping the usual large/generated directories, as '/'-separated paths
/// relative to `root`. The root itself is never skipped, even when its own
/// name is in `IGNORE_DIRS` (scanning a folder called `build` must work).
pub fn find_files(root: &Path) -> Vec<String> {
    let walker = WalkDir::new(root)
        .max_depth(MAX_DEPTH)
        .into_iter()
        .filter_entry(|e| {
            e.depth() == 0
                || !(e.file_type().is_dir()
                    && IGNORE_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
        });

    let mut found = vec![];
    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str() else {
            continue;
        };
        if !(MANIFEST_FILES.contains(&name) || LOCKFILE_FILES.contains(&name)) {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(root) {
            let rel = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            found.push(rel);
        }
    }
    found.sort();
    found
}

/// Reads files below a local scan root.
pub struct LocalSource {
    pub root: PathBuf,
}

impl FileSource for LocalSource {
    async fn read(&self, rel: &str) -> Result<Option<String>, AppError> {
        let path = rel
            .split('/')
            .fold(self.root.clone(), |acc, part| acc.join(part));
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => Ok(Some(content)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(AppError::with_detail("read_failed", e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }

    #[test]
    fn find_files_excludes_ignored_directories() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("Cargo.toml"));
        touch(&dir.path().join("Cargo.lock"));
        touch(&dir.path().join("packages").join("api").join("package.json"));
        touch(
            &dir.path()
                .join("node_modules")
                .join("foo")
                .join("package.json"),
        );
        touch(&dir.path().join("README.md"));

        assert_eq!(
            find_files(dir.path()),
            vec![
                "Cargo.lock".to_string(),
                "Cargo.toml".to_string(),
                "packages/api/package.json".to_string(),
            ]
        );
    }

    #[test]
    fn find_files_respects_the_max_depth_boundary() {
        let dir = tempfile::tempdir().unwrap();
        // Depth is counted in path segments: 8 is found, 9 is not.
        let ok: PathBuf = ["a", "b", "c", "d", "e", "f", "g", "Cargo.toml"]
            .iter()
            .collect();
        let too_deep: PathBuf = ["a", "b", "c", "d", "e", "f", "g", "h", "package.json"]
            .iter()
            .collect();
        touch(&dir.path().join(&ok));
        touch(&dir.path().join(&too_deep));

        assert_eq!(
            find_files(dir.path()),
            vec!["a/b/c/d/e/f/g/Cargo.toml".to_string()]
        );
    }

    #[test]
    fn find_files_on_empty_directory_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_files(dir.path()).is_empty());
    }

    #[test]
    fn a_root_folder_named_like_an_ignored_directory_is_still_scanned() {
        for name in ["build", "vendor", "dist", "target"] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join(name);
            touch(&root.join("package.json"));
            assert_eq!(
                find_files(&root),
                vec!["package.json".to_string()],
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn local_source_reads_nested_files_and_reports_absence() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a")).unwrap();
        fs::write(dir.path().join("a").join("x.txt"), "hello").unwrap();
        let src = LocalSource {
            root: dir.path().to_path_buf(),
        };
        assert_eq!(
            src.read("a/x.txt").await.unwrap(),
            Some("hello".to_string())
        );
        assert_eq!(src.read("a/missing.txt").await.unwrap(), None);
    }

    #[tokio::test]
    async fn local_source_reports_unreadable_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("bin"), [0xff, 0xfe, 0xfd]).unwrap();
        let src = LocalSource {
            root: dir.path().to_path_buf(),
        };
        let err = src.read("bin").await.unwrap_err();
        assert_eq!(err.code, "read_failed");
    }
}
