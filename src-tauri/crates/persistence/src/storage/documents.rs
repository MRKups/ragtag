use domain::{AppError, Document, EXTRACTED_TEXT_MAX_BYTES, MAX_UPLOAD_BYTES};
use sha2::{Digest, Sha512};

pub fn validate_upload(filename: &str, bytes: &[u8]) -> Result<&'static str, AppError> {
    let name = std::path::Path::new(filename);
    if filename.is_empty()
        || filename.len() > 255
        || name.file_name().and_then(|name| name.to_str()) != Some(filename)
    {
        return Err(AppError::invalid_input(
            "Validate document upload",
            "Upload filename must be a single file name no longer than 255 characters",
            "Provide a valid file name without path separators or special directory characters.",
        ));
    }
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(AppError::invalid_input(
            "Validate document upload",
            format!(
                "Document exceeds the {} MiB upload limit",
                MAX_UPLOAD_BYTES / (1024 * 1024)
            ),
            "Select a document file smaller than 50 MiB to upload.",
        ));
    }

    match name
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => {
            let prefix = &bytes[..bytes.len().min(1024)];
            if !prefix.windows(5).any(|window| window == b"%PDF-") {
                return Err(AppError::invalid_input(
                    "Validate document upload",
                    "Uploaded file is not a valid PDF",
                    "Ensure the file has a valid PDF header and is not corrupted.",
                ));
            }
            Ok("pdf")
        }
        Some("docx") => {
            if !bytes.starts_with(b"PK\x03\x04") {
                return Err(AppError::invalid_input(
                    "Validate document upload",
                    "Uploaded file is not a valid DOCX archive",
                    "Ensure the file is a valid Microsoft Word .docx document.",
                ));
            }
            Ok("docx")
        }
        _ => {
            if bytes.len() > EXTRACTED_TEXT_MAX_BYTES {
                return Err(AppError::invalid_input(
                    "Validate document upload",
                    format!(
                        "Text document exceeds the {} MiB indexing limit",
                        EXTRACTED_TEXT_MAX_BYTES / (1024 * 1024)
                    ),
                    "Upload a smaller text file or split it into sections under 16 MiB.",
                ));
            }
            if bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
                return Err(AppError::invalid_input(
                    "Validate document upload",
                    "Ragtag accepts UTF-8 text files, PDF documents, and DOCX documents",
                    "Choose a supported file format with valid UTF-8 encoding.",
                ));
            }
            Ok("text")
        }
    }
}

pub fn storage_extension(filename: &str, file_type: &str) -> String {
    std::path::Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 16
                && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| file_type.to_string())
}

pub fn is_managed_document_source_filename(filename: &str) -> bool {
    let Some((id, extension)) = filename.split_once('.') else {
        return false;
    };

    let is_canonical_v7_uuid = uuid::Uuid::parse_str(id)
        .is_ok_and(|uuid| uuid.get_version_num() == 7 && uuid.to_string() == id);
    let is_safe_extension = !extension.is_empty()
        && extension.len() <= 16
        && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        && extension
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit());

    is_canonical_v7_uuid && is_safe_extension
}

pub fn is_managed_document_temporary_filename(filename: &str) -> bool {
    let Some(filename) = filename.strip_prefix('.') else {
        return false;
    };
    let Some((source_filename, temporary_suffix)) = filename.rsplit_once('.') else {
        return false;
    };
    if temporary_suffix != "tmp" {
        return false;
    };
    let Some((source_filename, temporary_id)) = source_filename.rsplit_once('.') else {
        return false;
    };

    is_managed_document_source_filename(source_filename)
        && uuid::Uuid::parse_str(temporary_id)
            .is_ok_and(|uuid| uuid.get_version_num() == 7 && uuid.to_string() == temporary_id)
}

pub fn managed_documents_directory(
    app_local_data: &std::path::Path,
    create: bool,
) -> Result<std::path::PathBuf, AppError> {
    let app_data_metadata = std::fs::symlink_metadata(app_local_data).map_err(|error| {
        AppError::storage(
            "Inspect application data directory",
            format!("Could not inspect app data directory: {error}"),
            "Check that the application local data directory exists and is accessible.",
        )
    })?;
    if app_data_metadata.file_type().is_symlink() || !app_data_metadata.is_dir() {
        return Err(AppError::storage(
            "Inspect application data directory",
            "App data directory must be a real directory, not a symlink",
            "Ensure the application data directory is a regular folder.",
        ));
    }

    let documents = app_local_data.join("documents");
    match std::fs::symlink_metadata(&documents) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(AppError::storage(
                "Inspect managed documents directory",
                "Managed documents directory must be a real directory, not a symlink",
                "Ensure the documents folder in application local data is a regular directory.",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create => {
            match std::fs::create_dir(&documents) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(AppError::storage(
                        "Create documents directory",
                        format!("Could not create documents directory: {error}"),
                        "Verify folder permissions for the application data directory.",
                    ));
                }
            }
        }
        Err(error) => {
            return Err(AppError::storage(
                "Inspect managed documents directory",
                format!("Could not inspect managed documents directory: {error}"),
                "Verify disk permissions for the application data directory.",
            ));
        }
    }

    let metadata = std::fs::symlink_metadata(&documents).map_err(|error| {
        AppError::storage(
            "Inspect managed documents directory",
            format!("Could not inspect managed documents directory: {error}"),
            "Verify disk permissions for the application data directory.",
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AppError::storage(
            "Inspect managed documents directory",
            "Managed documents directory must be a real directory, not a symlink",
            "Ensure the documents folder in application local data is a regular directory.",
        ));
    }
    std::fs::canonicalize(documents)
        .map(strip_unc_prefix)
        .map_err(|error| {
            AppError::storage(
                "Resolve managed documents directory",
                format!("Could not resolve managed documents directory: {error}"),
                "Check folder permissions for the application data directory.",
            )
        })
}

pub fn strip_unc_prefix(path: std::path::PathBuf) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let path_str = path.to_string_lossy();
        if let Some(stripped) = path_str.strip_prefix(r"\\?\") {
            return std::path::PathBuf::from(stripped);
        }
    }
    path
}

pub fn managed_storage_filename(storage_path: &str) -> Result<&str, AppError> {
    let normalized = storage_path
        .strip_prefix("documents/")
        .or_else(|| storage_path.strip_prefix("documents\\"));
    let Some(filename) = normalized else {
        return Err(AppError::internal(
            "Resolve document storage filename",
            "Document has an unsafe storage path",
            "Provide a path residing within the managed documents directory.",
        ));
    };
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || std::path::Path::new(filename)
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        || !is_managed_document_source_filename(filename)
    {
        return Err(AppError::internal(
            "Resolve document storage filename",
            "Document has an unsafe storage path",
            "Provide a valid managed document filename.",
        ));
    }
    Ok(filename)
}

pub fn document_storage_candidate(
    app_local_data: &std::path::Path,
    storage_path: &str,
) -> Result<std::path::PathBuf, AppError> {
    let documents = managed_documents_directory(app_local_data, false)?;
    Ok(documents.join(managed_storage_filename(storage_path)?))
}

pub fn document_storage_candidate_for_deletion(
    app_local_data: &std::path::Path,
    storage_path: &str,
) -> Result<Option<std::path::PathBuf>, AppError> {
    let filename = managed_storage_filename(storage_path)?;
    let app_data_metadata = std::fs::symlink_metadata(app_local_data).map_err(|error| {
        AppError::storage(
            "Inspect application data directory",
            format!("Could not inspect app data directory: {error}"),
            "Check that the application local data directory exists and is accessible.",
        )
    })?;
    if app_data_metadata.file_type().is_symlink() || !app_data_metadata.is_dir() {
        return Err(AppError::storage(
            "Inspect application data directory",
            "App data directory must be a real directory, not a symlink",
            "Ensure the application data directory is a regular folder.",
        ));
    }

    let documents = app_local_data.join("documents");
    match std::fs::symlink_metadata(&documents) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(AppError::storage(
                "Inspect managed documents directory",
                "Managed documents directory must be a real directory, not a symlink",
                "Verify that the documents folder in application local data is a regular directory.",
            ))
        }
        Ok(_) => std::fs::canonicalize(documents)
            .map(|documents| Some(strip_unc_prefix(documents).join(filename)))
            .map_err(|error| {
                AppError::storage(
                    "Resolve managed documents directory",
                    format!("Could not resolve managed documents directory: {error}"),
                    "Check folder permissions for the application data directory.",
                )
            }),
        Err(error) => Err(AppError::storage(
            "Inspect managed documents directory",
            format!("Could not inspect managed documents directory: {error}"),
            "Check disk permissions for the application data directory.",
        )),
    }
}

pub fn document_storage_path(
    app_local_data: &std::path::Path,
    storage_path: &str,
) -> Result<std::path::PathBuf, AppError> {
    let candidate = document_storage_candidate(app_local_data, storage_path)?;
    let metadata = std::fs::symlink_metadata(&candidate).map_err(|error| {
        AppError::storage(
            "Inspect managed document source",
            format!("Could not inspect managed document source: {error}"),
            "Verify file permissions in the application data directory.",
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AppError::storage(
            "Validate managed document source",
            "Managed document source must be a regular file, not a symlink",
            "Ensure the documents directory contains only authentic document files.",
        ));
    }

    let documents = candidate.parent().ok_or_else(|| {
        AppError::internal(
            "Resolve managed document directory",
            "Managed document path has no parent",
            "Check that the documents directory exists in application local data.",
        )
    })?;
    let source = std::fs::canonicalize(&candidate)
        .map(strip_unc_prefix)
        .map_err(|error| {
            AppError::storage(
                "Resolve managed document source",
                format!("Could not resolve managed document source: {error}"),
                "Verify file path accessibility in the documents directory.",
            )
        })?;
    if !source.starts_with(documents) {
        return Err(AppError::storage(
            "Validate managed document path",
            "Managed document source resolves outside the documents directory",
            "Keep managed documents inside the application local data directory.",
        ));
    }
    Ok(source)
}

pub fn verify_document_source_integrity(
    app_local_data: &std::path::Path,
    document: &Document,
) -> Result<std::path::PathBuf, AppError> {
    use std::io::Read as _;

    let source = document_storage_path(app_local_data, &document.storage_path)?;
    let expected_size = document.file_size_bytes.ok_or_else(|| {
        AppError::storage(
            "Validate document integrity",
            "Managed document source has no recorded file size for integrity validation",
            "Re-index the document to recalculate integrity metadata.",
        )
    })?;
    let expected_hash = document.file_hash.as_deref().ok_or_else(|| {
        AppError::storage(
            "Validate document integrity",
            "Managed document source has no recorded hash for integrity validation",
            "Re-index the document to recalculate integrity metadata.",
        )
    })?;

    let mut file = std::fs::File::open(&source).map_err(|error| {
        AppError::storage(
            "Open managed document source",
            format!("Could not open managed document source for integrity validation: {error}"),
            "Check that the document file has not been deleted or locked.",
        )
    })?;
    let metadata = file.metadata().map_err(|error| {
        AppError::storage(
            "Inspect managed document metadata",
            format!("Could not inspect managed document source for integrity validation: {error}"),
            "Verify filesystem permissions in the documents directory.",
        )
    })?;
    if !metadata.is_file() {
        return Err(AppError::storage(
            "Validate document source",
            "Managed document source must be a regular file for integrity validation",
            "Ensure the document is a regular file on disk.",
        ));
    }
    if i64::try_from(metadata.len()).ok() != Some(expected_size) {
        return Err(AppError::storage(
            "Validate document integrity",
            "Managed document source size does not match its recorded metadata",
            "Re-upload or re-index the document if its contents were modified on disk.",
        ));
    }

    let mut hasher = Sha512::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            AppError::storage(
                "Read managed document source",
                format!("Could not read managed document source for integrity validation: {error}"),
                "Check disk health and file permissions.",
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    if hasher.finalize().as_slice() != expected_hash {
        return Err(AppError::storage(
            "Validate document integrity",
            "Managed document source hash does not match its recorded metadata",
            "Re-upload the original document if its content was altered on disk.",
        ));
    }

    Ok(source)
}

pub fn write_document_source_atomically(
    storage_path: &std::path::Path,
    bytes: &[u8],
) -> Result<(), AppError> {
    use std::io::Write as _;

    let parent = storage_path.parent().ok_or_else(|| {
        AppError::internal(
            "Write document source",
            "Managed document path has no parent",
            "Verify storage directory hierarchy in application data.",
        )
    })?;
    let filename = storage_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            AppError::internal(
                "Write document source",
                "Managed document path has no file name",
                "Provide a valid destination filename.",
            )
        })?;
    let temporary_path = parent.join(format!(".{filename}.{}.tmp", uuid::Uuid::now_v7()));

    let result = (|| -> std::io::Result<()> {
        let mut source = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)?;
        source.write_all(bytes)?;
        source.sync_all()?;
        std::fs::rename(&temporary_path, storage_path)?;
        let _ = std::fs::File::open(parent).and_then(|directory| directory.sync_all());
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }

    result.map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_utf8_pdf_filename_in_typed_upload() {
        let pdf = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\ntrailer\n<<>>\n%%EOF";
        let extension = validate_upload(
            "Dokumentacja techniczna \u{2014} \u{017c}\u{00f3}\u{0142}w.pdf",
            pdf,
        )
        .expect("utf-8 filename should be accepted");
        assert_eq!(extension, "pdf");
    }

    #[test]
    fn accepts_pdf_signature_in_short_payload() {
        let payload = b"%PDF-";
        let extension = validate_upload("short.pdf", payload).expect("short PDF prefix is valid");
        assert_eq!(extension, "pdf");
    }

    #[test]
    fn preserves_safe_source_extensions_in_storage() {
        assert_eq!(storage_extension("advisor.md", "text"), "md");
        assert_eq!(storage_extension("notes", "text"), "text");
        assert_eq!(storage_extension("manual.DOCX", "docx"), "docx");
    }

    #[test]
    fn recognizes_only_managed_document_source_filenames() {
        let valid_id = uuid::Uuid::now_v7().to_string();
        assert!(is_managed_document_source_filename(&format!(
            "{valid_id}.pdf"
        )));
        assert!(is_managed_document_source_filename(&format!(
            "{valid_id}.docx"
        )));
        assert!(is_managed_document_source_filename(&format!(
            "{valid_id}.txt"
        )));
        assert!(!is_managed_document_source_filename("not-a-uuid.pdf"));
        assert!(!is_managed_document_source_filename(&valid_id));
        assert!(!is_managed_document_source_filename(&format!(
            "{valid_id}."
        )));
        assert!(!is_managed_document_source_filename(&format!(
            "{valid_id}.toolongextension123"
        )));
    }

    #[test]
    fn validates_managed_storage_filename_cross_platform() {
        let valid_id = uuid::Uuid::now_v7().to_string();
        let valid_path = format!("documents/{valid_id}.pdf");
        assert_eq!(
            managed_storage_filename(&valid_path).expect("valid path"),
            format!("{valid_id}.pdf")
        );

        let valid_win_path = format!("documents\\{valid_id}.pdf");
        assert_eq!(
            managed_storage_filename(&valid_win_path).expect("valid windows path"),
            format!("{valid_id}.pdf")
        );

        assert!(managed_storage_filename(&format!("{valid_id}.pdf")).is_err());
        assert!(managed_storage_filename(&format!("documents/../{valid_id}.pdf")).is_err());
        assert!(managed_storage_filename(&format!("documents/nested/{valid_id}.pdf")).is_err());
    }

    #[test]
    fn classifies_utf8_text_and_docx_uploads() {
        let text = "Hello world\nThis is plain text documentation.".as_bytes();
        assert_eq!(
            validate_upload("notes.txt", text).expect("text upload"),
            "text"
        );

        let docx_sig = b"PK\x03\x04\x14\x00\x06\x00";
        assert_eq!(
            validate_upload("specs.docx", docx_sig).expect("docx upload"),
            "docx"
        );

        let bad_docx = b"NOT_A_ZIP";
        assert!(validate_upload("specs.docx", bad_docx).is_err());
    }
}
