pub mod documents;
pub mod maintenance;
pub mod reconciliation;

pub use documents::*;
pub use maintenance::*;
pub use reconciliation::*;

pub(crate) fn validate_app_data_directory(path: &std::path::Path) -> Result<(), domain::AppError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        domain::AppError::storage(
            "Inspect application data directory",
            format!("Could not inspect app data directory: {error}"),
            "Check that the application local data directory exists and is accessible.",
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(domain::AppError::storage(
            "Inspect application data directory",
            "App data directory must be a real directory, not a symlink",
            "Ensure the application data directory is a regular folder.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_data_validation_preserves_missing_file_and_symlink_errors() {
        let root = std::env::temp_dir().join(format!(
            "ragtag-directory-validation-{}",
            uuid::Uuid::now_v7()
        ));
        std::fs::create_dir_all(&root).unwrap();
        validate_app_data_directory(&root).unwrap();

        let missing = validate_app_data_directory(&root.join("missing")).unwrap_err();
        assert_eq!(missing.code, domain::ErrorCode::Storage);
        assert_eq!(missing.operation, "Inspect application data directory");
        assert_eq!(
            missing.recovery,
            "Check that the application local data directory exists and is accessible."
        );

        let file = root.join("file");
        std::fs::write(&file, b"not a directory").unwrap();
        let error = validate_app_data_directory(&file).unwrap_err();
        assert_eq!(
            error.detail,
            "App data directory must be a real directory, not a symlink"
        );
        #[cfg(unix)]
        {
            let link = root.join("link");
            std::os::unix::fs::symlink(&root, &link).unwrap();
            assert_eq!(validate_app_data_directory(&link).unwrap_err(), error);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
