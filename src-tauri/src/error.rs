// error.rs — Unified error type for all Tauri IPC commands
//
// All internal errors convert to AppError via From impls.
// AppError serializes as a string for Tauri IPC transport.

use uuid::Uuid;

use crate::state::{SessionId, TerminalId};
use thiserror::Error;

// ─── Profile-Domain Error ────────────────────────────────
//
// Fine-grained error enum for folder/profile CRUD operations on
// `ProfilesEnvelope`. Phase 4 Tauri commands map these to `AppError`
// via `From<ProfileError> for AppError`.

#[derive(Error, Debug, PartialEq)]
pub enum ProfileError {
    /// Folder not found by the given UUID.
    #[error("Folder not found")]
    FolderNotFound,
    /// Profile not found by the given UUID.
    #[error("Profile not found")]
    ProfileNotFound,
    /// Operation rejected: the system folder cannot be renamed or deleted.
    #[error("System folder is protected and cannot be modified")]
    SystemFolderProtected,
    /// Name is empty, whitespace-only, or exceeds 64 characters.
    #[error("Folder name is invalid (must be 1–64 non-whitespace characters)")]
    InvalidName,
    /// A folder with the same name (case-insensitive) already exists.
    #[error("A folder with this name already exists")]
    DuplicateName,
    /// Reorder vector does not include all existing IDs (subset provided).
    #[error("Reorder list is incomplete — all IDs must be present")]
    IncompleteReorder,
    /// Profile belongs to a different folder than the one being reordered.
    #[error("Profile belongs to a different folder")]
    ProfileFolderMismatch,
}

impl From<ProfileError> for AppError {
    fn from(e: ProfileError) -> Self {
        AppError::ProfileError(e.to_string())
    }
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("SSH error: {0}")]
    Ssh(#[from] russh::Error),

    #[error("SFTP error: {0}")]
    Sftp(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Session not found: {0}")]
    SessionNotFound(SessionId),

    #[error("Not connected")]
    NotConnected,

    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("Host key verification failed")]
    HostKeyRejected,

    #[error("Terminal not found: {0}")]
    TerminalNotFound(TerminalId),

    #[error("Tunnel error: {0}")]
    TunnelError(String),

    #[error("Profile error: {0}")]
    ProfileError(String),

    #[error("Keychain error: {0}")]
    KeychainError(String),

    #[error("Vault error: {0}")]
    VaultError(String),

    #[error("Vault is locked — unlock with master password first")]
    VaultLocked,

    #[error("Wrong master password")]
    VaultWrongPassword,

    #[error("Transfer cancelled")]
    TransferCancelled,

    #[error("Key error: {0}")]
    KeyError(String),

    #[error("Connection timeout")]
    ConnectionTimeout,

    #[error("User selection required: profile has multiple users — provide a userId")]
    UserSelectionRequired,

    #[error("User not found: {0}")]
    UserNotFound(Uuid),

    #[error("{0}")]
    Other(String),
}

// Serialize as string for Tauri IPC — Tauri requires Serialize on error types
impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::Other(format!("JSON error: {err}"))
    }
}

impl From<uuid::Error> for AppError {
    fn from(err: uuid::Error) -> Self {
        AppError::Other(format!("UUID error: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_serializes_to_string() {
        let err = AppError::NotConnected;
        let serialized = serde_json::to_string(&err).unwrap();
        assert_eq!(serialized, "\"Not connected\"");
    }

    #[test]
    fn auth_failed_serializes_with_message() {
        let err = AppError::AuthFailed("wrong password".to_string());
        let serialized = serde_json::to_string(&err).unwrap();
        assert_eq!(serialized, "\"Authentication failed: wrong password\"");
    }

    #[test]
    fn session_not_found_serializes() {
        let id = uuid::Uuid::nil();
        let err = AppError::SessionNotFound(id);
        let serialized = serde_json::to_string(&err).unwrap();
        assert!(serialized.contains("Session not found"));
    }

    #[test]
    fn io_error_converts() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let app_err: AppError = io_err.into();
        assert!(matches!(app_err, AppError::Io(_)));
    }

    #[test]
    fn all_variants_serialize() {
        let id = uuid::Uuid::nil();
        let variants: Vec<AppError> = vec![
            AppError::Sftp("test".into()),
            AppError::Io(std::io::Error::new(std::io::ErrorKind::Other, "test")),
            AppError::SessionNotFound(id),
            AppError::NotConnected,
            AppError::AuthFailed("test".into()),
            AppError::HostKeyRejected,
            AppError::TerminalNotFound(id),
            AppError::TunnelError("test".into()),
            AppError::ProfileError("test".into()),
            AppError::KeychainError("test".into()),
            AppError::VaultError("test".into()),
            AppError::VaultLocked,
            AppError::VaultWrongPassword,
            AppError::TransferCancelled,
            AppError::KeyError("test".into()),
            AppError::ConnectionTimeout,
            AppError::UserSelectionRequired,
            AppError::UserNotFound(id),
            AppError::Other("test".into()),
        ];

        for variant in variants {
            let result = serde_json::to_string(&variant);
            assert!(result.is_ok(), "Failed to serialize: {:?}", variant);
        }
    }
}
