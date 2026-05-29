use serde::ser::SerializeMap;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("Document not found: {0}")]
    NotFound(String),
    #[error("Database error: {0}")]
    Database(String),
    #[error("IO error: {0}")]
    Io(String),
    #[error("Vector DB error: {0}")]
    VectorDb(String),
    #[error("Queue full")]
    QueueFull,
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Inference error: {0}")]
    Inference(String),
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(2))?;
        // Convert the variant name into a kind string (e.g. NotFound, Database, etc.)
        // A simple way is to use Debug format for discriminant and clean it up, or just match
        let kind = match self {
            AppError::NotFound(_) => "NotFound",
            AppError::Database(_) => "Database",
            AppError::Io(_) => "Io",
            AppError::VectorDb(_) => "VectorDb",
            AppError::QueueFull => "QueueFull",
            AppError::Internal(_) => "Internal",
            AppError::Inference(_) => "Inference",
        };
        map.serialize_entry("kind", kind)?;
        map.serialize_entry("message", &self.to_string())?;
        map.end()
    }
}
