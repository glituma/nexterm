// commands/profile.rs — Profile CRUD + Folder CRUD Tauri commands
//
// Handles: save_profile, load_profiles, load_profiles_with_folders,
//          delete_profile, get_profile, export_profiles, import_profiles,
//          create_folder, rename_folder, delete_folder, reorder_folders,
//          move_profile_to_folder, reorder_profiles_in_folder, set_folder_expanded
//
// Credential storage is now handled by commands/vault.rs via the encrypted vault.

use serde::Serialize;
use tauri::{Manager, State};
use uuid::Uuid;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use rand::RngCore;

use crate::error::AppError;
use crate::profile::{
    self, AuthMethodConfig, ConnectionProfile, DeleteFolderResult, Folder, ProfilesEnvelope,
    UserCredential,
};
use crate::state::{AppState, SessionState};

// ─── Export/Import result types ─────────────────────────

/// Result returned by `export_profiles` over the Tauri IPC bridge.
///
/// `count` is the number of profiles written. `warnings` carries zero or more
/// stable string identifiers (NOT translation keys) that the frontend maps to
/// localised messages. Currently defined warning: `"acl_not_applied"`.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub count: u32,
    pub warnings: Vec<String>,
}

/// Build an `ExportResult` from a profile count and a `BestEffortOutcome`.
///
/// Centralises the "did hardening succeed?" → warning-string mapping so the
/// logic can be unit-tested without touching the async Tauri command surface.
///
/// When hardening fails with an unexpected error, the inner `io::Error` is
/// logged at `warn!` level with export-flow context (the generic
/// `best_effort_harden` call site already logs, but this adds the "during
/// export" framing so operators reading the logs know which write path the
/// failure belongs to).
pub(crate) fn build_export_result(
    count: u32,
    outcome: crate::fs_secure::BestEffortOutcome,
) -> ExportResult {
    use crate::fs_secure::BestEffortOutcome;
    let mut warnings = Vec::new();
    match outcome {
        BestEffortOutcome::Hardened => {}
        BestEffortOutcome::SkippedUnsupported => {
            warnings.push("acl_not_applied".to_string());
        }
        BestEffortOutcome::Failed(err) => {
            tracing::warn!(
                error = %err,
                "export file hardening failed; surfacing acl_not_applied warning to user"
            );
            warnings.push("acl_not_applied".to_string());
        }
    }
    ExportResult { count, warnings }
}

// ─── Helpers ────────────────────────────────────────────

/// Get the app data dir from the Tauri app handle
fn get_app_data_dir(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_data_dir().ok()
}

// ─── Profile CRUD Commands ──────────────────────────────

/// Save profile command.
///
/// Creates or updates a profile. Uses `save_profiles_envelope` so every
/// write is envelope format — eliminating the legacy re-migration cycle.
///
/// Arg validation: `profile_data` is validated by `ConnectionProfile::validate`.
/// Error cases: invalid profile data, disk write failure.
#[tauri::command]
pub async fn save_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mut profile_data: ConnectionProfile,
) -> Result<Uuid, AppError> {
    profile_data.validate()?;
    profile_data.updated_at = chrono::Utc::now();

    let app_data_dir = get_app_data_dir(&app);

    let mut profiles = state.profiles.lock().await;
    let folders = state.folders.lock().await;

    // Check if updating existing or creating new
    if let Some(existing) = profiles.iter_mut().find(|p| p.id == profile_data.id) {
        // Preserve display_order on update unless explicitly changed
        if profile_data.display_order == 0 && existing.display_order != 0 {
            profile_data.display_order = existing.display_order;
        }
        *existing = profile_data.clone();
    } else {
        // New profile — assign next available display_order
        let max_order = profiles.iter().map(|p| p.display_order).max().unwrap_or(0);
        profile_data.display_order = max_order + 1;
        profile_data.created_at = chrono::Utc::now();
        // Assign to system folder if no folder_id specified
        if profile_data.folder_id.is_none() {
            if let Some(sys_folder) = folders.iter().find(|f| f.is_system) {
                profile_data.folder_id = Some(sys_folder.id);
            }
        }
        profiles.push(profile_data.clone());
    }

    // Build envelope from updated state and persist
    let envelope = ProfilesEnvelope {
        folders: folders.clone(),
        profiles: profiles.clone(),
    };
    profile::save_profiles_envelope(&envelope, app_data_dir.as_ref())?;

    Ok(profile_data.id)
}

#[tauri::command]
pub async fn load_profiles(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<ConnectionProfile>, AppError> {
    let app_data_dir = get_app_data_dir(&app);
    // Phase 2: load_profiles_from_disk now returns ProfilesEnvelope.
    // This command retains its Vec<ConnectionProfile> return type for backward
    // compat with the existing frontend. Phase 4 will add `load_profiles_with_folders`
    // which exposes the full envelope.
    let envelope = profile::load_profiles_from_disk(app_data_dir.as_ref())?;

    // Sync in-memory state — both profiles and folders
    let mut profiles = state.profiles.lock().await;
    *profiles = envelope.profiles.clone();
    drop(profiles);

    let mut folders = state.folders.lock().await;
    *folders = envelope.folders.clone();
    drop(folders);

    Ok(envelope.profiles)
}

#[tauri::command]
pub async fn delete_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    profile_id: Uuid,
) -> Result<(), AppError> {
    // Check for active sessions using this profile before deleting
    {
        let sessions = state.sessions.lock().await;
        for session in sessions.values() {
            if session.profile.id == profile_id
                && !matches!(session.state, SessionState::Disconnected)
            {
                return Err(AppError::ProfileError(format!(
                    "Cannot delete profile: it has an active session ({}). Disconnect first.",
                    session.id
                )));
            }
        }
    } // sessions lock dropped here before acquiring profiles lock

    let mut profiles = state.profiles.lock().await;
    let folders = state.folders.lock().await;
    let app_data_dir = get_app_data_dir(&app);

    let initial_len = profiles.len();
    profiles.retain(|p| p.id != profile_id);

    if profiles.len() == initial_len {
        return Err(AppError::ProfileError(format!(
            "Profile not found: {profile_id}"
        )));
    }

    // Persist as envelope (no more legacy flat-array)
    let envelope = ProfilesEnvelope {
        folders: folders.clone(),
        profiles: profiles.clone(),
    };
    profile::save_profiles_envelope(&envelope, app_data_dir.as_ref())?;

    // Clean up vault credentials for this profile (best-effort)
    drop(profiles); // release profiles lock before acquiring vault lock
    drop(folders);  // release folders lock too
    let _ = crate::commands::vault::delete_profile_credentials(&state, &profile_id).await;

    Ok(())
}

#[tauri::command]
pub async fn get_profile(
    state: State<'_, AppState>,
    profile_id: Uuid,
) -> Result<ConnectionProfile, AppError> {
    let profiles = state.profiles.lock().await;
    profiles
        .iter()
        .find(|p| p.id == profile_id)
        .cloned()
        .ok_or_else(|| AppError::ProfileError(format!("Profile not found: {profile_id}")))
}

// ─── New Phase 4: load_profiles_with_folders ────────

/// Load the full `ProfilesEnvelope` (folders + profiles) from the current
/// in-memory state, triggering a disk load if state is empty.
///
/// Returns the full envelope so the frontend can render the folder tree.
/// This command supplements `load_profiles` — the existing command is kept
/// for backward compat and still returns `Vec<ConnectionProfile>`.
///
/// Error cases: disk read/parse failure.
#[tauri::command]
pub async fn load_profiles_with_folders(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<ProfilesEnvelope, AppError> {
    let app_data_dir = get_app_data_dir(&app);

    // If state not yet loaded, trigger disk load (same as load_profiles)
    {
        let profiles = state.profiles.lock().await;
        let folders = state.folders.lock().await;
        if profiles.is_empty() && folders.is_empty() {
            // State not loaded yet — fall through to disk load below
            drop(profiles);
            drop(folders);
        } else {
            let envelope = ProfilesEnvelope {
                folders: folders.clone(),
                profiles: profiles.clone(),
            };
            return Ok(envelope);
        }
    }

    // Disk load + populate state
    let envelope = profile::load_profiles_from_disk(app_data_dir.as_ref())?;

    let mut profiles = state.profiles.lock().await;
    let mut folders = state.folders.lock().await;
    *profiles = envelope.profiles.clone();
    *folders = envelope.folders.clone();

    Ok(envelope)
}

// ─── New Phase 4: Folder CRUD Commands ──────────────

/// Create a new user folder with the given name.
///
/// Validation: name trimmed, 1–64 chars, no case-insensitive duplicate.
/// Rollback: on persist failure, in-memory state is restored from snapshot.
///
/// Error cases: `InvalidName`, `DuplicateName`, disk write failure.
#[tauri::command]
pub async fn create_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<Folder, AppError> {
    let app_data_dir = get_app_data_dir(&app);

    let mut profiles = state.profiles.lock().await;
    let mut folders = state.folders.lock().await;

    // Build envelope + snapshot for rollback
    let mut envelope = ProfilesEnvelope {
        folders: folders.clone(),
        profiles: profiles.clone(),
    };
    let snapshot = envelope.clone();

    // Mutate via pure method
    let new_folder = envelope.create_folder(name).map_err(AppError::from)?;

    // Persist — rollback on failure
    if let Err(e) = profile::save_profiles_envelope(&envelope, app_data_dir.as_ref()) {
        // Restore in-memory state from snapshot
        *folders = snapshot.folders;
        *profiles = snapshot.profiles;
        return Err(e);
    }

    // Write back to AppState
    *folders = envelope.folders;
    *profiles = envelope.profiles;

    Ok(new_folder)
}

/// Rename an existing user folder.
///
/// Validation: name trimmed, 1–64 chars, no case-insensitive duplicate.
/// Rejects: system folders.
/// Rollback: on persist failure, in-memory state is restored from snapshot.
///
/// Error cases: `FolderNotFound`, `SystemFolderProtected`, `InvalidName`,
/// `DuplicateName`, disk write failure.
#[tauri::command]
pub async fn rename_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    folder_id: Uuid,
    new_name: String,
) -> Result<Folder, AppError> {
    let app_data_dir = get_app_data_dir(&app);

    let mut profiles = state.profiles.lock().await;
    let mut folders = state.folders.lock().await;

    let mut envelope = ProfilesEnvelope {
        folders: folders.clone(),
        profiles: profiles.clone(),
    };
    let snapshot = envelope.clone();

    let renamed_folder = envelope.rename_folder(folder_id, new_name).map_err(AppError::from)?;

    if let Err(e) = profile::save_profiles_envelope(&envelope, app_data_dir.as_ref()) {
        *folders = snapshot.folders;
        *profiles = snapshot.profiles;
        return Err(e);
    }

    *folders = envelope.folders;
    *profiles = envelope.profiles;

    Ok(renamed_folder)
}

/// Delete a user folder.
///
/// If the folder contains profiles, they are moved to the system folder
/// preserving relative order. Returns `DeleteFolderResult` with the count
/// of moved profiles.
/// Rejects: system folders, non-existent UUIDs.
/// Rollback: on persist failure, in-memory state is restored from snapshot.
///
/// Error cases: `FolderNotFound`, `SystemFolderProtected`, disk write failure.
#[tauri::command]
pub async fn delete_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    folder_id: Uuid,
) -> Result<DeleteFolderResult, AppError> {
    let app_data_dir = get_app_data_dir(&app);

    let mut profiles = state.profiles.lock().await;
    let mut folders = state.folders.lock().await;

    let mut envelope = ProfilesEnvelope {
        folders: folders.clone(),
        profiles: profiles.clone(),
    };
    let snapshot = envelope.clone();

    let result = envelope.delete_folder(folder_id).map_err(AppError::from)?;

    if let Err(e) = profile::save_profiles_envelope(&envelope, app_data_dir.as_ref()) {
        *folders = snapshot.folders;
        *profiles = snapshot.profiles;
        return Err(e);
    }

    *folders = envelope.folders;
    *profiles = envelope.profiles;

    Ok(result)
}

/// Reorder all folders.
///
/// `ordered_ids` must contain exactly the same UUIDs as the current folder list.
/// Each folder's `display_order` is set to its index in `ordered_ids`.
/// Rollback: on persist failure, in-memory state is restored from snapshot.
///
/// Error cases: `FolderNotFound` (unknown UUID), `IncompleteReorder` (missing UUID),
/// disk write failure.
#[tauri::command]
pub async fn reorder_folders(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ordered_ids: Vec<Uuid>,
) -> Result<(), AppError> {
    let app_data_dir = get_app_data_dir(&app);

    let mut profiles = state.profiles.lock().await;
    let mut folders = state.folders.lock().await;

    let mut envelope = ProfilesEnvelope {
        folders: folders.clone(),
        profiles: profiles.clone(),
    };
    let snapshot = envelope.clone();

    envelope.reorder_folders(ordered_ids).map_err(AppError::from)?;

    if let Err(e) = profile::save_profiles_envelope(&envelope, app_data_dir.as_ref()) {
        *folders = snapshot.folders;
        *profiles = snapshot.profiles;
        return Err(e);
    }

    *folders = envelope.folders;
    *profiles = envelope.profiles;

    Ok(())
}

/// Move a profile to a different folder (or reorder within the same folder).
///
/// Cross-folder: profile's `folder_id` is updated; siblings in target folder
/// with `display_order >= new_order` are shifted by +1.
/// Same-folder: sorted-list reinsert with sequential display_order compaction.
/// Rollback: on persist failure, in-memory state is restored from snapshot.
///
/// Error cases: `FolderNotFound`, `ProfileNotFound`, disk write failure.
#[tauri::command]
pub async fn move_profile_to_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    profile_id: Uuid,
    target_folder_id: Uuid,
    new_order: i32,
) -> Result<(), AppError> {
    let app_data_dir = get_app_data_dir(&app);

    let mut profiles = state.profiles.lock().await;
    let mut folders = state.folders.lock().await;

    let mut envelope = ProfilesEnvelope {
        folders: folders.clone(),
        profiles: profiles.clone(),
    };
    let snapshot = envelope.clone();

    envelope
        .move_profile_to_folder(profile_id, target_folder_id, new_order)
        .map_err(AppError::from)?;

    if let Err(e) = profile::save_profiles_envelope(&envelope, app_data_dir.as_ref()) {
        *folders = snapshot.folders;
        *profiles = snapshot.profiles;
        return Err(e);
    }

    *folders = envelope.folders;
    *profiles = envelope.profiles;

    Ok(())
}

/// Reorder all profiles within a specific folder.
///
/// `ordered_profile_ids` must contain exactly the same profile UUIDs that
/// currently belong to `folder_id`.
/// Rollback: on persist failure, in-memory state is restored from snapshot.
///
/// Error cases: `FolderNotFound`, `ProfileNotFound`, `ProfileFolderMismatch`,
/// `IncompleteReorder`, disk write failure.
#[tauri::command]
pub async fn reorder_profiles_in_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    folder_id: Uuid,
    ordered_profile_ids: Vec<Uuid>,
) -> Result<(), AppError> {
    let app_data_dir = get_app_data_dir(&app);

    let mut profiles = state.profiles.lock().await;
    let mut folders = state.folders.lock().await;

    let mut envelope = ProfilesEnvelope {
        folders: folders.clone(),
        profiles: profiles.clone(),
    };
    let snapshot = envelope.clone();

    envelope
        .reorder_profiles_in_folder(folder_id, ordered_profile_ids)
        .map_err(AppError::from)?;

    if let Err(e) = profile::save_profiles_envelope(&envelope, app_data_dir.as_ref()) {
        *folders = snapshot.folders;
        *profiles = snapshot.profiles;
        return Err(e);
    }

    *folders = envelope.folders;
    *profiles = envelope.profiles;

    Ok(())
}

/// Set the `is_expanded` state of a folder (persisted to disk).
///
/// Idempotent: calling with the same value is a no-op but does not error.
/// Rollback: on persist failure, in-memory state is restored from snapshot.
///
/// Error cases: `FolderNotFound`, disk write failure.
#[tauri::command]
pub async fn set_folder_expanded(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    folder_id: Uuid,
    expanded: bool,
) -> Result<(), AppError> {
    let app_data_dir = get_app_data_dir(&app);

    let mut profiles = state.profiles.lock().await;
    let mut folders = state.folders.lock().await;

    let mut envelope = ProfilesEnvelope {
        folders: folders.clone(),
        profiles: profiles.clone(),
    };
    let snapshot = envelope.clone();

    envelope
        .set_folder_expanded(folder_id, expanded)
        .map_err(AppError::from)?;

    if let Err(e) = profile::save_profiles_envelope(&envelope, app_data_dir.as_ref()) {
        *folders = snapshot.folders;
        *profiles = snapshot.profiles;
        return Err(e);
    }

    *folders = envelope.folders;
    *profiles = envelope.profiles;

    Ok(())
}

// ─── Reorder Profiles ───────────────────────────────

/// Reorder all profiles (legacy flat reorder — no folder context).
///
/// Updates `display_order` for each profile based on position in `profile_ids`.
/// Uses envelope persistence (no re-migration cycle).
///
/// Error cases: disk write failure.
#[tauri::command]
pub async fn reorder_profiles(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    profile_ids: Vec<Uuid>,
) -> Result<(), AppError> {
    let mut profiles = state.profiles.lock().await;
    let folders = state.folders.lock().await;
    let app_data_dir = get_app_data_dir(&app);

    // Update display_order based on the position in the provided list
    for (index, id) in profile_ids.iter().enumerate() {
        if let Some(profile) = profiles.iter_mut().find(|p| &p.id == id) {
            profile.display_order = index as i32;
        }
    }

    // Sort in-memory to match the new order
    profiles.sort_by_key(|p| p.display_order);

    // Persist as envelope
    let envelope = ProfilesEnvelope {
        folders: folders.clone(),
        profiles: profiles.clone(),
    };
    profile::save_profiles_envelope(&envelope, app_data_dir.as_ref())?;

    Ok(())
}

// ─── Export / Import ────────────────────────────────────

/// Exported user credential within a v2 export.
#[derive(Debug, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct ExportedUser {
    username: String,
    auth_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    private_key_path: Option<String>,
    #[serde(default)]
    is_default: bool,
    /// Password — only present in encrypted exports.
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
}

/// A single exported profile — safe metadata only, no secrets.
/// v1: had top-level `username`, `auth_method`, `password`
/// v2: has `users` array
#[derive(Debug, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct ExportedProfile {
    name: String,
    host: String,
    port: u16,
    /// v2 format: array of users
    #[serde(default)]
    users: Vec<ExportedUser>,
    /// Legacy v1 fields — used for importing old exports
    #[serde(default, skip_serializing)]
    username: Option<String>,
    #[serde(default, skip_serializing)]
    auth_method: Option<String>,
    #[serde(default, skip_serializing)]
    private_key_path: Option<String>,
    #[serde(default, skip_serializing)]
    password: Option<String>,
}

/// Top-level export envelope (plain JSON).
#[derive(Debug, Serialize, serde::Deserialize)]
struct ExportEnvelope {
    version: u32,
    app: String,
    exported_at: String,
    profiles: Vec<ExportedProfile>,
}

/// Encrypted export file format: magic + salt(32) + nonce(12) + ciphertext.
const ENCRYPTED_EXPORT_MAGIC: &[u8] = b"RMKT";
const EXPORT_SALT_SIZE: usize = 32;
const EXPORT_NONCE_SIZE: usize = 12;

/// Result returned to the frontend after import.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: u32,
    pub skipped: u32,
    pub errors: Vec<String>,
}

impl From<&ConnectionProfile> for ExportedProfile {
    fn from(p: &ConnectionProfile) -> Self {
        let users: Vec<ExportedUser> = p
            .users
            .iter()
            .map(|u| {
                let (auth_method, private_key_path) = match &u.auth_method {
                    AuthMethodConfig::Password => ("password".to_string(), None),
                    AuthMethodConfig::PublicKey {
                        private_key_path, ..
                    } => ("publickey".to_string(), Some(private_key_path.clone())),
                    AuthMethodConfig::KeyboardInteractive => {
                        ("keyboard-interactive".to_string(), None)
                    }
                };
                ExportedUser {
                    username: u.username.clone(),
                    auth_method,
                    private_key_path,
                    is_default: u.is_default,
                    password: None,
                }
            })
            .collect();
        ExportedProfile {
            name: p.name.clone(),
            host: p.host.clone(),
            port: p.port,
            users,
            // Legacy fields not serialized
            username: None,
            auth_method: None,
            private_key_path: None,
            password: None,
        }
    }
}

#[tauri::command]
pub async fn export_profiles(
    state: State<'_, AppState>,
    export_path: String,
    include_credentials: bool,
    export_password: Option<String>,
) -> Result<ExportResult, AppError> {
    let profiles = state.profiles.lock().await;

    if profiles.is_empty() {
        return Err(AppError::ProfileError("No profiles to export".to_string()));
    }

    if include_credentials && export_password.is_none() {
        return Err(AppError::ProfileError(
            "Export password is required when including credentials".to_string(),
        ));
    }

    let mut exported: Vec<ExportedProfile> = profiles.iter().map(ExportedProfile::from).collect();
    let count = exported.len() as u32;

    // If including credentials, read passwords from vault (per user)
    if include_credentials {
        let vault_guard = state.vault.lock().await;
        if let Some(ref vault) = *vault_guard {
            for (i, profile) in profiles.iter().enumerate() {
                for (j, user) in profile.users.iter().enumerate() {
                    // Try new key format first, then legacy
                    if let Ok(Some(password)) =
                        crate::commands::vault::get_credential_from_vault(
                            vault,
                            &profile.id,
                            Some(&user.id),
                            "password",
                        )
                    {
                        if j < exported[i].users.len() {
                            exported[i].users[j].password = Some(password);
                        }
                    }
                }
            }
        }
    }

    let envelope = ExportEnvelope {
        version: 2,
        app: "NexTerm".to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        profiles: exported,
    };

    let json = serde_json::to_string_pretty(&envelope)?;

    if include_credentials {
        // Encrypt and write as .nexterm file
        let password = export_password.unwrap();
        let encrypted = encrypt_export_data(json.as_bytes(), &password)?;
        std::fs::write(&export_path, &encrypted)
            .map_err(|e| AppError::ProfileError(format!("Failed to write export file: {e}")))?;
    } else {
        // Write as plain JSON
        std::fs::write(&export_path, &json)
            .map_err(|e| AppError::ProfileError(format!("Failed to write export file: {e}")))?;
    }

    // Best-effort ACL hardening on the export file.
    // The export path is user-chosen (may be FAT32, network share, etc.) so we
    // never fail the export on a hardening error — we just surface a warning.
    let harden_outcome = crate::fs_secure::best_effort_harden(std::path::Path::new(&export_path));
    Ok(build_export_result(count, harden_outcome))
}

#[tauri::command]
pub async fn import_profiles(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    import_path: String,
    import_password: Option<String>,
) -> Result<ImportResult, AppError> {
    // Detect file type by extension or magic bytes
    let raw_bytes = std::fs::read(&import_path)
        .map_err(|e| AppError::ProfileError(format!("Failed to read import file: {e}")))?;

    let is_encrypted = raw_bytes.starts_with(ENCRYPTED_EXPORT_MAGIC);

    let contents = if is_encrypted {
        let password = import_password.ok_or_else(|| {
            AppError::ProfileError("Import password is required for encrypted exports".to_string())
        })?;
        let decrypted = decrypt_export_data(&raw_bytes, &password)?;
        String::from_utf8(decrypted)
            .map_err(|e| AppError::ProfileError(format!("Invalid decrypted data: {e}")))?
    } else {
        String::from_utf8(raw_bytes)
            .map_err(|e| AppError::ProfileError(format!("Invalid file encoding: {e}")))?
    };

    let envelope: ExportEnvelope = serde_json::from_str(&contents).map_err(|e| {
        AppError::ProfileError(format!("Invalid import file format: {e}"))
    })?;

    if envelope.app != "NexTerm" {
        return Err(AppError::ProfileError(
            "File is not a NexTerm export".to_string(),
        ));
    }

    let mut profiles = state.profiles.lock().await;
    let app_data_dir = get_app_data_dir(&app);

    let mut imported: u32 = 0;
    let mut skipped: u32 = 0;
    let mut errors: Vec<String> = Vec::new();
    let mut credentials_to_store: Vec<(Uuid, Option<Uuid>, String)> = Vec::new();

    for ep in &envelope.profiles {
        // Duplicate check: same name + host
        let is_duplicate = profiles.iter().any(|existing| {
            existing.name == ep.name && existing.host == ep.host
        });

        if is_duplicate {
            skipped += 1;
            continue;
        }

        // Validate required fields
        if ep.name.trim().is_empty() || ep.host.trim().is_empty() {
            errors.push(format!("Skipped invalid profile: '{}'", ep.name));
            continue;
        }

        // Build users array — either from v2 `users` field or v1 legacy fields
        let users: Vec<UserCredential> = if !ep.users.is_empty() {
            // v2 format: reconstruct UserCredentials from exported users
            ep.users
                .iter()
                .map(|eu| {
                    let auth_method = match eu.auth_method.as_str() {
                        "publickey" => AuthMethodConfig::PublicKey {
                            private_key_path: eu
                                .private_key_path
                                .clone()
                                .unwrap_or_else(|| "~/.ssh/id_rsa".to_string()),
                            passphrase_in_keychain: false,
                        },
                        "keyboard-interactive" => AuthMethodConfig::KeyboardInteractive,
                        _ => AuthMethodConfig::Password,
                    };
                    UserCredential {
                        id: Uuid::new_v4(),
                        username: eu.username.clone(),
                        auth_method,
                        is_default: eu.is_default,
                    }
                })
                .collect()
        } else if let Some(ref username) = ep.username {
            // v1 format: single user from top-level fields
            if username.trim().is_empty() {
                errors.push(format!("Skipped invalid profile: '{}' (no username)", ep.name));
                continue;
            }
            let auth_method = match ep.auth_method.as_deref().unwrap_or("password") {
                "publickey" => AuthMethodConfig::PublicKey {
                    private_key_path: ep
                        .private_key_path
                        .clone()
                        .unwrap_or_else(|| "~/.ssh/id_rsa".to_string()),
                    passphrase_in_keychain: false,
                },
                "keyboard-interactive" => AuthMethodConfig::KeyboardInteractive,
                _ => AuthMethodConfig::Password,
            };
            vec![UserCredential {
                id: Uuid::new_v4(),
                username: username.clone(),
                auth_method,
                is_default: true,
            }]
        } else {
            errors.push(format!("Skipped profile with no users: '{}'", ep.name));
            continue;
        };

        let now = chrono::Utc::now();
        let new_id = Uuid::new_v4();
        let max_order = profiles.iter().map(|p| p.display_order).max().unwrap_or(0);
        let new_profile = ConnectionProfile {
            id: new_id,
            name: ep.name.clone(),
            host: ep.host.clone(),
            port: ep.port,
            username: None,
            auth_method: None,
            users: users.clone(),
            startup_directory: None,
            tunnels: Vec::new(),
            display_order: max_order + 1,
            folder_id: None,
            created_at: now,
            updated_at: now,
        };

        // Queue credential storage if passwords are present (v2: per-user, v1: single)
        for user in &users {
            // Check v2 user passwords
            if let Some(eu) = ep.users.iter().find(|eu| eu.username == user.username) {
                if let Some(ref password) = eu.password {
                    if !password.is_empty() {
                        credentials_to_store.push((new_id, Some(user.id), password.clone()));
                    }
                }
            }
        }
        // Also check v1 legacy password
        if ep.users.is_empty() {
            if let Some(ref password) = ep.password {
                if !password.is_empty() {
                    if let Some(user) = users.first() {
                        credentials_to_store.push((new_id, Some(user.id), password.clone()));
                    }
                }
            }
        }

        profiles.push(new_profile);
        imported += 1;
    }

    // Persist if anything was imported — use envelope format (no re-migration cycle)
    if imported > 0 {
        let folders = state.folders.lock().await;
        let envelope = ProfilesEnvelope {
            folders: folders.clone(),
            profiles: profiles.clone(),
        };
        drop(folders);
        profile::save_profiles_envelope(&envelope, app_data_dir.as_ref())?;
    }

    // Drop profiles lock before acquiring vault lock (avoid deadlock)
    drop(profiles);

    // Store imported credentials in vault
    if !credentials_to_store.is_empty() {
        let mut vault_guard = state.vault.lock().await;
        if let Some(ref mut vault) = *vault_guard {
            for (profile_id, user_id, password) in &credentials_to_store {
                let key = match user_id {
                    Some(uid) => format!("{profile_id}:{uid}:password"),
                    None => format!("{profile_id}:password"),
                };
                if let Err(e) = vault.store(&key, password) {
                    errors.push(format!("Failed to store credential: {e}"));
                }
            }
        }
    }

    Ok(ImportResult {
        imported,
        skipped,
        errors,
    })
}

// ─── Encryption Helpers ─────────────────────────────────

/// Derive a 32-byte key from password + salt using Argon2id.
fn derive_export_key(password: &str, salt: &[u8]) -> Result<[u8; 32], AppError> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| AppError::ProfileError(format!("Key derivation failed: {e}")))?;
    Ok(key)
}

/// Encrypt data for export: MAGIC(4) + salt(32) + nonce(12) + ciphertext.
fn encrypt_export_data(plaintext: &[u8], password: &str) -> Result<Vec<u8>, AppError> {
    let mut salt = [0u8; EXPORT_SALT_SIZE];
    OsRng.fill_bytes(&mut salt);

    let key = derive_export_key(password, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| AppError::ProfileError(format!("Cipher init failed: {e}")))?;

    let mut nonce_bytes = [0u8; EXPORT_NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| AppError::ProfileError(format!("Encryption failed: {e}")))?;

    let mut result = Vec::with_capacity(
        ENCRYPTED_EXPORT_MAGIC.len() + EXPORT_SALT_SIZE + EXPORT_NONCE_SIZE + ciphertext.len(),
    );
    result.extend_from_slice(ENCRYPTED_EXPORT_MAGIC);
    result.extend_from_slice(&salt);
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);

    Ok(result)
}

/// Decrypt an encrypted export file.
fn decrypt_export_data(data: &[u8], password: &str) -> Result<Vec<u8>, AppError> {
    let header_size = ENCRYPTED_EXPORT_MAGIC.len() + EXPORT_SALT_SIZE + EXPORT_NONCE_SIZE;
    if data.len() < header_size + 16 {
        return Err(AppError::ProfileError("Encrypted file is too short".to_string()));
    }

    let magic = &data[..ENCRYPTED_EXPORT_MAGIC.len()];
    if magic != ENCRYPTED_EXPORT_MAGIC {
        return Err(AppError::ProfileError("Not a valid encrypted export file".to_string()));
    }

    let salt_start = ENCRYPTED_EXPORT_MAGIC.len();
    let salt = &data[salt_start..salt_start + EXPORT_SALT_SIZE];

    let nonce_start = salt_start + EXPORT_SALT_SIZE;
    let nonce_bytes = &data[nonce_start..nonce_start + EXPORT_NONCE_SIZE];

    let ciphertext = &data[nonce_start + EXPORT_NONCE_SIZE..];

    let key = derive_export_key(password, salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| AppError::ProfileError(format!("Cipher init failed: {e}")))?;

    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| AppError::ProfileError("Wrong export password or corrupted file".to_string()))?;

    Ok(plaintext)
}

// ─── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_secure::BestEffortOutcome;
    use crate::profile::{DeleteFolderResult, Folder, ProfilesEnvelope};
    use std::io;

    // ── P7.1 / P7.2 — ExportResult shape and warning mapping ────────────────
    //
    // Strategy: we cannot call the async Tauri command directly in a unit test
    // (it requires AppState / Tauri internals). Instead we test the extracted
    // `build_export_result` helper which encapsulates the warning-mapping
    // seam. This covers R4 + R5 from the spec without Tauri ceremony.
    //
    // Deviation D11: command-level integration test (actual file + IPC round-trip)
    // deferred to manual/E2E verification (Phase 9). The helper function test is
    // the unit-level TDD gate.

    #[test]
    fn build_export_result_hardened_has_no_warnings() {
        // [RED written first; GREEN: `build_export_result` returns empty warnings
        //  when outcome is Hardened]
        let result = build_export_result(3, BestEffortOutcome::Hardened);
        assert_eq!(result.count, 3);
        assert!(
            result.warnings.is_empty(),
            "no warnings expected when ACL hardening succeeded"
        );
    }

    #[test]
    fn build_export_result_skipped_unsupported_emits_acl_not_applied() {
        // [RED: warns when SkippedUnsupported]
        let result = build_export_result(5, BestEffortOutcome::SkippedUnsupported);
        assert_eq!(result.count, 5);
        assert!(
            result.warnings.contains(&"acl_not_applied".to_string()),
            "expected 'acl_not_applied' warning for SkippedUnsupported outcome"
        );
    }

    #[test]
    fn build_export_result_failed_emits_acl_not_applied() {
        // [RED: warns when Failed]
        let err = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
        let result = build_export_result(2, BestEffortOutcome::Failed(err));
        assert_eq!(result.count, 2);
        assert!(
            result.warnings.contains(&"acl_not_applied".to_string()),
            "expected 'acl_not_applied' warning for Failed outcome"
        );
    }

    #[test]
    fn export_result_warning_string_is_stable_contract() {
        // Ensure the literal "acl_not_applied" string never changes accidentally.
        // Frontend depends on this exact string for i18n mapping.
        let result = build_export_result(1, BestEffortOutcome::SkippedUnsupported);
        assert_eq!(result.warnings[0], "acl_not_applied");
    }

    // ── P4.4 command surface smoke tests (library-layer only — no Tauri ceremony) ──
    //
    // We cannot call async Tauri commands directly in unit tests (require AppHandle).
    // Instead we verify the TYPE CONTRACT: that all result types serialize correctly
    // and that the pure method layer (ProfilesEnvelope) works when called in the
    // same pattern the commands use (build envelope → mutate → check).

    // P4.4a — create_folder command pattern: build envelope → mutate → verify result
    #[test]
    fn command_pattern_create_folder_produces_serializable_folder() {
        use crate::profile::{make_system_folder_for_test, ProfilesEnvelope};
        let sys = make_system_folder_for_test();
        let sys_id = sys.id;
        let mut env = ProfilesEnvelope {
            folders: vec![sys],
            profiles: vec![],
        };
        let snapshot = env.clone();
        let folder = env.create_folder("My Servers".to_string())
            .expect("create_folder must succeed");
        // Verify snapshot is unaffected (rollback contract)
        assert_eq!(snapshot.folders.len(), 1, "snapshot unaffected");
        // Verify result serializes cleanly
        let json = serde_json::to_string(&folder).expect("Folder must be Serialize");
        assert!(json.contains("\"name\":\"My Servers\""), "name must serialize: {json}");
        assert!(json.contains("\"isSystem\":false"), "isSystem must serialize: {json}");
        // Verify envelope state
        assert_eq!(env.folders.len(), 2);
        assert!(env.folders.iter().any(|f| f.id == sys_id));
    }

    // P4.4b — delete_folder command pattern: DeleteFolderResult serializes to camelCase
    #[test]
    fn command_pattern_delete_folder_result_camel_case() {
        let result = DeleteFolderResult { moved_profile_count: 5 };
        let json = serde_json::to_string(&result).expect("must serialize");
        assert!(
            json.contains("\"movedProfileCount\":5"),
            "must be camelCase: {json}"
        );
    }

    // P4.4c — ProfilesEnvelope serializes for load_profiles_with_folders return type
    #[test]
    fn command_pattern_profiles_envelope_serializes() {
        use crate::profile::{make_system_folder_for_test, ProfilesEnvelope};
        let env = ProfilesEnvelope {
            folders: vec![make_system_folder_for_test()],
            profiles: vec![],
        };
        let json = serde_json::to_string(&env).expect("ProfilesEnvelope must be Serialize");
        assert!(json.contains("\"folders\""), "must have 'folders' key: {json}");
        assert!(json.contains("\"profiles\""), "must have 'profiles' key: {json}");
    }
}
