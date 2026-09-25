use serde::{Deserialize, Serialize, ser::SerializeMap};

/// Machine-readable category classification for structured application errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    InvalidInput,
    QueueFull,
    DuplicateDocument,
    Storage,
    VectorDb,
    Inference,
    Internal,
}

impl ErrorCode {
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::QueueFull | Self::Inference)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::InvalidInput => "invalid_input",
            Self::QueueFull => "queue_full",
            Self::DuplicateDocument => "duplicate_document",
            Self::Storage => "storage",
            Self::VectorDb => "vector_db",
            Self::Inference => "inference",
            Self::Internal => "internal",
        }
    }
}

/// Structured application error contract serialized over the Tauri IPC bridge (DEV-14).
///
/// Encapsulates error classification (`code`), the executing context (`operation`),
/// human-readable diagnostics (`detail`), and actionable remediation guidance (`recovery`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, thiserror::Error)]
#[error("{operation}: {detail}. {recovery}")]
pub struct AppError {
    pub code: ErrorCode,
    pub operation: String,
    pub detail: String,
    pub recovery: String,
}

impl AppError {
    pub fn new(
        code: ErrorCode,
        operation: impl Into<String>,
        detail: impl Into<String>,
        recovery: impl Into<String>,
    ) -> Self {
        Self {
            code,
            operation: operation.into(),
            detail: detail.into(),
            recovery: recovery.into(),
        }
    }

    pub fn not_found(
        operation: impl Into<String>,
        detail: impl Into<String>,
        recovery: impl Into<String>,
    ) -> Self {
        Self::new(ErrorCode::NotFound, operation, detail, recovery)
    }

    pub fn invalid_input(
        operation: impl Into<String>,
        detail: impl Into<String>,
        recovery: impl Into<String>,
    ) -> Self {
        Self::new(ErrorCode::InvalidInput, operation, detail, recovery)
    }

    pub fn queue_full(
        operation: impl Into<String>,
        detail: impl Into<String>,
        recovery: impl Into<String>,
    ) -> Self {
        Self::new(ErrorCode::QueueFull, operation, detail, recovery)
    }

    pub fn duplicate_document(
        operation: impl Into<String>,
        detail: impl Into<String>,
        recovery: impl Into<String>,
    ) -> Self {
        Self::new(ErrorCode::DuplicateDocument, operation, detail, recovery)
    }

    pub fn storage(
        operation: impl Into<String>,
        detail: impl Into<String>,
        recovery: impl Into<String>,
    ) -> Self {
        Self::new(ErrorCode::Storage, operation, detail, recovery)
    }

    pub fn vector_db(
        operation: impl Into<String>,
        detail: impl Into<String>,
        recovery: impl Into<String>,
    ) -> Self {
        Self::new(ErrorCode::VectorDb, operation, detail, recovery)
    }

    pub fn inference(
        operation: impl Into<String>,
        detail: impl Into<String>,
        recovery: impl Into<String>,
    ) -> Self {
        Self::new(ErrorCode::Inference, operation, detail, recovery)
    }

    pub fn internal(
        operation: impl Into<String>,
        detail: impl Into<String>,
        recovery: impl Into<String>,
    ) -> Self {
        Self::new(ErrorCode::Internal, operation, detail, recovery)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::storage(
            "Filesystem I/O",
            e.to_string(),
            "Verify disk permissions and available storage in the application data directory.",
        )
    }
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(5))?;
        map.serialize_entry("code", &self.code)?;
        map.serialize_entry("operation", &self.operation)?;
        map.serialize_entry("detail", &self.detail)?;
        map.serialize_entry("recovery", &self.recovery)?;
        map.serialize_entry("message", &self.to_string())?;
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_serializes_structured_error() {
        let error = AppError::invalid_input(
            "Upload document",
            "File size exceeds 50 MB",
            "Select a document under 50 MB to upload.",
        );

        assert_eq!(
            error.to_string(),
            "Upload document: File size exceeds 50 MB. Select a document under 50 MB to upload."
        );

        let json = serde_json::to_value(&error).expect("serialize error");
        assert_eq!(json["code"], "invalid_input");
        assert_eq!(json["operation"], "Upload document");
        assert_eq!(json["detail"], "File size exceeds 50 MB");
        assert_eq!(
            json["recovery"],
            "Select a document under 50 MB to upload."
        );
        assert_eq!(
            json["message"],
            "Upload document: File size exceeds 50 MB. Select a document under 50 MB to upload."
        );
    }

    #[test]
    fn classifies_transient_codes() {
        assert!(ErrorCode::QueueFull.is_transient());
        assert!(ErrorCode::Inference.is_transient());
        assert!(!ErrorCode::NotFound.is_transient());
        assert!(!ErrorCode::InvalidInput.is_transient());
        assert!(!ErrorCode::DuplicateDocument.is_transient());
    }

    #[test]
    fn io_error_converts_to_storage_error() {
        let io_error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied");
        let app_error = AppError::from(io_error);

        assert_eq!(app_error.code, ErrorCode::Storage);
        assert_eq!(app_error.operation, "Filesystem I/O");
        assert!(app_error.detail.contains("permission denied"));
        assert!(!app_error.recovery.is_empty());
    }
}
