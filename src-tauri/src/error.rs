//! Structured, user-facing error type shared by every Tauri command.
//!
//! Commands return `Result<T, AppError>`. The frontend renders `title`,
//! `message`, `suggestion` and a collapsible `details` block — never a raw
//! stack trace. `code` is a stable machine-readable discriminator.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    /// Short human-readable title, e.g. "Not enough disk space".
    pub title: String,
    /// One or two sentence explanation in plain language.
    pub message: String,
    /// Optional technical detail (stderr, IO error, exit code) for the
    /// collapsible "Technical details" area.
    pub details: Option<String>,
    /// Optional actionable suggestion, e.g. "Free up space and try again".
    pub suggestion: Option<String>,
    /// Stable machine code, e.g. "INVALID_PDF", "NO_DISK_SPACE".
    pub code: String,
}

impl AppError {
    pub fn new(
        code: impl Into<String>,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            title: title.into(),
            message: message.into(),
            details: None,
            suggestion: None,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    // ---- Common, reusable errors -------------------------------------------

    pub fn invalid_pdf(path: &str) -> Self {
        Self::new(
            "INVALID_PDF",
            "The selected file is not a valid PDF",
            format!("\"{path}\" could not be opened as a PDF document."),
        )
        .with_suggestion("Make sure the file is a real PDF and is not corrupted.")
    }

    pub fn output_not_writable(path: &str) -> Self {
        Self::new(
            "OUTPUT_NOT_WRITABLE",
            "The output folder is not writable",
            format!("OffPDF cannot write to \"{path}\"."),
        )
        .with_suggestion("Pick a different output folder, or check folder permissions.")
    }

    #[allow(dead_code)] // Reusable error; surfaced when a check is added backend-side.
    pub fn no_disk_space() -> Self {
        Self::new(
            "NO_DISK_SPACE",
            "Not enough disk space",
            "There may not be enough free space to complete this operation safely.",
        )
        .with_suggestion("Free up disk space, or choose an output folder on a drive with more room.")
    }

    pub fn engine_failed(details: impl Into<String>) -> Self {
        Self::new(
            "ENGINE_FAILED",
            "PDF engine failed to process this file",
            "The local PDF engine reported an error while processing the document.",
        )
        .with_details(details)
        .with_suggestion("Open the technical details below. The file may be encrypted or damaged.")
    }

    pub fn engine_missing() -> Self {
        Self::new(
            "ENGINE_MISSING",
            "PDF engine not found",
            "The bundled qpdf engine could not be located and qpdf is not on your PATH.",
        )
        .with_suggestion("Reinstall OffPDF, or install qpdf so it is available on your system PATH.")
    }

    pub fn cancelled() -> Self {
        Self::new(
            "CANCELLED",
            "Operation cancelled",
            "The operation was cancelled before it finished.",
        )
    }

    pub fn io(context: &str, err: impl std::fmt::Display) -> Self {
        Self::new(
            "IO_ERROR",
            "A file system error occurred",
            context.to_string(),
        )
        .with_details(err.to_string())
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}: {}", self.code, self.title, self.message)
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::io("An unexpected file system error occurred.", err)
    }
}
