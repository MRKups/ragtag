//! Credential mutations with explicit partial-failure reporting.

use domain::{ApiKeyStatus, ApiKeyStorage, AppError};
use persistence::AppSettingsRepository;

const KEY: &str = "external_api_key";
// Credential mutations span blocking keychain and async SQLite/lifecycle work.
pub(crate) static MUTATIONS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn storage_status(keychain_has_key: bool, database_has_key: bool) -> ApiKeyStatus {
    // A plaintext copy must remain visible even when a keychain copy also exists
    // after a partially completed migration. Inference remains keychain-first.
    let storage = if database_has_key {
        ApiKeyStorage::Database
    } else if keychain_has_key {
        ApiKeyStorage::Keychain
    } else {
        ApiKeyStorage::None
    };
    ApiKeyStatus {
        has_key: keychain_has_key || database_has_key,
        storage,
    }
}

pub async fn save_api_key(
    settings: &AppSettingsRepository,
    key: String,
) -> Result<ApiKeyStatus, AppError> {
    let key_for_keychain = key.clone();
    save_api_key_with(settings, &key, move || {
        let entry = keyring::v1::Entry::new("ragtag", KEY).map_err(|_| ())?;
        entry.set_password(&key_for_keychain).map_err(|_| ())
    })
    .await
}

async fn save_api_key_with(
    settings: &AppSettingsRepository,
    key: &str,
    write: impl FnOnce() -> Result<(), ()> + Send + 'static,
) -> Result<ApiKeyStatus, AppError> {
    let storage = match tokio::task::spawn_blocking(write).await {
        Ok(Ok(())) => {
            settings.delete(KEY).await.map_err(|_| {
                AppError::storage(
                    "Save API key",
                    "The key was saved in the OS keychain, but its plaintext database copy could not be removed",
                    "Restore database access and save the key again to remove the plaintext copy.",
                )
            })?;
            ApiKeyStorage::Keychain
        }
        Ok(Err(())) => {
            log::warn!(
                "System keychain is unavailable; falling back to SQLite storage for API key"
            );
            settings.set(KEY, key).await.map_err(|_| {
                AppError::storage(
                    "Save API key",
                    "The OS keychain is unavailable and the database could not store the key",
                    "Unlock the OS keychain or restore database access, then save the key again.",
                )
            })?;
            ApiKeyStorage::Database
        }
        Err(_) => {
            return Err(AppError::internal(
                "Save API key",
                "The keychain worker did not complete; credential storage could not be confirmed",
                "Restart the application and save the key again.",
            ));
        }
    };
    Ok(ApiKeyStatus {
        has_key: true,
        storage,
    })
}

pub async fn remove_api_key(settings: &AppSettingsRepository) -> Result<bool, AppError> {
    remove_api_key_with(settings, || {
        let entry = keyring::v1::Entry::new("ragtag", KEY).map_err(|_| ())?;
        match entry.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::v1::Error::NoEntry) => Ok(false),
            Err(_) => Err(()),
        }
    })
    .await
}

async fn remove_api_key_with(
    settings: &AppSettingsRepository,
    remove: impl FnOnce() -> Result<bool, ()> + Send + 'static,
) -> Result<bool, AppError> {
    let keychain = tokio::task::spawn_blocking(remove).await;
    // Always attempt both stores, even if one cannot complete. Never describe
    // partial removal as success or include backend error/panic text in IPC.
    let database = settings.delete(KEY).await;
    match (keychain, database) {
        (Ok(Ok(keychain_removed)), Ok(database_removed)) => {
            Ok(keychain_removed || database_removed)
        }
        (_, Err(_)) => Err(AppError::storage(
            "Delete API key",
            "Credential cleanup is incomplete; the database copy could not be removed",
            "Restore database access and retry deletion. Check the OS keychain if its copy remains.",
        )),
        (Ok(Err(())), Ok(_)) => Err(AppError::storage(
            "Delete API key",
            "Database cleanup completed, but OS keychain removal could not be confirmed",
            "Unlock the OS keychain and retry deletion, or remove Ragtag's external_api_key entry in the system keychain manager.",
        )),
        (Err(_), Ok(_)) => Err(AppError::internal(
            "Delete API key",
            "Database cleanup completed, but the keychain cleanup worker did not complete",
            "Restart the application and retry deletion before assuming the key was removed.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::ErrorCode;

    #[test]
    fn plaintext_copy_takes_precedence_in_storage_warning_status() {
        let status = storage_status(true, true);
        assert!(status.has_key);
        assert_eq!(status.storage, ApiKeyStorage::Database);
        assert_eq!(storage_status(true, false).storage, ApiKeyStorage::Keychain);
        assert_eq!(storage_status(false, true).storage, ApiKeyStorage::Database);
        let absent = storage_status(false, false);
        assert!(!absent.has_key);
        assert_eq!(absent.storage, ApiKeyStorage::None);
    }

    async fn database() -> (std::path::PathBuf, sqlx::SqlitePool, AppSettingsRepository) {
        let dir = std::env::temp_dir().join(format!("ragtag-credentials-{}", uuid::Uuid::now_v7()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let pool = persistence::init_pool(&dir).await.unwrap();
        let settings = AppSettingsRepository::new(pool.clone());
        (dir, pool, settings)
    }

    async fn finish(dir: std::path::PathBuf, pool: sqlx::SqlitePool) {
        pool.close().await;
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    async fn prevent_deletion(pool: &sqlx::SqlitePool) {
        sqlx::query("CREATE TRIGGER prevent_key_deletion BEFORE DELETE ON app_settings BEGIN SELECT RAISE(ABORT, 'test deletion failure'); END")
            .execute(pool).await.unwrap();
    }

    #[tokio::test]
    async fn keychain_save_removes_plaintext_fallback() {
        let (dir, pool, settings) = database().await;
        settings.set(KEY, "dummy-old-key").await.unwrap();
        let status = save_api_key_with(&settings, "dummy-new-key", || Ok(()))
            .await
            .unwrap();
        assert_eq!(status.storage, ApiKeyStorage::Keychain);
        assert!(settings.get(KEY).await.unwrap().is_none());
        finish(dir, pool).await;
    }

    #[tokio::test]
    async fn unavailable_keychain_still_uses_documented_database_fallback() {
        let (dir, pool, settings) = database().await;
        let status = save_api_key_with(&settings, "dummy-key", || Err(()))
            .await
            .unwrap();
        assert_eq!(status.storage, ApiKeyStorage::Database);
        assert_eq!(
            settings.get(KEY).await.unwrap().as_deref(),
            Some("dummy-key")
        );
        finish(dir, pool).await;
    }

    #[tokio::test]
    async fn keychain_migration_reports_database_cleanup_failure() {
        let (dir, pool, settings) = database().await;
        settings.set(KEY, "dummy-plaintext").await.unwrap();
        prevent_deletion(&pool).await;
        let error = save_api_key_with(&settings, "dummy-new-key", || Ok(()))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Storage);
        assert!(error.detail.contains("plaintext database copy"));
        assert!(!error.to_string().contains("dummy-plaintext"));
        assert_eq!(
            settings.get(KEY).await.unwrap().as_deref(),
            Some("dummy-plaintext")
        );
        finish(dir, pool).await;
    }

    #[tokio::test]
    async fn removal_reports_keychain_failure_after_removing_database_copy() {
        let (dir, pool, settings) = database().await;
        settings.set(KEY, "dummy-key").await.unwrap();
        let error = remove_api_key_with(&settings, || Err(()))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Storage);
        assert!(settings.get(KEY).await.unwrap().is_none());
        finish(dir, pool).await;
    }

    #[tokio::test]
    async fn removal_reports_database_failure_after_keychain_success() {
        let (dir, pool, settings) = database().await;
        settings.set(KEY, "dummy-key").await.unwrap();
        prevent_deletion(&pool).await;
        let error = remove_api_key_with(&settings, || Ok(true))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Storage);
        assert!(settings.get(KEY).await.unwrap().is_some());
        finish(dir, pool).await;
    }

    #[tokio::test]
    async fn deletion_is_idempotent_and_reports_whether_any_copy_existed() {
        let (dir, pool, settings) = database().await;
        assert!(!remove_api_key_with(&settings, || Ok(false)).await.unwrap());
        settings.set(KEY, "dummy-key").await.unwrap();
        assert!(remove_api_key_with(&settings, || Ok(false)).await.unwrap());
        assert!(!remove_api_key_with(&settings, || Ok(false)).await.unwrap());
        assert!(remove_api_key_with(&settings, || Ok(true)).await.unwrap());
        finish(dir, pool).await;
    }

    #[tokio::test]
    async fn database_fallback_failure_is_typed_and_does_not_expose_the_key() {
        let (dir, pool, settings) = database().await;
        pool.close().await;
        let error = save_api_key_with(&settings, "dummy-secret", || Err(()))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Storage);
        assert!(!error.to_string().contains("dummy-secret"));
        finish(dir, pool).await;
    }
}
