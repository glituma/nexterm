// vault.rs — Encrypted credential vault (replaces OS keychain)
//
// All SSH credentials are stored in an AES-256-GCM encrypted file,
// keyed by a master password via Argon2id key derivation.
//
// Vault file format (JSON on disk):
// {
//   "version": 1,
//   "salt": "<base64 32-byte salt>",
//   "credentials": {
//     "<profile_id:type>": "<base64 nonce(12) + ciphertext + tag(16)>"
//   }
// }

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::error::AppError;

/// Vault file name in the app data directory.
const VAULT_FILE: &str = "vault.json";

/// Current vault format version.
const VAULT_VERSION: u32 = 1;

/// AES-256-GCM nonce size in bytes.
const NONCE_SIZE: usize = 12;

/// Salt size in bytes for Argon2id.
const SALT_SIZE: usize = 32;

// ─── On-Disk Format ─────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct VaultFile {
    version: u32,
    salt: String,
    credentials: HashMap<String, String>,
}

// ─── Vault ──────────────────────────────────────────────

pub struct Vault {
    file_path: PathBuf,
    derived_key: Option<[u8; 32]>,
    salt: [u8; SALT_SIZE],
    credentials: HashMap<String, Vec<u8>>,
}

impl Drop for Vault {
    fn drop(&mut self) {
        self.lock();
    }
}

impl Vault {
    /// Check if vault file exists on disk.
    pub fn exists(data_dir: &Path) -> bool {
        data_dir.join(VAULT_FILE).exists()
    }

    /// Create a new vault with a master password.
    ///
    /// Generates a random salt, derives the encryption key via Argon2id,
    /// and writes an empty vault file to disk.
    pub fn create(data_dir: &Path, master_password: &str) -> Result<Self, AppError> {
        let file_path = data_dir.join(VAULT_FILE);

        // Ensure parent directory exists
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::VaultError(format!("Failed to create vault directory: {e}"))
            })?;
        }

        // Generate random salt
        let mut salt = [0u8; SALT_SIZE];
        OsRng.fill_bytes(&mut salt);

        // Derive key
        let derived_key = Self::derive_key(master_password, &salt)?;

        let vault = Vault {
            file_path,
            derived_key: Some(derived_key),
            salt,
            credentials: HashMap::new(),
        };

        vault.save_to_disk()?;

        Ok(vault)
    }

    /// Open an existing vault with the master password.
    ///
    /// Reads the vault file, derives the key, and validates it by attempting
    /// to decrypt one credential (if any exist).
    pub fn unlock(data_dir: &Path, master_password: &str) -> Result<Self, AppError> {
        let file_path = data_dir.join(VAULT_FILE);

        let contents = std::fs::read_to_string(&file_path)
            .map_err(|e| AppError::VaultError(format!("Failed to read vault file: {e}")))?;

        let vault_file: VaultFile = serde_json::from_str(&contents)
            .map_err(|e| AppError::VaultError(format!("Corrupt vault file: {e}")))?;

        if vault_file.version != VAULT_VERSION {
            return Err(AppError::VaultError(format!(
                "Unsupported vault version: {}",
                vault_file.version
            )));
        }

        // Decode salt
        let salt_bytes = BASE64
            .decode(&vault_file.salt)
            .map_err(|e| AppError::VaultError(format!("Invalid salt encoding: {e}")))?;
        if salt_bytes.len() != SALT_SIZE {
            return Err(AppError::VaultError("Invalid salt length".to_string()));
        }
        let mut salt = [0u8; SALT_SIZE];
        salt.copy_from_slice(&salt_bytes);

        // Derive key
        let derived_key = Self::derive_key(master_password, &salt)?;

        // Decode all credentials from base64
        let mut credentials = HashMap::new();
        for (key, b64_val) in &vault_file.credentials {
            let bytes = BASE64.decode(b64_val).map_err(|e| {
                AppError::VaultError(format!("Invalid credential encoding for {key}: {e}"))
            })?;
            credentials.insert(key.clone(), bytes);
        }

        let vault = Vault {
            file_path,
            derived_key: Some(derived_key),
            salt,
            credentials,
        };

        // Validate the password by trying to decrypt the first credential
        if let Some((key, _)) = vault.credentials.iter().next() {
            vault.get(key).map_err(|_| AppError::VaultWrongPassword)?;
        }

        Ok(vault)
    }

    /// Store a credential (encrypt + save to disk).
    pub fn store(&mut self, key: &str, value: &str) -> Result<(), AppError> {
        let encrypted = self.encrypt(value)?;
        self.credentials.insert(key.to_owned(), encrypted);
        self.save_to_disk()
    }

    /// Get a credential (decrypt from memory).
    pub fn get(&self, key: &str) -> Result<Option<String>, AppError> {
        match self.credentials.get(key) {
            Some(encrypted) => {
                let plaintext = self.decrypt(encrypted)?;
                Ok(Some(plaintext))
            }
            None => Ok(None),
        }
    }

    /// Check if a credential exists.
    pub fn has(&self, key: &str) -> bool {
        self.credentials.contains_key(key)
    }

    /// Delete a credential.
    pub fn delete(&mut self, key: &str) -> Result<(), AppError> {
        self.credentials.remove(key);
        self.save_to_disk()
    }

    /// Delete all credentials matching a key prefix (e.g., "profile_id:").
    pub fn delete_by_prefix(&mut self, prefix: &str) -> Result<(), AppError> {
        self.credentials.retain(|k, _| !k.starts_with(prefix));
        self.save_to_disk()
    }

    /// Change the master password — re-derive key and re-encrypt all credentials.
    pub fn change_master_password(&mut self, new_password: &str) -> Result<(), AppError> {
        // Decrypt all credentials with current key
        let mut plaintext_map: HashMap<String, String> = HashMap::new();
        for (key, encrypted) in &self.credentials {
            let plain = self.decrypt(encrypted)?;
            plaintext_map.insert(key.clone(), plain);
        }

        // Generate new salt and derive new key
        let mut new_salt = [0u8; SALT_SIZE];
        OsRng.fill_bytes(&mut new_salt);
        let new_key = Self::derive_key(new_password, &new_salt)?;

        // Zeroize old key
        if let Some(ref mut old_key) = self.derived_key {
            old_key.zeroize();
        }

        self.salt = new_salt;
        self.derived_key = Some(new_key);

        // Re-encrypt all credentials with new key
        self.credentials.clear();
        for (key, plain) in &plaintext_map {
            let encrypted = self.encrypt(plain)?;
            self.credentials.insert(key.clone(), encrypted);
        }

        self.save_to_disk()
    }

    /// Lock the vault — clear derived key from memory.
    pub fn lock(&mut self) {
        if let Some(ref mut key) = self.derived_key {
            key.zeroize();
        }
        self.derived_key = None;
    }

    /// Check if the vault is unlocked (has a derived key in memory).
    pub fn is_unlocked(&self) -> bool {
        self.derived_key.is_some()
    }

    // ─── Private Helpers ────────────────────────────────

    /// Derive a 32-byte key from password + salt using Argon2id.
    fn derive_key(password: &str, salt: &[u8; SALT_SIZE]) -> Result<[u8; 32], AppError> {
        let mut key = [0u8; 32];
        Argon2::default()
            .hash_password_into(password.as_bytes(), salt, &mut key)
            .map_err(|e| AppError::VaultError(format!("Key derivation failed: {e}")))?;
        Ok(key)
    }

    /// Encrypt a plaintext string → nonce(12) + ciphertext + tag(16).
    fn encrypt(&self, plaintext: &str) -> Result<Vec<u8>, AppError> {
        let key = self.derived_key.as_ref().ok_or(AppError::VaultLocked)?;

        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|e| AppError::VaultError(format!("Cipher init failed: {e}")))?;

        // Generate random nonce
        let mut nonce_bytes = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| AppError::VaultError(format!("Encryption failed: {e}")))?;

        // Prepend nonce to ciphertext
        let mut result = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);

        Ok(result)
    }

    /// Decrypt nonce(12) + ciphertext + tag(16) → plaintext string.
    fn decrypt(&self, data: &[u8]) -> Result<String, AppError> {
        let key = self.derived_key.as_ref().ok_or(AppError::VaultLocked)?;

        if data.len() < NONCE_SIZE + 16 {
            return Err(AppError::VaultError("Ciphertext too short".to_string()));
        }

        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|e| AppError::VaultError(format!("Cipher init failed: {e}")))?;

        let nonce = Nonce::from_slice(&data[..NONCE_SIZE]);
        let ciphertext = &data[NONCE_SIZE..];

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| AppError::VaultError("Decryption failed".to_string()))?;

        String::from_utf8(plaintext)
            .map_err(|e| AppError::VaultError(format!("Invalid UTF-8 in credential: {e}")))
    }

    /// Save vault to disk (atomic write via temp file, 0o600 permissions on Unix).
    fn save_to_disk(&self) -> Result<(), AppError> {
        // Encode credentials to base64
        let mut encoded_creds = HashMap::new();
        for (key, bytes) in &self.credentials {
            encoded_creds.insert(key.clone(), BASE64.encode(bytes));
        }

        let vault_file = VaultFile {
            version: VAULT_VERSION,
            salt: BASE64.encode(self.salt),
            credentials: encoded_creds,
        };

        let json = serde_json::to_string_pretty(&vault_file)
            .map_err(|e| AppError::VaultError(format!("Failed to serialize vault: {e}")))?;

        // Atomic write with owner-only permission hardening (cross-platform).
        // On Unix: sets mode 0o600. On Windows: sets owner-only DACL.
        // The .tmp file is hardened BEFORE rename, closing the race window.
        crate::fs_secure::secure_write(&self.file_path, json.as_bytes())
            .map_err(|e| AppError::VaultError(format!("Failed to write vault: {e}")))?;

        Ok(())
    }
}

// ─── Post-unlock migration helper ───────────────────────

/// Re-apply owner-only permission hardening to existing credential files.
///
/// Called from `commands/vault.rs::vault_unlock` after a successful unlock
/// to ensure files written by older app versions (without ACL hardening) get
/// upgraded on first unlock.
///
/// This is idempotent and best-effort: files that don't exist are skipped,
/// and ACL failures are not propagated (the function itself logs them).
pub(crate) fn harden_existing_credential_files(data_dir: &std::path::Path) {
    for filename in ["vault.json", "profiles.json"] {
        let path = data_dir.join(filename);
        if path.exists() {
            let _ = crate::fs_secure::best_effort_harden(&path);
        }
    }
}

// ─── Tests ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a vault in a temp directory.
    ///
    /// Uses a simple master password; the vault is fully functional for tests.
    fn make_vault(dir: &TempDir) -> Vault {
        Vault::create(dir.path(), "test-master-password")
            .expect("Vault::create must succeed in a writable temp dir")
    }

    // ── P5.1 RED — vault save produces hardened file ─────
    //
    // Asserts that after `Vault::create` (which calls `save_to_disk` internally),
    // the vault.json file:
    //   - Exists at the expected path.
    //   - Contains valid JSON (round-trip parseable).
    //   - On Windows: has an owner-only DACL (exactly 1 ACE, protected, current user).
    //   - On Unix: has mode 0o600.

    #[test]
    fn vault_save_to_disk_file_exists_with_valid_content() {
        let dir = TempDir::new().expect("TempDir creation");
        let _vault = make_vault(&dir);

        let vault_path = dir.path().join("vault.json");
        assert!(vault_path.exists(), "vault.json must exist after Vault::create");

        let contents = std::fs::read_to_string(&vault_path).expect("read vault.json");
        // Validate it's parseable JSON with expected fields.
        let parsed: serde_json::Value =
            serde_json::from_str(&contents).expect("vault.json must be valid JSON");
        assert_eq!(
            parsed["version"],
            serde_json::json!(1),
            "vault.json must have version=1"
        );
        assert!(
            parsed["salt"].is_string(),
            "vault.json must have a string salt field"
        );
    }

    #[test]
    fn vault_save_to_disk_no_tmp_file_remains() {
        let dir = TempDir::new().expect("TempDir creation");
        let _vault = make_vault(&dir);

        // The .tmp file must be gone after a successful save.
        let tmp_path = dir.path().join("vault.json.tmp");
        assert!(
            !tmp_path.exists(),
            "vault.json.tmp must not remain after Vault::create"
        );
    }

    // ── P6.1 RED — vault_unlock re-hardens existing files ───────────────────────
    //
    // We extract the re-hardening logic into a testable helper
    // `harden_existing_credential_files(data_dir)` and test it here.
    //
    // The test:
    //   1. Creates vault.json and profiles.json using plain `std::fs::write`
    //      (NOT hardened — they'll have default inherited ACLs).
    //   2. Calls the helper.
    //   3. On Windows: asserts both files now have an owner-only DACL.
    //
    // This test is RED until P6.2 implements `harden_existing_credential_files`.

    /// P6.1 — On Windows, `harden_existing_credential_files` must harden both
    /// vault.json and profiles.json.
    #[cfg(windows)]
    #[test]
    fn harden_existing_credential_files_hardens_vault_and_profiles() {
        let dir = TempDir::new().expect("TempDir creation");

        // Write files with plain fs::write — NOT hardened (many ACEs from inheritance).
        std::fs::write(dir.path().join("vault.json"), b"{}").expect("write vault.json");
        std::fs::write(dir.path().join("profiles.json"), b"[]").expect("write profiles.json");

        // Verify they start unhardened (sanity check that the test is meaningful).
        let (ace_before, _, _) =
            crate::fs_secure::assert_owner_only_acl_for_test(&dir.path().join("vault.json"));
        assert_ne!(ace_before, 1, "vault.json should NOT be hardened before the call");

        // Call the helper.
        harden_existing_credential_files(dir.path());

        // Assert vault.json is now hardened.
        let (ace_count, dacl_protected, all_owner) =
            crate::fs_secure::assert_owner_only_acl_for_test(&dir.path().join("vault.json"));
        assert_eq!(ace_count, 1, "vault.json must have 1 ACE after harden; got {ace_count}");
        assert!(dacl_protected, "vault.json must have SE_DACL_PROTECTED");
        assert!(all_owner, "vault.json ACE must belong to the current user SID");

        // Assert profiles.json is now hardened.
        let (ace_count, dacl_protected, all_owner) =
            crate::fs_secure::assert_owner_only_acl_for_test(&dir.path().join("profiles.json"));
        assert_eq!(ace_count, 1, "profiles.json must have 1 ACE after harden; got {ace_count}");
        assert!(dacl_protected, "profiles.json must have SE_DACL_PROTECTED");
        assert!(all_owner, "profiles.json ACE must belong to the current user SID");
    }

    /// P6.1 triangulation — `harden_existing_credential_files` is a no-op for
    /// non-existent files (must not panic or return an error).
    #[test]
    fn harden_existing_credential_files_skips_nonexistent_files() {
        let dir = TempDir::new().expect("TempDir creation");
        // No files created — helper must silently skip them.
        harden_existing_credential_files(dir.path()); // must not panic
    }

    /// P5.1 — On Windows, vault.json must have an owner-only DACL after creation.
    ///
    /// Asserts: exactly 1 ACE, DACL protected, ACE belongs to the current user.
    /// This test is the RED gate: it will FAIL until P5.2 replaces the old
    /// write+rename+#[cfg(unix)] block with `crate::fs_secure::secure_write`.
    #[cfg(windows)]
    #[test]
    fn vault_save_to_disk_produces_owner_only_dacl() {
        let dir = TempDir::new().expect("TempDir creation");
        let _vault = make_vault(&dir);

        let vault_path = dir.path().join("vault.json");
        let (ace_count, dacl_protected, all_owner) =
            crate::fs_secure::assert_owner_only_acl_for_test(&vault_path);

        assert_eq!(
            ace_count, 1,
            "vault.json DACL must have exactly 1 ACE; got {ace_count}"
        );
        assert!(
            dacl_protected,
            "vault.json DACL must have SE_DACL_PROTECTED set (no inherited ACEs)"
        );
        assert!(
            all_owner,
            "The single ACE must belong to the current user SID"
        );
    }

    /// P5.1 triangulation — On Unix, vault.json must have mode 0o600 after creation.
    #[cfg(unix)]
    #[test]
    fn vault_save_to_disk_produces_0600_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().expect("TempDir creation");
        let _vault = make_vault(&dir);

        let vault_path = dir.path().join("vault.json");
        let mode = std::fs::metadata(&vault_path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "vault.json must have mode 0o600 on Unix");
    }
}
