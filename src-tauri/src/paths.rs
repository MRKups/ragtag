use domain::AppError;
use std::path::PathBuf;
use tauri::Manager;

pub(crate) fn app_local_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, AppError> {
    resolve_app_directory(
        app,
        "Resolve storage path",
        "Verify application local storage permissions.",
    )
}

pub(crate) fn models_dir(app: &tauri::AppHandle, recovery: &str) -> Result<PathBuf, AppError> {
    resolve_app_directory(app, "Resolve models directory", recovery)
        .map(|directory| directory.join("models"))
}

fn resolve_app_directory(
    app: &tauri::AppHandle,
    operation: &str,
    recovery: &str,
) -> Result<PathBuf, AppError> {
    app.path().app_local_data_dir().map_err(|error| {
        AppError::storage(
            operation,
            format!("Failed to resolve app local data directory: {error}"),
            recovery,
        )
    })
}
