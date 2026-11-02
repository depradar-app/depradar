//! Structured, machine-readable errors returned by Tauri commands.
//!
//! Unlike the plain warning strings collected in `ScanReport.warnings`
//! (secondary, non-fatal annotations shown alongside otherwise-successful
//! results), an `AppError` is a fatal command failure. It carries a stable
//! `code` the frontend can look up in its translation dictionary, plus an
//! optional `detail` for dynamic values (a path, a status code...) that the
//! translated message can interpolate.

use serde::Serialize;

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    pub code: String,
    pub detail: Option<String>,
}

impl AppError {
    pub fn new(code: &str) -> Self {
        Self {
            code: code.to_string(),
            detail: None,
        }
    }

    pub fn with_detail(code: &str, detail: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            detail: Some(detail.into()),
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{}: {detail}", self.code),
            None => write!(f, "{}", self.code),
        }
    }
}

impl std::error::Error for AppError {}
