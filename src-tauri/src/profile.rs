// profile.rs — Connection profile types and JSON persistence
//
// Profiles are stored as a JSON object (v2 envelope) in {app_data_dir}/profiles.json.
// Legacy format: plain JSON array (v1). Dual-format detection on load; auto-migration
// writes a backup before upgrading.
// Passwords/passphrases are NEVER stored in the JSON file — only in the encrypted vault.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, ProfileError};
use crate::state::TunnelConfig;

// ─── Folder ─────────────────────────────────────────────

/// System folder name (raw, never shown to user — rendered via i18n key).
pub const SYSTEM_FOLDER_NAME: &str = "__system__";

/// A named group that organises connection profiles in the sidebar.
///
/// `is_system: true` marks the built-in "Sin agrupar" folder —
/// it cannot be renamed or deleted.
/// `is_expanded` persists the sidebar expand/collapse state across restarts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: Uuid,
    pub name: String,
    pub display_order: i32,
    /// Whether this is the built-in system folder (cannot be renamed/deleted).
    #[serde(default)]
    pub is_system: bool,
    /// Sidebar expand/collapse state — persisted to disk.
    #[serde(default = "default_is_expanded")]
    pub is_expanded: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_is_expanded() -> bool {
    true
}

// ─── Profiles Envelope (v2 format) ──────────────────────

/// On-disk format v2 — JSON object with `folders` and `profiles` arrays.
/// Dual-format detection: array root = v1 legacy, object root = v2 envelope.
/// There is NO explicit `schema_version` field — root shape encodes version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilesEnvelope {
    pub folders: Vec<Folder>,
    pub profiles: Vec<ConnectionProfile>,
}

// ─── Folder CRUD result types ───────────────────────────

/// Result returned by `ProfilesEnvelope::delete_folder` and the
/// `delete_folder` Tauri command.
///
/// Carries the number of profiles relocated to the system folder so the
/// frontend can show a confirmation summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFolderResult {
    /// Number of profiles that were moved to the system folder.
    pub moved_profile_count: usize,
}

// ─── Folder CRUD — impl ProfilesEnvelope ────────────────
//
// Pure mutation methods on `ProfilesEnvelope`.
// Callers (Phase 4 Tauri commands) are responsible for:
//   1. Acquiring the AppState lock.
//   2. Optionally cloning before mutation for rollback on persist failure.
//   3. Calling `save_profiles_envelope` after a successful mutation.
//
// Rollback contract: these methods mutate `&mut self`. If the caller needs
// rollback-on-persist-failure, it should `let snapshot = envelope.clone()`
// before calling, then reassign from snapshot on error. See P3.28 test.

impl ProfilesEnvelope {
    // ── Private helpers ──────────────────────────────────

    /// Find a mutable reference to a folder by UUID.
    /// Returns `ProfileError::FolderNotFound` if not present.
    fn find_folder_mut(&mut self, id: Uuid) -> Result<&mut Folder, ProfileError> {
        self.folders.iter_mut().find(|f| f.id == id).ok_or(ProfileError::FolderNotFound)
    }

    /// Validate a proposed folder name:
    ///   - Trim whitespace.
    ///   - Must not be empty after trimming.
    ///   - Must be ≤ 64 characters after trimming.
    ///
    /// Returns the trimmed name on success.
    fn validate_folder_name(name: &str) -> Result<String, ProfileError> {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() || trimmed.len() > 64 {
            return Err(ProfileError::InvalidName);
        }
        Ok(trimmed)
    }

    /// Check for a case-insensitive name duplicate, ignoring `exclude_id`.
    /// Used for both create (exclude_id = nil) and rename (exclude_id = own id).
    fn name_conflicts(
        &self,
        name: &str,
        exclude_id: Option<Uuid>,
    ) -> bool {
        let lower = name.to_lowercase();
        self.folders.iter().any(|f| {
            // Skip the folder being renamed (if any)
            if let Some(eid) = exclude_id {
                if f.id == eid {
                    return false;
                }
            }
            f.name.to_lowercase() == lower
        })
    }

    /// Return the system folder id. The system folder is always present after
    /// migration / load; this helper is infallible within Phase 3.
    fn system_folder_id(&self) -> Uuid {
        self.folders
            .iter()
            .find(|f| f.is_system)
            .map(|f| f.id)
            .expect("invariant: system folder is always present in ProfilesEnvelope")
    }

    // ── Public API ───────────────────────────────────────

    /// Create a new user folder with the given name.
    ///
    /// Validation: name is trimmed; must be 1–64 chars; must not duplicate an
    /// existing folder name (case-insensitive).
    /// `display_order` is `max(existing display_orders) + 1`.
    /// `is_expanded` defaults to `true`.
    ///
    /// Returns a clone of the newly created folder.
    pub fn create_folder(&mut self, name: String) -> Result<Folder, ProfileError> {
        let trimmed = Self::validate_folder_name(&name)?;

        if self.name_conflicts(&trimmed, None) {
            return Err(ProfileError::DuplicateName);
        }

        let max_order = self.folders.iter().map(|f| f.display_order).max().unwrap_or(-1);
        let now = Utc::now();
        let folder = Folder {
            id: Uuid::new_v4(),
            name: trimmed,
            display_order: max_order + 1,
            is_system: false,
            is_expanded: true,
            created_at: now,
            updated_at: now,
        };
        self.folders.push(folder.clone());
        Ok(folder)
    }

    /// Rename an existing user folder.
    ///
    /// Rejects: system folders, invalid names, case-insensitive duplicates
    /// (renaming to the folder's own name or a case variant of its own name
    /// is explicitly allowed).
    ///
    /// Returns a clone of the renamed folder.
    pub fn rename_folder(&mut self, folder_id: Uuid, new_name: String) -> Result<Folder, ProfileError> {
        // Validate name first (before we borrow self mutably)
        let trimmed = Self::validate_folder_name(&new_name)?;

        // Check system protection
        {
            let folder = self.folders.iter().find(|f| f.id == folder_id)
                .ok_or(ProfileError::FolderNotFound)?;
            if folder.is_system {
                return Err(ProfileError::SystemFolderProtected);
            }
        }

        // Check duplicate — exclude own id so renaming to own name (case change) is allowed
        if self.name_conflicts(&trimmed, Some(folder_id)) {
            return Err(ProfileError::DuplicateName);
        }

        let folder = self.find_folder_mut(folder_id)?;
        folder.name = trimmed;
        folder.updated_at = Utc::now();

        Ok(folder.clone())
    }

    /// Delete a user folder.
    ///
    /// If the folder contains profiles, they are moved to the system folder
    /// with their relative order preserved (appended to end of system folder's profiles).
    /// Returns `DeleteFolderResult` with the number of profiles moved.
    /// Rejects: system folders, non-existent UUIDs.
    pub fn delete_folder(&mut self, folder_id: Uuid) -> Result<DeleteFolderResult, ProfileError> {
        // Find folder and check protection
        let folder_idx = self.folders.iter().position(|f| f.id == folder_id)
            .ok_or(ProfileError::FolderNotFound)?;

        if self.folders[folder_idx].is_system {
            return Err(ProfileError::SystemFolderProtected);
        }

        let sys_id = self.system_folder_id();

        // Find profiles in this folder, sorted by their current display_order
        let mut profile_ids_ordered: Vec<(Uuid, i32)> = self.profiles.iter()
            .filter(|p| p.folder_id == Some(folder_id))
            .map(|p| (p.id, p.display_order))
            .collect();
        profile_ids_ordered.sort_by_key(|(_, order)| *order);

        let moved_count = profile_ids_ordered.len();

        if moved_count > 0 {
            // Find the max display_order in the system folder (for append)
            let sys_max_order = self.profiles.iter()
                .filter(|p| p.folder_id == Some(sys_id))
                .map(|p| p.display_order)
                .max()
                .unwrap_or(-1);

            // Move profiles to system folder, preserving relative order
            for (new_offset, (pid, _)) in profile_ids_ordered.iter().enumerate() {
                if let Some(p) = self.profiles.iter_mut().find(|p| p.id == *pid) {
                    p.folder_id = Some(sys_id);
                    p.display_order = sys_max_order + 1 + new_offset as i32;
                }
            }
        }

        // Remove the folder
        self.folders.remove(folder_idx);

        Ok(DeleteFolderResult { moved_profile_count: moved_count })
    }

    /// Reorder all folders.
    ///
    /// `ordered_ids` must contain EXACTLY the same UUIDs as `self.folders`
    /// (no more, no less). Each folder's `display_order` is set to its index
    /// in `ordered_ids`.
    ///
    /// Errors:
    /// - `FolderNotFound` if any UUID in the input is not in `self.folders`.
    /// - `IncompleteReorder` if the input is missing one or more existing UUIDs.
    pub fn reorder_folders(&mut self, ordered_ids: Vec<Uuid>) -> Result<(), ProfileError> {
        // Check for unknown IDs
        for id in &ordered_ids {
            if !self.folders.iter().any(|f| f.id == *id) {
                return Err(ProfileError::FolderNotFound);
            }
        }
        // Check for missing IDs
        if ordered_ids.len() != self.folders.len() {
            return Err(ProfileError::IncompleteReorder);
        }
        // Apply new display_orders
        for (idx, id) in ordered_ids.iter().enumerate() {
            let folder = self.find_folder_mut(*id)?;
            folder.display_order = idx as i32;
        }
        Ok(())
    }

    /// Move a profile to a different folder (or reorder within the same folder).
    ///
    /// If `target_folder_id != current folder`, the profile's `folder_id` is
    /// updated and siblings in the target folder with `display_order >= new_order`
    /// are shifted by +1 to make room.
    ///
    /// If `target_folder_id == current folder`, behaves like an in-folder reorder
    /// to `new_order`.
    pub fn move_profile_to_folder(
        &mut self,
        profile_id: Uuid,
        target_folder_id: Uuid,
        new_order: i32,
    ) -> Result<(), ProfileError> {
        // Validate target folder exists
        if !self.folders.iter().any(|f| f.id == target_folder_id) {
            return Err(ProfileError::FolderNotFound);
        }
        // Validate profile exists
        if !self.profiles.iter().any(|p| p.id == profile_id) {
            return Err(ProfileError::ProfileNotFound);
        }

        let current_folder_id = self.profiles.iter()
            .find(|p| p.id == profile_id)
            .and_then(|p| p.folder_id)
            .unwrap_or(target_folder_id);

        if current_folder_id == target_folder_id {
            // Same folder: simple reorder — shift profiles to accommodate new_order
            // Remove profile from current position, insert at new_order.
            // Collect all (id, order) pairs for the folder, sorted by current order.
            let mut folder_profiles: Vec<(Uuid, i32)> = self.profiles.iter()
                .filter(|p| p.folder_id == Some(target_folder_id))
                .map(|p| (p.id, p.display_order))
                .collect();
            folder_profiles.sort_by_key(|(_, o)| *o);

            let current_pos = folder_profiles.iter().position(|(id, _)| *id == profile_id).unwrap();
            folder_profiles.remove(current_pos);

            let insert_pos = (new_order as usize).min(folder_profiles.len());
            folder_profiles.insert(insert_pos, (profile_id, new_order));

            // Reassign sequential display_orders
            for (idx, (pid, _)) in folder_profiles.iter().enumerate() {
                if let Some(p) = self.profiles.iter_mut().find(|p| p.id == *pid) {
                    p.display_order = idx as i32;
                }
            }
        } else {
            // Cross-folder: shift siblings in target folder at >= new_order
            for p in self.profiles.iter_mut() {
                if p.folder_id == Some(target_folder_id) && p.display_order >= new_order {
                    p.display_order += 1;
                }
            }
            // Update the profile
            if let Some(p) = self.profiles.iter_mut().find(|p| p.id == profile_id) {
                p.folder_id = Some(target_folder_id);
                p.display_order = new_order;
            }
        }

        Ok(())
    }

    /// Reorder all profiles within a specific folder.
    ///
    /// `ordered_profile_ids` must contain EXACTLY the same profile UUIDs that
    /// currently belong to `folder_id` (no more, no less, no cross-folder).
    ///
    /// Errors:
    /// - `FolderNotFound` if `folder_id` doesn't exist.
    /// - `ProfileNotFound` if any UUID in input doesn't exist in `self.profiles`.
    /// - `ProfileFolderMismatch` if a profile exists but belongs to a different folder.
    /// - `IncompleteReorder` if the input is a proper subset of the folder's profiles.
    pub fn reorder_profiles_in_folder(
        &mut self,
        folder_id: Uuid,
        ordered_profile_ids: Vec<Uuid>,
    ) -> Result<(), ProfileError> {
        // Validate folder exists
        if !self.folders.iter().any(|f| f.id == folder_id) {
            return Err(ProfileError::FolderNotFound);
        }

        // Validate each id: exists + belongs to this folder
        for pid in &ordered_profile_ids {
            match self.profiles.iter().find(|p| p.id == *pid) {
                None => return Err(ProfileError::ProfileNotFound),
                Some(p) => {
                    if p.folder_id != Some(folder_id) {
                        return Err(ProfileError::ProfileFolderMismatch);
                    }
                }
            }
        }

        // Validate completeness: no missing profiles in this folder
        let folder_profile_count = self.profiles.iter()
            .filter(|p| p.folder_id == Some(folder_id))
            .count();
        if ordered_profile_ids.len() != folder_profile_count {
            return Err(ProfileError::IncompleteReorder);
        }

        // Apply new display_orders
        for (idx, pid) in ordered_profile_ids.iter().enumerate() {
            if let Some(p) = self.profiles.iter_mut().find(|p| p.id == *pid) {
                p.display_order = idx as i32;
            }
        }

        Ok(())
    }

    /// Set the `is_expanded` state of a folder (persisted to disk by caller).
    /// Idempotent: calling with the same value is a no-op but does not error.
    pub fn set_folder_expanded(&mut self, folder_id: Uuid, expanded: bool) -> Result<(), ProfileError> {
        let folder = self.find_folder_mut(folder_id)?;
        folder.is_expanded = expanded;
        Ok(())
    }
}

// ─── User Credential ────────────────────────────────────

/// A single user identity + auth config within a connection profile.
/// Each profile has one or more users that can connect to the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserCredential {
    pub id: Uuid,
    pub username: String,
    pub auth_method: AuthMethodConfig,
    #[serde(default)]
    pub is_default: bool,
}

// ─── Connection Profile ─────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub port: u16,
    /// Legacy field — only used for backward-compatible deserialization of old profiles.
    /// New profiles store credentials in the `users` array.
    #[serde(default, skip_serializing)]
    pub username: Option<String>,
    /// Legacy field — only used for backward-compatible deserialization of old profiles.
    #[serde(default, skip_serializing)]
    pub auth_method: Option<AuthMethodConfig>,
    /// Users array — the canonical source of user credentials for this profile.
    /// On deserialization of old profiles (with top-level username/auth_method),
    /// post-processing migrates them into this array.
    #[serde(default)]
    pub users: Vec<UserCredential>,
    pub startup_directory: Option<String>,
    pub tunnels: Vec<TunnelConfig>,
    #[serde(default)]
    pub display_order: i32,
    /// Folder assignment. Post-load invariant: always Some after migrate/load.
    /// `#[serde(default)]` ensures old profiles without this field parse as None.
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─── Auth Method Config (persisted) ─────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AuthMethodConfig {
    /// Password fetched from keychain at connect time
    Password,
    /// Public key auth — passphrase may be in keychain
    PublicKey {
        private_key_path: String,
        passphrase_in_keychain: bool,
    },
    /// Keyboard-interactive auth
    KeyboardInteractive,
}

impl Default for ConnectionProfile {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            host: String::new(),
            port: 22,
            username: None,
            auth_method: None,
            users: Vec::new(),
            startup_directory: None,
            tunnels: Vec::new(),
            display_order: 0,
            folder_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }
}

impl ConnectionProfile {
    /// Migrate legacy profiles that have top-level username/auth_method
    /// but no users array. Creates a single UserCredential from the old fields.
    /// This is idempotent — if users is already populated, does nothing.
    pub fn migrate_legacy_fields(&mut self) {
        if !self.users.is_empty() {
            return;
        }
        if let Some(ref username) = self.username {
            if !username.is_empty() {
                let auth = self
                    .auth_method
                    .clone()
                    .unwrap_or(AuthMethodConfig::Password);
                self.users.push(UserCredential {
                    id: Uuid::new_v4(),
                    username: username.clone(),
                    auth_method: auth,
                    is_default: true,
                });
            }
        }
        // Clear legacy fields after migration
        self.username = None;
        self.auth_method = None;
    }
}

// ─── Validation ─────────────────────────────────────────

impl ConnectionProfile {
    /// Validate required fields before persisting
    pub fn validate(&self) -> Result<(), AppError> {
        if self.name.trim().is_empty() {
            return Err(AppError::ProfileError("name is required".to_string()));
        }
        if self.host.trim().is_empty() {
            return Err(AppError::ProfileError("hostname is required".to_string()));
        }
        if self.port == 0 {
            return Err(AppError::ProfileError("port must be > 0".to_string()));
        }
        if self.users.is_empty() {
            return Err(AppError::ProfileError(
                "profile must have at least one user".to_string(),
            ));
        }
        // Validate each user has a username
        for user in &self.users {
            if user.username.trim().is_empty() {
                return Err(AppError::ProfileError(
                    "each user must have a username".to_string(),
                ));
            }
        }
        // Reject multiple defaults
        let default_count = self.users.iter().filter(|u| u.is_default).count();
        if default_count > 1 {
            return Err(AppError::ProfileError(
                "only one default user allowed".to_string(),
            ));
        }
        Ok(())
    }
}

// ─── Format Detection (Phase 2) ─────────────────────────

/// Discriminates between the two on-disk formats for profiles.json.
/// - `LegacyArray`: root is a JSON array (v1 format)
/// - `Envelope`: root is a JSON object with `folders` and `profiles` (v2 format)
#[derive(Debug, Clone, PartialEq)]
pub enum ProfilesFormat {
    /// Legacy flat-array format: `[{...}, {...}]`
    LegacyArray,
    /// Modern envelope format: `{"folders": [...], "profiles": [...]}`
    Envelope,
}

/// Detect the on-disk format of profiles.json from raw bytes.
///
/// Uses `serde_json::Value` peek — does NOT deserialize into typed structs.
/// Returns `AppError::ProfileError` if the bytes are not valid JSON or are
/// not one of the two known root shapes.
pub fn detect_profiles_format(bytes: &[u8]) -> Result<ProfilesFormat, AppError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| AppError::ProfileError(format!("Invalid JSON in profiles file: {e}")))?;

    match value {
        serde_json::Value::Array(_) => Ok(ProfilesFormat::LegacyArray),
        serde_json::Value::Object(_) => Ok(ProfilesFormat::Envelope),
        other => Err(AppError::ProfileError(format!(
            "Unrecognised profiles.json root shape: {}",
            other
        ))),
    }
}

// ─── Migration Helper (Phase 2) ──────────────────────────

/// Build the default system folder (the "Sin agrupar" / unclassified bucket).
/// Always `is_system: true`, `is_expanded: true`, `display_order: 0`.
fn make_system_folder() -> Folder {
    let now = Utc::now();
    Folder {
        id: Uuid::new_v4(),
        name: SYSTEM_FOLDER_NAME.to_string(),
        display_order: 0,
        is_system: true,
        is_expanded: true,
        created_at: now,
        updated_at: now,
    }
}

/// Pure migration function: converts a legacy `Vec<ConnectionProfile>` to a
/// v2 `ProfilesEnvelope`.
///
/// Creates exactly one folder — the system folder — and assigns all profiles
/// to it. Sequential `display_order` values starting at 0 are assigned.
pub fn migrate_legacy_to_envelope(mut profiles: Vec<ConnectionProfile>) -> ProfilesEnvelope {
    let system_folder = make_system_folder();
    let sys_id = system_folder.id;

    for (i, p) in profiles.iter_mut().enumerate() {
        p.folder_id = Some(sys_id);
        p.display_order = i as i32;
    }

    ProfilesEnvelope {
        folders: vec![system_folder],
        profiles,
    }
}

// ─── Backup Helpers (Phase 2) ────────────────────────────

/// Compute the backup path for `profiles.json`.
///
/// - If `profiles.backup.json` does NOT exist → returns that path.
/// - If it DOES exist → returns `profiles.backup.{UTC_YYYYMMDD_HHMMSS}.json`
///   using the current UTC timestamp. This ensures we never overwrite an
///   existing backup.
fn backup_path_for(profiles_path: &std::path::Path) -> PathBuf {
    let dir = profiles_path.parent().unwrap_or(std::path::Path::new("."));
    let primary = dir.join("profiles.backup.json");
    if !primary.exists() {
        return primary;
    }
    // Collision — use timestamped variant
    let stamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    dir.join(format!("profiles.backup.{stamp}.json"))
}

/// Write a backup of the original `profiles.json` bytes before migration.
/// Never overwrites an existing backup — uses timestamped names on collision.
/// Best-effort ACL hardening is applied after the write.
fn write_backup(profiles_path: &std::path::Path, original_bytes: &[u8]) -> Result<(), AppError> {
    let backup = backup_path_for(profiles_path);
    std::fs::write(&backup, original_bytes)
        .map_err(|e| AppError::ProfileError(format!("Failed to write profiles backup: {e}")))?;
    let _ = crate::fs_secure::best_effort_harden(&backup);
    Ok(())
}

// ─── Envelope Persistence (Phase 2) ──────────────────────

/// Save a full `ProfilesEnvelope` to disk (atomic write via secure_write).
pub fn save_profiles_envelope(
    envelope: &ProfilesEnvelope,
    app_data_dir: Option<&PathBuf>,
) -> Result<(), AppError> {
    let path = profiles_file_path(app_data_dir);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::ProfileError(format!("Failed to create profiles directory: {e}"))
        })?;
    }

    let json = serde_json::to_string_pretty(envelope)?;
    crate::fs_secure::secure_write(&path, json.as_bytes())
        .map_err(|e| AppError::ProfileError(format!("Failed to write profiles: {e}")))?;

    Ok(())
}

// ─── Profile Storage ────────────────────────────────────

/// Returns the path to the profiles JSON file.
/// Uses `dirs::data_dir()` as a fallback when no Tauri AppHandle is available.
pub fn profiles_file_path(app_data_dir: Option<&PathBuf>) -> PathBuf {
    if let Some(dir) = app_data_dir {
        dir.join("profiles.json")
    } else {
        // Fallback for tests or non-Tauri contexts
        let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
        base.join("nexterm").join("profiles.json")
    }
}

/// Load all profiles from disk, returning a `ProfilesEnvelope`.
///
/// Dual-format detection:
/// 1. File missing / empty → return envelope with system folder + zero profiles.
/// 2. LegacyArray (v1 flat array) → migrate to envelope, write backup first, save.
/// 3. Envelope (v2 object) → deserialize directly; ensure system folder present.
/// 4. Corrupted / unknown shape → return error; do NOT touch the file.
pub fn load_profiles_from_disk(
    app_data_dir: Option<&PathBuf>,
) -> Result<ProfilesEnvelope, AppError> {
    let path = profiles_file_path(app_data_dir);

    // ── 1. File missing or empty ─────────────────────────
    if !path.exists() {
        return Ok(ProfilesEnvelope {
            folders: vec![make_system_folder()],
            profiles: vec![],
        });
    }

    let bytes = std::fs::read(&path)
        .map_err(|e| AppError::ProfileError(format!("Failed to read profiles file: {e}")))?;

    if bytes.is_empty() {
        return Ok(ProfilesEnvelope {
            folders: vec![make_system_folder()],
            profiles: vec![],
        });
    }

    // ── 2/3/4. Detect format ─────────────────────────────
    let fmt = detect_profiles_format(&bytes)?; // On error (corrupted): returns Err, file untouched.

    match fmt {
        // ── Legacy flat array → migrate ──────────────────
        ProfilesFormat::LegacyArray => {
            let mut profiles: Vec<ConnectionProfile> = serde_json::from_slice(&bytes)
                .map_err(|e| AppError::ProfileError(format!("Failed to parse legacy profiles: {e}")))?;

            // Migrate per-profile legacy fields (username/auth_method → users)
            for p in profiles.iter_mut() {
                if p.users.is_empty() && p.username.is_some() {
                    p.migrate_legacy_fields();
                }
            }

            let envelope = migrate_legacy_to_envelope(profiles);

            // Write backup FIRST, then persist envelope
            if let Err(e) = write_backup(&path, &bytes) {
                tracing::warn!("Failed to create profiles backup during migration: {e}");
            }
            if let Err(e) = save_profiles_envelope(&envelope, app_data_dir) {
                tracing::warn!("Failed to persist migrated profiles envelope: {e}");
            }

            Ok(envelope)
        }

        // ── Modern envelope → deserialize + heal ─────────
        ProfilesFormat::Envelope => {
            let mut envelope: ProfilesEnvelope = serde_json::from_slice(&bytes)
                .map_err(|e| AppError::ProfileError(format!("Failed to parse profiles envelope: {e}")))?;

            // Ensure system folder exists (spec R2 auto-heal)
            if !envelope.folders.iter().any(|f| f.is_system) {
                tracing::warn!("profiles.json has no system folder — auto-healing");
                envelope.folders.push(make_system_folder());
                // Persist the healed envelope
                if let Err(e) = save_profiles_envelope(&envelope, app_data_dir) {
                    tracing::warn!("Failed to persist healed envelope: {e}");
                }
            }

            Ok(envelope)
        }
    }
}

/// Save all profiles to disk using the legacy flat-array format.
///
/// # Phase 2 wrapper strategy
///
/// The four callers in `commands/profile.rs` (lines 103, 156, 199, 538)
/// still pass `&[ConnectionProfile]`. Changing their signatures is Phase 4
/// work. This wrapper preserves backward compatibility by constructing a
/// minimal flat-array JSON — NOT an envelope — so those callers continue
/// to write the legacy format until Phase 4 migrates them to
/// `save_profiles_envelope`.
///
/// **This function is intentionally kept as a shim. Phase 4 will remove it.**
pub fn save_profiles_to_disk(
    profiles: &[ConnectionProfile],
    app_data_dir: Option<&PathBuf>,
) -> Result<(), AppError> {
    let path = profiles_file_path(app_data_dir);

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AppError::ProfileError(format!("Failed to create profiles directory: {e}"))
        })?;
    }

    let json = serde_json::to_string_pretty(profiles)?;

    // Atomic write with owner-only permission hardening (cross-platform).
    // On Unix: sets mode 0o600. On Windows: sets owner-only DACL.
    // The .tmp file is hardened BEFORE rename, closing the race window.
    crate::fs_secure::secure_write(&path, json.as_bytes())
        .map_err(|e| AppError::ProfileError(format!("Failed to write profiles: {e}")))?;

    Ok(())
}

// ─── Test helpers (cfg(test) only) ──────────────────────

/// Public test helper: create a fresh system folder for use in test code
/// outside of `profile::tests`. Exposed only when `#[cfg(test)]`.
#[cfg(test)]
pub fn make_system_folder_for_test() -> Folder {
    make_system_folder()
}

// ─── Tests ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a profile with a single user (the common test case).
    fn test_profile(name: &str, host: &str, username: &str) -> ConnectionProfile {
        ConnectionProfile {
            name: name.to_string(),
            host: host.to_string(),
            users: vec![UserCredential {
                id: Uuid::new_v4(),
                username: username.to_string(),
                auth_method: AuthMethodConfig::Password,
                is_default: true,
            }],
            ..ConnectionProfile::default()
        }
    }

    #[test]
    fn profile_serialize_deserialize_roundtrip() {
        let profile = test_profile("Test", "example.com", "user");

        let json = serde_json::to_string(&profile).unwrap();
        let deserialized: ConnectionProfile = serde_json::from_str(&json).unwrap();

        assert_eq!(profile.id, deserialized.id);
        assert_eq!(profile.name, deserialized.name);
        assert_eq!(profile.host, deserialized.host);
        assert_eq!(profile.port, deserialized.port);
        assert_eq!(deserialized.users.len(), 1);
        assert_eq!(deserialized.users[0].username, "user");
    }

    #[test]
    fn publickey_auth_serializes_correctly() {
        let auth = AuthMethodConfig::PublicKey {
            private_key_path: "~/.ssh/id_ed25519".to_string(),
            passphrase_in_keychain: true,
        };

        let json = serde_json::to_string(&auth).unwrap();
        assert!(json.contains("\"type\":\"publicKey\""));
        assert!(json.contains("privateKeyPath"));
    }

    #[test]
    fn validation_rejects_empty_name() {
        let profile = test_profile("", "example.com", "user");
        assert!(profile.validate().is_err());
    }

    #[test]
    fn validation_rejects_empty_host() {
        let profile = test_profile("Test", "", "user");
        assert!(profile.validate().is_err());
    }

    #[test]
    fn validation_rejects_empty_users() {
        let profile = ConnectionProfile {
            name: "Test".to_string(),
            host: "example.com".to_string(),
            users: vec![],
            ..ConnectionProfile::default()
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn validation_rejects_user_with_empty_username() {
        let profile = ConnectionProfile {
            name: "Test".to_string(),
            host: "example.com".to_string(),
            users: vec![UserCredential {
                id: Uuid::new_v4(),
                username: "".to_string(),
                auth_method: AuthMethodConfig::Password,
                is_default: true,
            }],
            ..ConnectionProfile::default()
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn validation_rejects_multiple_defaults() {
        let profile = ConnectionProfile {
            name: "Test".to_string(),
            host: "example.com".to_string(),
            users: vec![
                UserCredential {
                    id: Uuid::new_v4(),
                    username: "root".to_string(),
                    auth_method: AuthMethodConfig::Password,
                    is_default: true,
                },
                UserCredential {
                    id: Uuid::new_v4(),
                    username: "deploy".to_string(),
                    auth_method: AuthMethodConfig::Password,
                    is_default: true,
                },
            ],
            ..ConnectionProfile::default()
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn validation_accepts_valid_profile() {
        let profile = test_profile("Production", "prod.example.com", "deploy");
        assert!(profile.validate().is_ok());
    }

    // ── P5.3 RED — profiles save produces hardened file ────────────────────────
    //
    // After `save_profiles_to_disk`, the resulting profiles.json must:
    //   - Exist at the expected path.
    //   - Contain valid JSON.
    //   - On Windows: have an owner-only DACL (exactly 1 ACE, protected, current user).
    //   - On Unix: have mode 0o600.
    //
    // This test is the RED gate: it will FAIL until P5.4 replaces the old
    // write+rename+#[cfg(unix)] block with `crate::fs_secure::secure_write`.

    #[test]
    fn save_profiles_to_disk_file_exists_with_valid_content() {
        let dir = TempDir::new().expect("TempDir creation");
        let profiles = vec![test_profile("Server A", "a.example.com", "admin")];

        save_profiles_to_disk(&profiles, Some(&dir.path().to_path_buf())).unwrap();

        let profiles_path = dir.path().join("profiles.json");
        assert!(profiles_path.exists(), "profiles.json must exist after save");

        let contents = std::fs::read_to_string(&profiles_path).expect("read profiles.json");
        let parsed: serde_json::Value =
            serde_json::from_str(&contents).expect("profiles.json must be valid JSON");
        assert!(parsed.is_array(), "profiles.json must be a JSON array");
        assert_eq!(
            parsed.as_array().unwrap().len(),
            1,
            "profiles.json must contain 1 profile"
        );
    }

    #[test]
    fn save_profiles_to_disk_no_tmp_file_remains() {
        let dir = TempDir::new().expect("TempDir creation");
        let profiles = vec![test_profile("Server A", "a.example.com", "admin")];

        save_profiles_to_disk(&profiles, Some(&dir.path().to_path_buf())).unwrap();

        let tmp_path = dir.path().join("profiles.json.tmp");
        assert!(
            !tmp_path.exists(),
            "profiles.json.tmp must not remain after successful save"
        );
    }

    /// P5.3 — On Windows, profiles.json must have an owner-only DACL after save.
    #[cfg(windows)]
    #[test]
    fn save_profiles_to_disk_produces_owner_only_dacl() {
        let dir = TempDir::new().expect("TempDir creation");
        let profiles = vec![test_profile("Server A", "a.example.com", "admin")];

        save_profiles_to_disk(&profiles, Some(&dir.path().to_path_buf())).unwrap();

        let profiles_path = dir.path().join("profiles.json");
        let (ace_count, dacl_protected, all_owner) =
            crate::fs_secure::assert_owner_only_acl_for_test(&profiles_path);

        assert_eq!(
            ace_count, 1,
            "profiles.json DACL must have exactly 1 ACE; got {ace_count}"
        );
        assert!(
            dacl_protected,
            "profiles.json DACL must have SE_DACL_PROTECTED set (no inherited ACEs)"
        );
        assert!(
            all_owner,
            "The single ACE must belong to the current user SID"
        );
    }

    /// P5.3 triangulation — On Unix, profiles.json must have mode 0o600 after save.
    #[cfg(unix)]
    #[test]
    fn save_profiles_to_disk_produces_0600_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().expect("TempDir creation");
        let profiles = vec![test_profile("Server A", "a.example.com", "admin")];

        save_profiles_to_disk(&profiles, Some(&dir.path().to_path_buf())).unwrap();

        let profiles_path = dir.path().join("profiles.json");
        let mode = std::fs::metadata(&profiles_path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "profiles.json must have mode 0o600 on Unix");
    }

    // ── P5.5 RED — legacy migration backup is best-effort hardened ─────────────
    //
    // When `load_profiles_from_disk` encounters a legacy format (top-level
    // username/auth_method), it migrates the profiles AND creates a backup file
    // `profiles.backup.json` via `fs::copy`.
    //
    // This test asserts:
    //   1. The backup file exists after migration.
    //   2. On Windows: it has an owner-only DACL.
    //   3. On Unix: it has mode 0o600.
    //
    // The test is RED until P5.6 adds `best_effort_harden` after the `fs::copy`.

    #[test]
    fn legacy_migration_backup_exists_after_migration() {
        let dir = TempDir::new().expect("TempDir creation");
        let dir_path = dir.path().to_path_buf();

        // Write a legacy-format profiles.json (top-level username/auth_method,
        // no `users` array) — this triggers the migration path in load_profiles_from_disk.
        let legacy_json = r#"[{
            "id": "00000000-0000-0000-0000-000000000001",
            "name": "Legacy Server",
            "host": "legacy.example.com",
            "port": 22,
            "username": "root",
            "authMethod": {"type": "password"},
            "tunnels": [],
            "displayOrder": 0,
            "createdAt": "2024-01-01T00:00:00Z",
            "updatedAt": "2024-01-01T00:00:00Z"
        }]"#;
        std::fs::write(dir.path().join("profiles.json"), legacy_json)
            .expect("write legacy profiles.json");

        // Run load — this triggers migration + backup creation.
        let envelope = load_profiles_from_disk(Some(&dir_path))
            .expect("load_profiles_from_disk must succeed");
        assert_eq!(envelope.profiles.len(), 1, "migration should preserve 1 profile");

        // Assert the backup file exists.
        let backup_path = dir.path().join("profiles.backup.json");
        assert!(
            backup_path.exists(),
            "profiles.backup.json must be created during migration"
        );
    }

    /// P5.5 — On Windows, the legacy migration backup must have an owner-only DACL.
    ///
    /// RED gate: fails until P5.6 adds `best_effort_harden` after `fs::copy`.
    #[cfg(windows)]
    #[test]
    fn legacy_migration_backup_is_best_effort_hardened() {
        let dir = TempDir::new().expect("TempDir creation");
        let dir_path = dir.path().to_path_buf();

        let legacy_json = r#"[{
            "id": "00000000-0000-0000-0000-000000000002",
            "name": "Legacy Server 2",
            "host": "legacy2.example.com",
            "port": 22,
            "username": "admin",
            "authMethod": {"type": "password"},
            "tunnels": [],
            "displayOrder": 0,
            "createdAt": "2024-01-01T00:00:00Z",
            "updatedAt": "2024-01-01T00:00:00Z"
        }]"#;
        std::fs::write(dir.path().join("profiles.json"), legacy_json)
            .expect("write legacy profiles.json");

        load_profiles_from_disk(Some(&dir_path)).expect("load must succeed");

        let backup_path = dir.path().join("profiles.backup.json");
        assert!(backup_path.exists(), "backup must exist");

        let (ace_count, dacl_protected, all_owner) =
            crate::fs_secure::assert_owner_only_acl_for_test(&backup_path);

        assert_eq!(
            ace_count, 1,
            "profiles.backup.json DACL must have exactly 1 ACE; got {ace_count}"
        );
        assert!(
            dacl_protected,
            "profiles.backup.json must have SE_DACL_PROTECTED set"
        );
        assert!(
            all_owner,
            "The single ACE must belong to the current user SID"
        );
    }

    #[test]
    fn disk_persistence_roundtrip() {
        // NOTE (Phase 2): save_profiles_to_disk writes a flat array (legacy shim).
        // load_profiles_from_disk now returns a ProfilesEnvelope. The round-trip
        // goes through the LegacyArray migration path.
        let dir = std::env::temp_dir().join(format!("profile_test_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let profiles = vec![
            test_profile("Server A", "a.example.com", "admin"),
            ConnectionProfile {
                name: "Server B".to_string(),
                host: "b.example.com".to_string(),
                users: vec![UserCredential {
                    id: Uuid::new_v4(),
                    username: "deploy".to_string(),
                    auth_method: AuthMethodConfig::PublicKey {
                        private_key_path: "~/.ssh/id_ed25519".to_string(),
                        passphrase_in_keychain: false,
                    },
                    is_default: true,
                }],
                ..ConnectionProfile::default()
            },
        ];

        save_profiles_to_disk(&profiles, Some(&dir)).unwrap();
        let envelope = load_profiles_from_disk(Some(&dir)).unwrap();

        assert_eq!(envelope.profiles.len(), 2);
        assert_eq!(envelope.profiles[0].name, "Server A");
        assert_eq!(envelope.profiles[1].name, "Server B");

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_from_nonexistent_returns_empty() {
        // Phase 2: returns envelope with system folder + 0 profiles (not an error).
        let dir = PathBuf::from("/tmp/nonexistent_profile_dir_12345_phase2");
        let envelope = load_profiles_from_disk(Some(&dir)).unwrap();
        // System folder is always present; profiles must be empty.
        assert!(envelope.profiles.is_empty());
        assert_eq!(envelope.folders.len(), 1);
        assert!(envelope.folders[0].is_system);
    }

    // ── P1.3 [RED] → P1.4 [GREEN] — ProfilesEnvelope round-trip ───────────────
    #[test]
    fn envelope_serialize_roundtrip() {
        let folder1 = Folder {
            id: Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap(),
            name: "Proxmox".to_string(),
            display_order: 0,
            is_system: false,
            is_expanded: true,
            created_at: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };
        let folder2 = Folder {
            id: Uuid::parse_str("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb").unwrap(),
            name: "__system__".to_string(),
            display_order: 1,
            is_system: true,
            is_expanded: false,
            created_at: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };
        let profiles = vec![
            test_profile("server-a", "a.example.com", "admin"),
            test_profile("server-b", "b.example.com", "deploy"),
            test_profile("server-c", "c.example.com", "root"),
        ];

        let envelope = ProfilesEnvelope {
            folders: vec![folder1, folder2],
            profiles,
        };

        let json = serde_json::to_string(&envelope).unwrap();
        let deserialized: ProfilesEnvelope = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.folders.len(), 2);
        assert_eq!(deserialized.profiles.len(), 3);
        assert_eq!(envelope, deserialized);
    }

    // ── P1.8 [DESIGN] — SYSTEM_FOLDER_NAME constant stability ─────────────────
    #[test]
    fn system_folder_name_constant_is_stable() {
        // The constant must NEVER change — changing it would break existing envelope files.
        assert_eq!(SYSTEM_FOLDER_NAME, "__system__");

        // A Folder with is_system: true and the system name is the system folder.
        let sys_folder = Folder {
            id: Uuid::new_v4(),
            name: SYSTEM_FOLDER_NAME.to_string(),
            display_order: 0,
            is_system: true,
            is_expanded: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(sys_folder.is_system);
        assert_eq!(sys_folder.name, "__system__");

        // A normal folder must have is_system: false by default (serde default)
        let json = r#"{"id":"00000000-0000-0000-0000-000000000001","name":"Lab","displayOrder":0,"isExpanded":true,"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"}"#;
        let folder: Folder = serde_json::from_str(json).unwrap();
        assert!(!folder.is_system, "is_system must default to false");
    }

    // ── P1.5 [RED] → P1.6 [GREEN] — ConnectionProfile folder_id round-trip ────
    #[test]
    fn connection_profile_has_folder_id() {
        let folder_uuid = Uuid::parse_str("cccccccc-cccc-cccc-cccc-cccccccccccc").unwrap();
        // Profile with explicit folder_id = Some(uuid)
        let mut profile = test_profile("server-x", "x.example.com", "user");
        profile.folder_id = Some(folder_uuid);

        let json = serde_json::to_string(&profile).unwrap();
        // Verify the JSON key is present and not null
        assert!(json.contains("\"folderId\":"), "JSON must contain folderId key");

        let deserialized: ConnectionProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.folder_id, Some(folder_uuid));

        // Profile with folder_id = None serializes to null
        let mut profile_none = test_profile("server-y", "y.example.com", "user");
        profile_none.folder_id = None;
        let json_none = serde_json::to_string(&profile_none).unwrap();
        assert!(
            json_none.contains("\"folderId\":null"),
            "JSON with None folder_id must serialize to null"
        );
        let deserialized_none: ConnectionProfile = serde_json::from_str(&json_none).unwrap();
        assert_eq!(deserialized_none.folder_id, None);

        // Old JSON without folderId deserializes to None (backward compat)
        let old_json = r#"{"id":"00000000-0000-0000-0000-000000000099","name":"old","host":"h","port":22,"users":[],"tunnels":[],"displayOrder":0,"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"}"#;
        let old_profile: ConnectionProfile = serde_json::from_str(old_json).unwrap();
        assert_eq!(
            old_profile.folder_id, None,
            "Old profiles without folderId must deserialize to None"
        );
    }

    // ── P1.1 [RED] → P1.2 [GREEN] — Folder serialize/deserialize round-trip ───
    #[test]
    fn folder_serialize_roundtrip() {
        let folder = Folder {
            id: Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
            name: "Proxmox".to_string(),
            display_order: 1,
            is_system: false,
            is_expanded: true,
            created_at: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };

        let json = serde_json::to_string(&folder).unwrap();
        let deserialized: Folder = serde_json::from_str(&json).unwrap();

        assert_eq!(folder, deserialized);
        assert_eq!(deserialized.name, "Proxmox");
        assert!(!deserialized.is_system);
        assert!(deserialized.is_expanded);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // P2 RED TESTS — Phase 2 Migration (written FIRST, before any GREEN code)
    // All tests use `migration_` prefix for easy filtering:
    //   cargo test migration_
    // ═══════════════════════════════════════════════════════════════════════════

    // ── P2.1 [RED] — detect_profiles_format returns LegacyArray for flat array ─
    #[test]
    fn migration_detect_format_legacy_array() {
        let bytes = br#"[{"id":"00000000-0000-0000-0000-000000000001","name":"Server","host":"h","port":22,"tunnels":[],"displayOrder":0,"createdAt":"2024-01-01T00:00:00Z","updatedAt":"2024-01-01T00:00:00Z"}]"#;
        let fmt = detect_profiles_format(bytes).expect("detect_profiles_format must succeed on valid array");
        assert!(
            matches!(fmt, ProfilesFormat::LegacyArray),
            "expected LegacyArray, got {fmt:?}"
        );
    }

    // ── P2.2 [RED] — detect_profiles_format returns Envelope for object with folders+profiles ─
    #[test]
    fn migration_detect_format_envelope() {
        let bytes = br#"{"folders":[],"profiles":[]}"#;
        let fmt = detect_profiles_format(bytes).expect("detect_profiles_format must succeed on valid envelope");
        assert!(
            matches!(fmt, ProfilesFormat::Envelope),
            "expected Envelope, got {fmt:?}"
        );
    }

    // ── P2.3 [RED] — migrate_legacy_to_envelope creates system folder + assigns profiles ─
    #[test]
    fn migration_legacy_to_envelope_produces_system_folder() {
        let profiles = vec![
            test_profile("Server A", "a.example.com", "admin"),
            test_profile("Server B", "b.example.com", "deploy"),
        ];
        let envelope = migrate_legacy_to_envelope(profiles);

        // Exactly one folder
        assert_eq!(envelope.folders.len(), 1, "must have exactly 1 folder");
        let sys = &envelope.folders[0];
        assert_eq!(sys.name, SYSTEM_FOLDER_NAME, "folder name must be SYSTEM_FOLDER_NAME");
        assert!(sys.is_system, "folder must be is_system: true");
        assert!(sys.is_expanded, "folder must be is_expanded: true");
        assert_eq!(sys.display_order, 0, "system folder display_order must be 0");

        // All profiles assigned to system folder, sequential display_order
        assert_eq!(envelope.profiles.len(), 2);
        for (i, p) in envelope.profiles.iter().enumerate() {
            assert_eq!(p.folder_id, Some(sys.id), "profile {i} must have folder_id = system folder id");
            assert_eq!(p.display_order, i as i32, "profile {i} must have display_order = {i}");
        }
    }

    // ── P2.4 [RED] — load envelope format returns it unchanged (no migration, no backup) ─
    #[test]
    fn migration_envelope_format_is_idempotent() {
        let dir = TempDir::new().expect("TempDir");
        let dir_path = dir.path().to_path_buf();

        // Build an envelope and write it
        let sys_folder = Folder {
            id: Uuid::new_v4(),
            name: SYSTEM_FOLDER_NAME.to_string(),
            display_order: 0,
            is_system: true,
            is_expanded: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let envelope = ProfilesEnvelope {
            folders: vec![sys_folder],
            profiles: vec![],
        };
        let json = serde_json::to_string(&envelope).unwrap();
        std::fs::write(dir.path().join("profiles.json"), &json).unwrap();

        // Load — must succeed and NOT write a backup
        let loaded = load_profiles_from_disk(Some(&dir_path)).expect("must succeed");
        assert_eq!(loaded.folders.len(), 1, "must preserve 1 folder");
        assert_eq!(loaded.profiles.len(), 0, "must preserve 0 profiles");

        // No backup written
        let backup = dir.path().join("profiles.backup.json");
        assert!(!backup.exists(), "no backup should be written for already-modern format");
    }

    // ── P2.5 [RED] — missing profiles.json returns envelope with system folder, zero profiles ─
    #[test]
    fn migration_missing_file_returns_system_folder_envelope() {
        let dir = TempDir::new().expect("TempDir");
        let dir_path = dir.path().to_path_buf();
        // NOTE: do NOT write profiles.json — file is intentionally absent

        let envelope = load_profiles_from_disk(Some(&dir_path)).expect("must not error on missing file");
        assert_eq!(envelope.folders.len(), 1, "must return exactly 1 folder (system folder)");
        assert!(envelope.folders[0].is_system, "the folder must be is_system: true");
        assert_eq!(envelope.profiles.len(), 0, "must return 0 profiles");
    }

    // ── P2.6 [RED] — legacy migration writes backup BEFORE saving envelope ─
    #[test]
    fn migration_legacy_writes_backup_before_envelope() {
        let dir = TempDir::new().expect("TempDir");
        let dir_path = dir.path().to_path_buf();

        let legacy_json = br#"[{
            "id": "00000000-0000-0000-0000-000000000010",
            "name": "Legacy",
            "host": "legacy.example.com",
            "port": 22,
            "tunnels": [],
            "displayOrder": 0,
            "createdAt": "2024-01-01T00:00:00Z",
            "updatedAt": "2024-01-01T00:00:00Z"
        }]"#;
        let profiles_path = dir.path().join("profiles.json");
        std::fs::write(&profiles_path, legacy_json).unwrap();

        load_profiles_from_disk(Some(&dir_path)).expect("must succeed on legacy format");

        let backup_path = dir.path().join("profiles.backup.json");
        assert!(backup_path.exists(), "profiles.backup.json must exist after legacy migration");

        let backup_content = std::fs::read(&backup_path).unwrap();
        assert_eq!(backup_content, legacy_json, "backup must contain original legacy bytes");
    }

    // ── P2.7 [RED] — when backup already exists, timestamped variant is used ─
    #[test]
    fn migration_backup_collision_uses_timestamped_variant() {
        let dir = TempDir::new().expect("TempDir");
        let dir_path = dir.path().to_path_buf();

        // Pre-create profiles.backup.json to simulate prior migration
        let prior_backup_content = b"prior backup content";
        let backup_path = dir.path().join("profiles.backup.json");
        std::fs::write(&backup_path, prior_backup_content).unwrap();

        // Write a new legacy profiles.json
        let legacy_json = br#"[{
            "id": "00000000-0000-0000-0000-000000000020",
            "name": "Legacy2",
            "host": "legacy2.example.com",
            "port": 22,
            "tunnels": [],
            "displayOrder": 0,
            "createdAt": "2024-01-01T00:00:00Z",
            "updatedAt": "2024-01-01T00:00:00Z"
        }]"#;
        std::fs::write(dir.path().join("profiles.json"), legacy_json).unwrap();

        load_profiles_from_disk(Some(&dir_path)).expect("must succeed");

        // The original backup must be untouched
        assert_eq!(
            std::fs::read(&backup_path).unwrap(),
            prior_backup_content,
            "original backup.json must be untouched"
        );

        // A NEW timestamped backup must exist in the directory
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with("profiles.backup.") && name.ends_with(".json") && name.len() > "profiles.backup..json".len())
            .collect();
        assert!(!entries.is_empty(), "a timestamped backup file must exist; found none. Dir contents: {:?}",
            std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name()).collect::<Vec<_>>());
    }

    // ── P2.8 [RED] — corrupted JSON returns error, original file NOT modified ─
    #[test]
    fn migration_corrupted_json_returns_error_no_modification() {
        let dir = TempDir::new().expect("TempDir");
        let dir_path = dir.path().to_path_buf();

        let corrupted = b"{not valid json at all";
        let profiles_path = dir.path().join("profiles.json");
        std::fs::write(&profiles_path, corrupted).unwrap();

        let result = load_profiles_from_disk(Some(&dir_path));
        assert!(result.is_err(), "corrupted JSON must return an error");

        // Original file must NOT be modified
        let file_content = std::fs::read(&profiles_path).unwrap();
        assert_eq!(file_content, corrupted, "corrupted file must NOT be modified");

        // No backup must be written
        let backup = dir.path().join("profiles.backup.json");
        assert!(!backup.exists(), "no backup must be written for corrupted JSON");
    }

    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn legacy_profile_migration() {
        // Simulate old-format JSON with top-level username/authMethod
        let legacy_json = r#"[{
            "id": "00000000-0000-0000-0000-000000000001",
            "name": "Legacy Server",
            "host": "legacy.example.com",
            "port": 22,
            "username": "root",
            "authMethod": {"type": "password"},
            "tunnels": [],
            "displayOrder": 0,
            "createdAt": "2024-01-01T00:00:00Z",
            "updatedAt": "2024-01-01T00:00:00Z"
        }]"#;

        let mut profiles: Vec<ConnectionProfile> = serde_json::from_str(legacy_json).unwrap();

        // Before migration: users is empty, username is Some
        assert!(profiles[0].users.is_empty());
        assert_eq!(profiles[0].username, Some("root".to_string()));

        // Run migration
        profiles[0].migrate_legacy_fields();

        // After migration: users has 1 entry, legacy fields cleared
        assert_eq!(profiles[0].users.len(), 1);
        assert_eq!(profiles[0].users[0].username, "root");
        assert!(profiles[0].users[0].is_default);
        assert!(profiles[0].username.is_none());
        assert!(profiles[0].auth_method.is_none());
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // P3 RED TESTS — Phase 3 Folder CRUD
    // All tests use `crud_` prefix for easy filtering:
    //   cargo test crud_
    // ═══════════════════════════════════════════════════════════════════════════

    // ── P3.1–P3.5: create_folder ────────────────────────────────────────────

    // P3.1 — create_folder happy path
    #[test]
    fn crud_create_folder_happy_path() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let folder = env.create_folder("Proxmox".to_string()).expect("create_folder must succeed");
        assert!(!folder.id.is_nil(), "must have a non-nil UUID");
        assert_eq!(folder.name, "Proxmox");
        assert!(!folder.is_system, "new folder must not be system");
        assert!(folder.is_expanded, "new folder must default is_expanded: true");
        assert_eq!(folder.display_order, 1, "display_order must be current_max+1");
        assert_eq!(env.folders.len(), 2, "envelope must contain 2 folders now");
    }

    // P3.2 — create_folder rejects empty name
    #[test]
    fn crud_create_folder_rejects_empty_name() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let result = env.create_folder("".to_string());
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ProfileError::InvalidName));
    }

    // P3.3 — create_folder rejects whitespace-only name (after trim)
    #[test]
    fn crud_create_folder_rejects_whitespace_name() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let result = env.create_folder("    ".to_string());
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ProfileError::InvalidName));
    }

    // P3.4 — create_folder rejects name > 64 chars
    #[test]
    fn crud_create_folder_rejects_name_over_64_chars() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let long_name = "a".repeat(65);
        let result = env.create_folder(long_name);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ProfileError::InvalidName));
    }

    // P3.5 — create_folder rejects case-insensitive duplicate name
    #[test]
    fn crud_create_folder_rejects_duplicate_name_case_insensitive() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        env.create_folder("Proxmox".to_string()).expect("first create must succeed");
        // Same case
        let err1 = env.create_folder("Proxmox".to_string()).unwrap_err();
        assert!(matches!(err1, ProfileError::DuplicateName), "same-case duplicate must fail");
        // Different case
        let err2 = env.create_folder("PROXMOX".to_string()).unwrap_err();
        assert!(matches!(err2, ProfileError::DuplicateName), "different-case duplicate must fail");
    }

    // ── P3.6–P3.10: rename_folder ───────────────────────────────────────────

    // P3.6 — rename_folder happy path
    #[test]
    fn crud_rename_folder_happy_path() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let folder = env.create_folder("OldName".to_string()).expect("create must succeed");
        let folder_id = folder.id;
        let old_order = folder.display_order;
        let renamed = env.rename_folder(folder_id, "NewName".to_string()).expect("rename must succeed");
        assert_eq!(renamed.name, "NewName");
        assert_eq!(renamed.id, folder_id, "UUID must not change");
        assert_eq!(renamed.display_order, old_order, "display_order must not change");
    }

    // P3.7 — rename_folder non-existent UUID
    #[test]
    fn crud_rename_folder_not_found() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let fake_id = Uuid::new_v4();
        let result = env.rename_folder(fake_id, "NewName".to_string());
        assert!(matches!(result.unwrap_err(), ProfileError::FolderNotFound));
    }

    // P3.8 — rename_folder system folder is protected
    #[test]
    fn crud_rename_folder_system_protected() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let sys_id = env.folders[0].id;
        let result = env.rename_folder(sys_id, "NewName".to_string());
        assert!(matches!(result.unwrap_err(), ProfileError::SystemFolderProtected));
    }

    // P3.9 — rename_folder invalid name
    #[test]
    fn crud_rename_folder_invalid_name() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let folder = env.create_folder("ValidName".to_string()).expect("create must succeed");
        let folder_id = folder.id;
        // Empty
        assert!(matches!(env.rename_folder(folder_id, "".to_string()).unwrap_err(), ProfileError::InvalidName));
        // Whitespace
        assert!(matches!(env.rename_folder(folder_id, "   ".to_string()).unwrap_err(), ProfileError::InvalidName));
        // Too long
        assert!(matches!(env.rename_folder(folder_id, "a".repeat(65)).unwrap_err(), ProfileError::InvalidName));
    }

    // P3.10 — rename_folder duplicate name (case-insensitive), but own name is allowed
    #[test]
    fn crud_rename_folder_duplicate_name_and_own_name_allowed() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let fa = env.create_folder("FolderA".to_string()).expect("create A");
        let fb = env.create_folder("FolderB".to_string()).expect("create B");
        let fa_id = fa.id;
        let fb_id = fb.id;
        // Rename FolderB to FolderA (duplicate) must fail
        let err = env.rename_folder(fb_id, "FolderA".to_string()).unwrap_err();
        assert!(matches!(err, ProfileError::DuplicateName));
        // Rename FolderA to "FOLDERA" (case change of own name) must succeed
        let renamed = env.rename_folder(fa_id, "FOLDERA".to_string()).expect("renaming own name with case change must succeed");
        assert_eq!(renamed.name, "FOLDERA");
    }

    // ── P3.11–P3.14: delete_folder ──────────────────────────────────────────

    // P3.11 — delete_folder empty folder
    #[test]
    fn crud_delete_folder_empty() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let folder = env.create_folder("EmptyFolder".to_string()).expect("create must succeed");
        let folder_id = folder.id;
        let result = env.delete_folder(folder_id).expect("delete must succeed");
        assert_eq!(result.moved_profile_count, 0);
        assert_eq!(env.folders.len(), 1, "only system folder remains");
        assert!(!env.folders.iter().any(|f| f.id == folder_id), "folder must be gone");
    }

    // P3.12 — delete_folder with profiles → moves profiles to system folder
    #[test]
    fn crud_delete_folder_with_profiles_moves_to_system() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let sys_id = env.folders[0].id;
        let folder = env.create_folder("Target".to_string()).expect("create folder");
        let folder_id = folder.id;

        // Add 3 profiles to the folder
        for i in 0..3 {
            let mut p = ConnectionProfile::default();
            p.folder_id = Some(folder_id);
            p.display_order = i;
            env.profiles.push(p);
        }

        let result = env.delete_folder(folder_id).expect("delete must succeed");
        assert_eq!(result.moved_profile_count, 3, "3 profiles must be moved");
        assert!(!env.folders.iter().any(|f| f.id == folder_id), "folder must be removed");

        // All moved profiles now have system folder id
        for p in &env.profiles {
            assert_eq!(p.folder_id, Some(sys_id), "all profiles must move to system folder");
        }
        // Relative order must be preserved: they are appended to end of system folder's profiles
        // (system folder had 0 profiles before, so they should have display_orders 0, 1, 2)
        let orders: Vec<i32> = env.profiles.iter().map(|p| p.display_order).collect();
        assert_eq!(orders, vec![0, 1, 2], "relative order must be preserved");
    }

    // P3.13 — delete_folder system folder is protected
    #[test]
    fn crud_delete_folder_system_protected() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let sys_id = env.folders[0].id;
        let result = env.delete_folder(sys_id);
        assert!(matches!(result.unwrap_err(), ProfileError::SystemFolderProtected));
    }

    // P3.14 — delete_folder non-existent UUID
    #[test]
    fn crud_delete_folder_not_found() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let result = env.delete_folder(Uuid::new_v4());
        assert!(matches!(result.unwrap_err(), ProfileError::FolderNotFound));
    }

    // ── P3.15–P3.17: reorder_folders ────────────────────────────────────────

    // P3.15 — reorder_folders happy path
    #[test]
    fn crud_reorder_folders_happy_path() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let f1 = env.create_folder("F1".to_string()).expect("F1");
        let f2 = env.create_folder("F2".to_string()).expect("F2");
        let f3 = env.create_folder("F3".to_string()).expect("F3");
        let sys_id = env.folders[0].id;
        let f1_id = f1.id;
        let f2_id = f2.id;
        let f3_id = f3.id;
        // Reorder: F3, F1, sys, F2
        let new_order = vec![f3_id, f1_id, sys_id, f2_id];
        env.reorder_folders(new_order.clone()).expect("reorder must succeed");
        // Each folder's display_order == its index in the input vec
        for (idx, id) in new_order.iter().enumerate() {
            let folder = env.folders.iter().find(|f| f.id == *id).unwrap();
            assert_eq!(folder.display_order, idx as i32, "folder {id} must have display_order {idx}");
        }
    }

    // P3.16 — reorder_folders with missing UUID
    #[test]
    fn crud_reorder_folders_missing_id() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let f1 = env.create_folder("F1".to_string()).expect("F1");
        let _f2 = env.create_folder("F2".to_string()).expect("F2");
        let sys_id = env.folders[0].id;
        // Omit _f2 from the input
        let result = env.reorder_folders(vec![f1.id, sys_id]);
        assert!(matches!(result.unwrap_err(), ProfileError::IncompleteReorder));
    }

    // P3.17 — reorder_folders with unknown UUID
    #[test]
    fn crud_reorder_folders_unknown_id() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let f1 = env.create_folder("F1".to_string()).expect("F1");
        let sys_id = env.folders[0].id;
        let unknown = Uuid::new_v4();
        let result = env.reorder_folders(vec![f1.id, sys_id, unknown]);
        assert!(matches!(result.unwrap_err(), ProfileError::FolderNotFound));
    }

    // ── P3.18–P3.21: move_profile_to_folder ─────────────────────────────────

    // P3.18 — move_profile_to_folder happy path, siblings shift
    #[test]
    fn crud_move_profile_to_folder_shifts_siblings() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let sys_id = env.folders[0].id;
        let fb = env.create_folder("FolderB".to_string()).expect("FolderB");
        let fb_id = fb.id;

        // Add 1 profile to folder A (system), and 2 profiles already in FolderB
        let mut pa = ConnectionProfile::default();
        pa.folder_id = Some(sys_id);
        pa.display_order = 0;
        let pa_id = pa.id;
        env.profiles.push(pa);

        let mut pb1 = ConnectionProfile::default();
        pb1.folder_id = Some(fb_id);
        pb1.display_order = 0;
        env.profiles.push(pb1);

        let mut pb2 = ConnectionProfile::default();
        pb2.folder_id = Some(fb_id);
        pb2.display_order = 1;
        env.profiles.push(pb2);

        // Move pa into FolderB at position 0
        env.move_profile_to_folder(pa_id, fb_id, 0).expect("move must succeed");

        let moved = env.profiles.iter().find(|p| p.id == pa_id).unwrap();
        assert_eq!(moved.folder_id, Some(fb_id), "profile must be in FolderB");
        assert_eq!(moved.display_order, 0, "moved profile must be at order 0");

        // Siblings in FolderB with old order >= 0 must be shifted by +1
        let sibling_orders: Vec<i32> = env.profiles.iter()
            .filter(|p| p.id != pa_id && p.folder_id == Some(fb_id))
            .map(|p| p.display_order)
            .collect();
        assert!(sibling_orders.contains(&1), "first sibling must shift to 1");
        assert!(sibling_orders.contains(&2), "second sibling must shift to 2");
    }

    // P3.19 — move_profile_to_folder: non-existent target folder
    #[test]
    fn crud_move_profile_to_folder_unknown_folder() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let sys_id = env.folders[0].id;
        let mut p = ConnectionProfile::default();
        p.folder_id = Some(sys_id);
        let p_id = p.id;
        env.profiles.push(p);
        let result = env.move_profile_to_folder(p_id, Uuid::new_v4(), 0);
        assert!(matches!(result.unwrap_err(), ProfileError::FolderNotFound));
        // State unchanged
        let still_there = env.profiles.iter().find(|x| x.id == p_id).unwrap();
        assert_eq!(still_there.folder_id, Some(sys_id));
    }

    // P3.20 — move_profile_to_folder: non-existent profile
    #[test]
    fn crud_move_profile_to_folder_unknown_profile() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let sys_id = env.folders[0].id;
        let result = env.move_profile_to_folder(Uuid::new_v4(), sys_id, 0);
        assert!(matches!(result.unwrap_err(), ProfileError::ProfileNotFound));
    }

    // P3.21 — move_profile_to_folder same folder (reorder within folder)
    #[test]
    fn crud_move_profile_same_folder_reorder() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let sys_id = env.folders[0].id;
        let mut p0 = ConnectionProfile::default();
        p0.folder_id = Some(sys_id);
        p0.display_order = 0;
        let p0_id = p0.id;
        let mut p1 = ConnectionProfile::default();
        p1.folder_id = Some(sys_id);
        p1.display_order = 1;
        let p1_id = p1.id;
        let mut p2 = ConnectionProfile::default();
        p2.folder_id = Some(sys_id);
        p2.display_order = 2;
        let _p2_id = p2.id;
        env.profiles.extend([p0, p1, p2]);

        // Move p0 from position 0 to position 2 within same folder
        env.move_profile_to_folder(p0_id, sys_id, 2).expect("same-folder move must succeed");
        let p0_now = env.profiles.iter().find(|p| p.id == p0_id).unwrap();
        assert_eq!(p0_now.display_order, 2);
        assert_eq!(p0_now.folder_id, Some(sys_id));

        // Moving to its own current position should be a no-op / valid
        let current_order = env.profiles.iter().find(|p| p.id == p1_id).unwrap().display_order;
        env.move_profile_to_folder(p1_id, sys_id, current_order).expect("same-position no-op must succeed");
    }

    // ── P3.22–P3.25: reorder_profiles_in_folder ─────────────────────────────

    // P3.22 — reorder_profiles_in_folder happy path
    #[test]
    fn crud_reorder_profiles_in_folder_happy_path() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let sys_id = env.folders[0].id;
        let mut p0 = ConnectionProfile::default();
        p0.folder_id = Some(sys_id);
        p0.display_order = 0;
        let p0_id = p0.id;
        let mut p1 = ConnectionProfile::default();
        p1.folder_id = Some(sys_id);
        p1.display_order = 1;
        let p1_id = p1.id;
        let mut p2 = ConnectionProfile::default();
        p2.folder_id = Some(sys_id);
        p2.display_order = 2;
        let p2_id = p2.id;
        env.profiles.extend([p0, p1, p2]);

        // Reorder: p2, p0, p1
        env.reorder_profiles_in_folder(sys_id, vec![p2_id, p0_id, p1_id]).expect("reorder must succeed");
        let p2_order = env.profiles.iter().find(|p| p.id == p2_id).unwrap().display_order;
        let p0_order = env.profiles.iter().find(|p| p.id == p0_id).unwrap().display_order;
        let p1_order = env.profiles.iter().find(|p| p.id == p1_id).unwrap().display_order;
        assert_eq!(p2_order, 0);
        assert_eq!(p0_order, 1);
        assert_eq!(p1_order, 2);
    }

    // P3.23 — reorder_profiles_in_folder missing profile id
    #[test]
    fn crud_reorder_profiles_in_folder_missing_id() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let sys_id = env.folders[0].id;
        let mut p0 = ConnectionProfile::default();
        p0.folder_id = Some(sys_id);
        p0.display_order = 0;
        let p0_id = p0.id;
        let mut p1 = ConnectionProfile::default();
        p1.folder_id = Some(sys_id);
        p1.display_order = 1;
        env.profiles.extend([p0, p1]);
        // Omit p1 from the input
        let result = env.reorder_profiles_in_folder(sys_id, vec![p0_id]);
        assert!(matches!(result.unwrap_err(), ProfileError::IncompleteReorder));
    }

    // P3.24 — reorder_profiles_in_folder unknown profile id
    #[test]
    fn crud_reorder_profiles_in_folder_unknown_id() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let sys_id = env.folders[0].id;
        let mut p0 = ConnectionProfile::default();
        p0.folder_id = Some(sys_id);
        p0.display_order = 0;
        let p0_id = p0.id;
        env.profiles.push(p0);
        let result = env.reorder_profiles_in_folder(sys_id, vec![p0_id, Uuid::new_v4()]);
        assert!(matches!(result.unwrap_err(), ProfileError::ProfileNotFound));
    }

    // P3.25 — reorder_profiles_in_folder with profile from a different folder
    #[test]
    fn crud_reorder_profiles_in_folder_cross_folder_profile() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let sys_id = env.folders[0].id;
        let fb = env.create_folder("FB".to_string()).expect("FB");
        let fb_id = fb.id;

        let mut p0 = ConnectionProfile::default();
        p0.folder_id = Some(sys_id);
        p0.display_order = 0;
        let p0_id = p0.id;

        let mut p_other = ConnectionProfile::default();
        p_other.folder_id = Some(fb_id);
        p_other.display_order = 0;
        let p_other_id = p_other.id;

        env.profiles.extend([p0, p_other]);

        // Try to reorder sys_id folder including a profile from fb
        let result = env.reorder_profiles_in_folder(sys_id, vec![p0_id, p_other_id]);
        assert!(matches!(result.unwrap_err(), ProfileError::ProfileFolderMismatch));
    }

    // ── P3.26–P3.27: set_folder_expanded ────────────────────────────────────

    // P3.26 — set_folder_expanded happy path + idempotent
    #[test]
    fn crud_set_folder_expanded_happy_path() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let sys_id = env.folders[0].id;
        // Start expanded, collapse it
        env.set_folder_expanded(sys_id, false).expect("set_folder_expanded must succeed");
        assert!(!env.folders[0].is_expanded, "folder must be collapsed");
        // Idempotent: call again with false
        env.set_folder_expanded(sys_id, false).expect("idempotent set_folder_expanded");
        assert!(!env.folders[0].is_expanded, "folder must still be collapsed");
        // Expand it back
        env.set_folder_expanded(sys_id, true).expect("expand must succeed");
        assert!(env.folders[0].is_expanded, "folder must be expanded again");
    }

    // P3.27 — set_folder_expanded non-existent UUID
    #[test]
    fn crud_set_folder_expanded_not_found() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        let result = env.set_folder_expanded(Uuid::new_v4(), true);
        assert!(matches!(result.unwrap_err(), ProfileError::FolderNotFound));
    }

    // ── P3.28: Atomicity / rollback contract ─────────────────────────────────

    // P3.28 — clone before op: original clone is unaffected by mutations
    #[test]
    fn crud_clone_before_op_proves_no_aliased_state() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        // Take snapshot before mutation
        let snapshot = env.clone();
        // Mutate the live envelope
        env.create_folder("NewFolder".to_string()).expect("create must succeed");
        // Snapshot must be unaffected
        assert_eq!(snapshot.folders.len(), 1, "snapshot must still have 1 folder");
        assert_eq!(env.folders.len(), 2, "live env must have 2 folders");
    }

    // ── P3 Error path invariant: failing ops leave state unchanged ────────────
    // Extra test: verify that a failing create_folder does not mutate the envelope
    #[test]
    fn crud_failed_create_folder_leaves_state_unchanged() {
        let mut env = ProfilesEnvelope { folders: vec![make_system_folder()], profiles: vec![] };
        env.create_folder("FolderA".to_string()).expect("first create");
        let before_len = env.folders.len();
        // Duplicate name → must fail without mutating
        let _ = env.create_folder("FolderA".to_string());
        assert_eq!(env.folders.len(), before_len, "failed create_folder must not mutate state");
    }

    #[test]
    fn already_migrated_profile_untouched() {
        let user_id = Uuid::new_v4();
        let mut profile = ConnectionProfile {
            name: "Modern".to_string(),
            host: "modern.example.com".to_string(),
            users: vec![UserCredential {
                id: user_id,
                username: "deploy".to_string(),
                auth_method: AuthMethodConfig::Password,
                is_default: true,
            }],
            ..ConnectionProfile::default()
        };

        profile.migrate_legacy_fields();

        // Should be untouched — still 1 user with same ID
        assert_eq!(profile.users.len(), 1);
        assert_eq!(profile.users[0].id, user_id);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // P4 RED TESTS — Phase 4 Integration / Persistence
    // All tests use `integration_` prefix for easy filtering:
    //   cargo test integration_
    // ═══════════════════════════════════════════════════════════════════════════

    // P4.7 — Full round-trip integration: migration → create_folder → save →
    //         reload → move_profile_to_folder → save → reload →
    //         delete_folder → save → reload → profile back in system folder.
    //
    // This test validates that Phases 2 + 3 + 4 persistence work end-to-end
    // without going through Tauri (pure library layer test).
    #[test]
    fn integration_full_round_trip_create_move_delete_folder() {
        let dir = TempDir::new().expect("TempDir");
        let dir_path = dir.path().to_path_buf();

        // ── 1. Fresh load on empty dir ──────────────────────────────────────
        let env0 = load_profiles_from_disk(Some(&dir_path)).expect("fresh load must succeed");
        assert_eq!(env0.folders.len(), 1, "fresh load: 1 system folder");
        assert_eq!(env0.profiles.len(), 0, "fresh load: 0 profiles");

        // Seed a profile into the system folder by building an envelope manually
        // and saving it (simulates app seeding a profile via save_profile command).
        let sys_id = env0.folders[0].id;
        let mut seed_profile = ConnectionProfile {
            name: "seed-server".to_string(),
            host: "seed.example.com".to_string(),
            users: vec![UserCredential {
                id: Uuid::new_v4(),
                username: "admin".to_string(),
                auth_method: AuthMethodConfig::Password,
                is_default: true,
            }],
            folder_id: Some(sys_id),
            display_order: 0,
            ..ConnectionProfile::default()
        };
        let seed_id = seed_profile.id;
        seed_profile.created_at = Utc::now();
        seed_profile.updated_at = Utc::now();
        let mut env_with_profile = env0.clone();
        env_with_profile.profiles.push(seed_profile);
        save_profiles_envelope(&env_with_profile, Some(&dir_path)).expect("save seeded profile");

        // ── 2. Reload + create_folder("Test") + save ─────────────────────────
        let mut env1 = load_profiles_from_disk(Some(&dir_path)).expect("reload after seed");
        assert_eq!(env1.profiles.len(), 1, "reload: 1 seeded profile");

        let snapshot1 = env1.clone();
        let test_folder = env1.create_folder("Test".to_string()).expect("create_folder Test");
        let test_folder_id = test_folder.id;
        save_profiles_envelope(&env1, Some(&dir_path)).expect("save after create_folder");

        // Verify snapshot is unaffected (rollback contract)
        assert_eq!(snapshot1.folders.len(), 1, "snapshot must be unaffected");

        // ── 3. Reload — envelope must have 2 folders ─────────────────────────
        let env2 = load_profiles_from_disk(Some(&dir_path)).expect("reload after create_folder");
        assert_eq!(env2.folders.len(), 2, "after create_folder: 2 folders");
        assert!(env2.folders.iter().any(|f| f.name == "Test"), "Test folder must exist");

        // ── 4. Move seeded profile to "Test" folder + save ───────────────────
        let mut env3 = env2.clone();
        env3.move_profile_to_folder(seed_id, test_folder_id, 0)
            .expect("move_profile_to_folder must succeed");
        save_profiles_envelope(&env3, Some(&dir_path)).expect("save after move");

        // ── 5. Reload — profile must be in Test folder ───────────────────────
        let env4 = load_profiles_from_disk(Some(&dir_path)).expect("reload after move");
        let moved = env4.profiles.iter().find(|p| p.id == seed_id).expect("profile must exist");
        assert_eq!(moved.folder_id, Some(test_folder_id), "profile folder_id must be Test");

        // ── 6. Delete "Test" folder + save ───────────────────────────────────
        let mut env5 = env4.clone();
        let del_result = env5.delete_folder(test_folder_id).expect("delete_folder must succeed");
        assert_eq!(del_result.moved_profile_count, 1, "1 profile must be moved on delete");
        save_profiles_envelope(&env5, Some(&dir_path)).expect("save after delete_folder");

        // ── 7. Reload — profile must be back in system folder ────────────────
        let env6 = load_profiles_from_disk(Some(&dir_path)).expect("reload after delete");
        assert_eq!(env6.folders.len(), 1, "only system folder remains");
        let restored = env6.profiles.iter().find(|p| p.id == seed_id).expect("profile must still exist");
        let sys_id_final = env6.folders.iter().find(|f| f.is_system).unwrap().id;
        assert_eq!(
            restored.folder_id,
            Some(sys_id_final),
            "profile must be in system folder after folder deletion"
        );
    }

    // P4.8 — Persistence invariant: every save via save_profiles_envelope
    //         writes a JSON object (envelope format) with `folders` + `profiles` keys.
    //         A legacy flat-array root MUST NOT appear after Phase 4 code paths.
    #[test]
    fn integration_persisted_json_is_envelope_format() {
        let dir = TempDir::new().expect("TempDir");
        let dir_path = dir.path().to_path_buf();

        // Build an envelope with one user folder
        let mut env = ProfilesEnvelope {
            folders: vec![{
                let mut sf = ProfilesEnvelope {
                    folders: vec![],
                    profiles: vec![],
                };
                sf.folders.push({
                    let now = Utc::now();
                    Folder {
                        id: Uuid::new_v4(),
                        name: SYSTEM_FOLDER_NAME.to_string(),
                        display_order: 0,
                        is_system: true,
                        is_expanded: true,
                        created_at: now,
                        updated_at: now,
                    }
                });
                sf.folders.remove(0)
            }],
            profiles: vec![],
        };
        let _ = env.create_folder("InvariantTest".to_string()).expect("create folder");
        save_profiles_envelope(&env, Some(&dir_path)).expect("save envelope");

        // Read raw bytes and confirm root is a JSON object
        let profiles_path = dir.path().join("profiles.json");
        let raw = std::fs::read(&profiles_path).expect("read profiles.json");
        let root: serde_json::Value = serde_json::from_slice(&raw).expect("must be valid JSON");

        assert!(root.is_object(), "root must be a JSON object, not array");
        assert!(
            root.get("folders").is_some(),
            "root must have 'folders' key"
        );
        assert!(
            root.get("profiles").is_some(),
            "root must have 'profiles' key"
        );
        assert!(
            !root.is_array(),
            "root must NOT be a JSON array (legacy format)"
        );
    }

    // P4.8b — Serialize check: DeleteFolderResult must serialize to camelCase JSON.
    #[test]
    fn integration_delete_folder_result_serializes() {
        let result = DeleteFolderResult { moved_profile_count: 3 };
        let json = serde_json::to_string(&result).expect("must serialize");
        assert!(
            json.contains("\"movedProfileCount\""),
            "must use camelCase key: {json}"
        );
    }
}
