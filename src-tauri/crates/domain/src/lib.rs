pub mod error;
pub mod events;
pub mod models;

pub use error::{AppError, ErrorCode};
pub use events::{Citation, DownloadStage, IngestionProgress, ModelDownloadProgress, TokenEvent};
pub use models::{
    ApiKeyStatus, ApiKeyStorage, AppLifecycleState, CapacityState, Document, DocumentPreview,
    DocumentStatus, EXTRACTED_TEXT_MAX_BYTES, ExternalProviderConnection, IndexProvenance,
    MAX_UPLOAD_BYTES, MarkdownBlock, MarkdownInline,
};
