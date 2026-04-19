# Exploration: windows-vault-acl-hardening

**Date**: 2026-04-19  
**Change**: `windows-vault-acl-hardening`  
**Phase**: explore  
**Model**: anthropic/claude-sonnet-4-6

---

## I. Current State — Unix-only ACL Code (with exact file:line evidence)

### vault.rs — `save_to_disk()` (lines 297–340)

```
src-tauri/src/vault.rs:297   fn save_to_disk(&self) -> Result<(), AppError>
src-tauri/src/vault.rs:322   let tmp_path = self.file_path.with_extension("json.tmp");
src-tauri/src/vault.rs:323   std::fs::write(&tmp_path, &json)                   // ← writes vault.json.tmp
src-tauri/src/vault.rs:326   std::fs::rename(&tmp_path, &self.file_path)        // ← renames to vault.json
src-tauri/src/vault.rs:330   #[cfg(unix)]
src-tauri/src/vault.rs:331   {
src-tauri/src/vault.rs:332       use std::os::unix::fs::PermissionsExt;
src-tauri/src/vault.rs:333       let perms = std::fs::Permissions::from_mode(0o600);
src-tauri/src/vault.rs:334       std::fs::set_permissions(&self.file_path, perms)  // ← only on final file
src-tauri/src/vault.rs:337   }
```

**Call sites that trigger `save_to_disk()`:**
| Call site | File:Line | Trigger |
|---|---|---|
| `Vault::create()` | vault.rs:98 | New vault creation |
| `Vault::store()` | vault.rs:164 | Any credential write |
| `Vault::delete()` | vault.rs:186 | Credential deletion |
| `Vault::delete_by_prefix()` | vault.rs:192 | Profile credential cleanup |
| `Vault::change_master_password()` | vault.rs:224 | Password change |

### profile.rs — `save_profiles_to_disk()` (lines 220–254)

```
src-tauri/src/profile.rs:237  let tmp_path = path.with_extension("json.tmp");
src-tauri/src/profile.rs:238  std::fs::write(&tmp_path, &json)                  // ← writes profiles.json.tmp
src-tauri/src/profile.rs:241  std::fs::rename(&tmp_path, &path)                 // ← renames to profiles.json
src-tauri/src/profile.rs:244  #[cfg(unix)]
src-tauri/src/profile.rs:245  {
src-tauri/src/profile.rs:246      use std::os::unix::fs::PermissionsExt;
src-tauri/src/profile.rs:247      let perms = std::fs::Permissions::from_mode(0o600);
src-tauri/src/profile.rs:248      std::fs::set_permissions(&path, perms)            // ← only on final file
src-tauri/src/profile.rs:251  }
```

**Call sites that trigger `save_profiles_to_disk()`:**
| Call site | File:Line | Trigger |
|---|---|---|
| `commands/profile.rs:57` | save_profile command | Profile create/update |
| `commands/profile.rs:110` | delete_profile command | Profile deletion |
| `commands/profile.rs:153` | reorder_profiles command | Drag-and-drop reorder |
| `commands/profile.rs:488` | import_profiles command | Import flow |
| `profile.rs:211` | load_profiles_from_disk (migration) | Legacy format migration |

### Gap Summary

**On Windows:**  
- `vault.json` is written with whatever ACL the parent directory inherits (typically `Users` group has read access on shared machines).  
- `profiles.json` same situation — exposed to any local user.  
- The `#[cfg(unix)]` block at vault.rs:330 and profile.rs:244 is simply **not compiled** on Windows.  
- No `#[cfg(windows)]` counterpart exists anywhere in the codebase.

---

## II. Files That Need ACL Hardening (Complete Inventory)

| File | Location | Sensitivity | Write path | Currently Protected |
|---|---|---|---|---|
| `vault.json` | `%APPDATA%\com.cognidevai.nexterm\vault.json` | 🔴 HIGH — encrypted vault (AES-256-GCM master file) | vault.rs:334 via `save_to_disk()` | Unix only (0o600) |
| `vault.json.tmp` | Same dir | 🔴 HIGH — transient plaintext-before-rename window | vault.rs:323 via `std::fs::write` | **Never** (race window) |
| `profiles.json` | `%APPDATA%\com.cognidevai.nexterm\profiles.json` | 🟡 MEDIUM — hostnames, ports, usernames, key paths (no passwords) | profile.rs:241 via `save_profiles_to_disk()` | Unix only (0o600) |
| `profiles.json.tmp` | Same dir | 🟡 MEDIUM — same content as profiles.json | profile.rs:238 via `std::fs::write` | **Never** (race window) |
| `profiles.backup.json` | Same dir | 🟡 MEDIUM — legacy migration backup, created once | profile.rs:204 via `std::fs::copy` | **Never** — direct copy, no permission logic |
| Export `.nexterm` file | User-chosen path | 🟠 HIGH if `include_credentials=true` — AES-GCM encrypted blob w/ passwords | commands/profile.rs:318 via `std::fs::write` | **Never** — user path, no permission logic |
| Export plain JSON file | User-chosen path | 🟡 MEDIUM — profile metadata without passwords | commands/profile.rs:322 via `std::fs::write` | **Never** — user path |

### Key Observation on Race Window

In `vault.rs:322-326`, the sequence is:
1. `std::fs::write(&tmp_path, &json)` — writes `vault.json.tmp` with default ACL
2. `std::fs::rename(&tmp_path, &self.file_path)` — renames to `vault.json`
3. `#[cfg(unix)] set_permissions(&self.file_path, perms)` — applies 0o600 AFTER rename

**On Windows** there is a race window (between write and future ACL-set) because the ACL is never set at all. On Unix, there is a smaller but real race between steps 2 and 3. The correct fix sets the ACL on the `.tmp` file (step 1) BEFORE the rename — the ACL is preserved through `rename()` on both NTFS and ext4/APFS.

---

## III. Windows ACL Options — Comparison Table + Recommendation

### Options Evaluated

#### (a) `windows` crate (Microsoft official bindings) — `SetNamedSecurityInfoW`

The `windows` crate is **already a transitive dependency** at versions 0.58.0 and 0.61.3 (confirmed in Cargo.lock). Adding it explicitly as a direct dependency with security features adds **zero new compilation overhead** — those DLLs are already compiled.

Required features: `Win32_Security_Authorization`, `Win32_Foundation`, `Win32_Security`

**API flow:**
```rust
// Pseudocode
let sid = get_current_user_sid();   // GetTokenInformation / GetCurrentProcessToken
let dacl = build_owner_only_dacl(sid, GENERIC_ALL | FILE_ALL_ACCESS);
SetNamedSecurityInfoW(path, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, None, None, Some(dacl), None)
```

| Attribute | Assessment |
|---|---|
| **Maintenance** | ✅ Official Microsoft crate, continuously updated, matches Windows SDK |
| **Binary size impact** | ✅ Zero (already transitive dep) — only feature flags added |
| **Error handling** | ✅ Returns `windows::core::Result<()>` — maps to `HRESULT` |
| **Owner-only DACL** | ✅ Full control: explicit `DENY Everyone` + `ALLOW CurrentUser:GENERIC_ALL` with `PROTECTED_DACL_SECURITY_INFORMATION` to prevent inheritance |
| **Complexity** | ⚠️ ~40-60 lines of unsafe Win32 code; well-documented |
| **Test support** | ✅ Can verify via `GetNamedSecurityInfoW` in tests |

#### (b) `windows-acl` crate

A safe wrapper over Win32 ACL APIs.

| Attribute | Assessment |
|---|---|
| **Maintenance** | ⚠️ Last released 2020 (v0.3.0), unmaintained — no updates in 4+ years |
| **Binary size impact** | 🔴 New transitive dep, adds ~100KB |
| **Error handling** | ⚠️ Returns `windows_acl::Error` — less idiomatic |
| **Owner-only DACL** | ✅ Has `ACL::set_user_entry()` / `ACL::remove_all_entries()` |
| **Complexity** | ✅ Simpler API surface (~15 lines) |
| **Test support** | ✅ Can verify via `ACL::get()` |

**Verdict**: Do NOT use — unmaintained since 2020, would be a maintenance liability.

#### (c) `winapi` crate (older approach)

| Attribute | Assessment |
|---|---|
| **Maintenance** | ⚠️ Deprecated in favor of `windows` crate; community recommends migration |
| **Binary size impact** | 🔴 New dep, ~300KB |
| **Error handling** | ⚠️ Raw `BOOL` returns, manual `GetLastError()` calls |
| **Owner-only DACL** | ✅ Technically possible but verbose |
| **Complexity** | 🔴 Most verbose — pure unsafe C-style code |

**Verdict**: Do NOT use — superseded by `windows` crate.

#### (d) `icacls.exe` subprocess

```rust
std::process::Command::new("icacls")
    .args([path, "/inheritance:r", "/grant:r", &format!("{username}:(F)")])
    .output()
```

| Attribute | Assessment |
|---|---|
| **Maintenance** | ✅ OS-provided, no dep needed |
| **Binary size impact** | ✅ Zero |
| **Error handling** | ⚠️ Must parse stdout/stderr strings — fragile, locale-dependent |
| **Owner-only DACL** | ✅ `icacls path /inheritance:r /grant:r "DOMAIN\User:(F)"` |
| **Complexity** | ⚠️ ~20 lines + requires resolving current username (env var unreliable in service accounts) |
| **Test support** | 🔴 Hard to test — subprocess spawning not mockable in unit tests |
| **Security** | ⚠️ Path injection risk if path not properly quoted |
| **CI compatibility** | ✅ Works on GitHub Actions Windows runners |

**Verdict**: Avoid for primary implementation. Acceptable as a last-resort fallback if Win32 API fails, but not the primary path.

#### (e) `std::os::windows::fs::OpenOptionsExt` with security_qos_flags

`OpenOptionsExt::security_qos_flags()` controls **impersonation level** for named pipes/RPC, NOT file ACLs. It does **not** set DACL permissions. This is a red herring — it cannot restrict who reads the file.

**Verdict**: Irrelevant for ACL hardening.

### Recommendation: Option (a) — `windows` crate direct dependency

**Justification:**
1. **Zero incremental cost** — already in the compiled dependency graph; adding explicit dependency only adds the requested feature flags for the linker
2. **Official, actively maintained** — Microsoft ships updates alongside Windows SDK releases
3. **Full DACL control** — `PROTECTED_DACL_SECURITY_INFORMATION` flag prevents ACL inheritance from parent directory; we can grant `GENERIC_ALL` to current user SID only and explicitly deny everyone else
4. **Testable** — `GetNamedSecurityInfoW` lets tests verify ACL state after write
5. **Consistent with codebase** — already uses unsafe via other transitive deps; adding a small unsafe block in a dedicated helper is consistent

**Feature flags needed:**
```toml
[target.'cfg(windows)'.dependencies]
windows = { version = "0.61", features = [
    "Win32_Foundation",
    "Win32_Security",
    "Win32_Security_Authorization",
    "Win32_System_Threading",  # for OpenProcessToken / GetCurrentProcessToken
] }
```

---

## IV. Proposed `secure_write` Abstraction

### Signature and Location

**File**: `src-tauri/src/fs_secure.rs` (new module, registered in `lib.rs`)

```rust
// fs_secure.rs — Secure file write with owner-only permissions
//
// Provides a cross-platform `secure_write(path, bytes)` that:
//   - Unix:    writes bytes, applies 0o600 permissions BEFORE rename
//   - Windows: writes bytes, applies owner-only DACL BEFORE rename
//   - Other:   falls back to standard write (no permission hardening)

/// Write `bytes` atomically to `path` with owner-only file permissions.
///
/// Uses a temp file (`<path>.tmp`) for atomicity. The permission hardening
/// is applied to the TEMP file BEFORE the rename, eliminating the race
/// window where the file is world-readable for any interval.
///
/// # Errors
/// Returns `AppError::VaultError` / `AppError::ProfileError` on any I/O
/// or permission failure (caller wraps with context via `map_err`).
///
/// # Platform behavior
/// - Unix: sets `0o600` (owner read+write, no group/world bits)
/// - Windows: sets a protected DACL granting GENERIC_ALL to the current
///   user SID only, removing all inherited ACEs.
/// - Other: plain write (no hardening, no error)
pub fn secure_write(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error>
```

### Implementation sketch

```rust
pub fn secure_write(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp_path = path.with_extension(
        format!("{}.tmp", path.extension().and_then(|e| e.to_str()).unwrap_or(""))
    );

    // Write to temp file
    std::fs::write(&tmp_path, bytes)?;

    // Harden permissions on temp file BEFORE rename
    harden_file_permissions(&tmp_path)?;

    // Atomic rename
    std::fs::rename(&tmp_path, path)?;

    Ok(())
}

#[cfg(unix)]
fn harden_file_permissions(path: &Path) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
}

#[cfg(windows)]
fn harden_file_permissions(path: &Path) -> Result<(), std::io::Error> {
    // Windows ACL implementation using windows crate
    // (see design phase for full implementation)
    windows_set_owner_only_acl(path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::PermissionDenied, e.to_string()))
}

#[cfg(not(any(unix, windows)))]
fn harden_file_permissions(_path: &Path) -> Result<(), std::io::Error> {
    Ok(()) // No hardening on unknown platforms — best effort
}
```

### Callers updated

| Current call site | Change |
|---|---|
| `vault.rs:save_to_disk()` lines 321-337 | Replace `std::fs::write + rename + #[cfg(unix)]` block with `secure_write(&tmp_path → handled internally, &path, &json.as_bytes())` |
| `profile.rs:save_profiles_to_disk()` lines 236-251 | Same replacement |
| `commands/profile.rs:export_profiles()` lines 317-323 | Add `secure_write` for the export file (both plain and encrypted cases) — only when writing to app data dir; user-chosen paths are left with default ACL |

> **Note on export path**: The export file lands wherever the user chose via the file dialog (could be a FAT32 USB drive, network share, etc.). ACL hardening for exports should be **best-effort** — call `harden_file_permissions` but swallow the error and log a warning rather than failing the export.

### The `#[cfg]` consolidation

By centralizing all platform branching in `fs_secure.rs`, **zero `#[cfg]` attributes remain** in `vault.rs` or `profile.rs`. This is the key architectural win — the callers are completely platform-agnostic.

---

## V. Migration Strategy for Existing Vaults

### The Problem

Existing users on Windows already have `vault.json` and `profiles.json` written with default NTFS ACLs (inheriting from `%APPDATA%` — typically granting `Users` group read access on shared/domain machines).

### Options

**Option A — Harden on first write (lazy migration)**  
The new `secure_write` applies the ACL every time the file is written. Any `store_credential`, `delete_profile`, or similar mutation triggers a write, which applies the new ACL. No explicit migration needed. Downside: users who only read and never trigger a write never get hardened.

**Option B — Harden on vault unlock (active migration)**  
In `vault_unlock` (commands/vault.rs:85) and `Vault::unlock()` (vault.rs:107), after successful unlock, call `harden_existing_files(data_dir)` which applies the ACL to `vault.json` and `profiles.json` if they exist.

**Option C — Harden on app startup (aggressive migration)**  
In `lib.rs:run()` setup closure, before `manage(AppState::default())`, call the hardening function unconditionally.

### Recommendation: Option B (harden on vault unlock)

**Reasoning:**
- The vault unlock is the natural security checkpoint — a user who unlocks the vault is "authenticated" and expects security to be enforced from that point forward.
- Covers all existing users on first run after update.
- Avoids hardening in the startup path before Tauri's AppHandle is available (required to get `app_data_dir`).
- Avoids complication in App startup where `data_dir` resolution requires the AppHandle.

**Migration code location**: `commands/vault.rs:vault_unlock()` — after `*vault_guard = Some(vault);`:

```rust
// Harden existing files on first run after update (migration)
if let Some(ref data_dir) = get_app_data_dir(&app) {
    for filename in &["vault.json", "profiles.json"] {
        let path = data_dir.join(filename);
        if path.exists() {
            if let Err(e) = crate::fs_secure::harden_file_permissions(&path) {
                tracing::warn!("Failed to harden ACL on {filename}: {e}");
                // Non-fatal: warn and continue
            }
        }
    }
}
```

---

## VI. Failure Mode Recommendation

### Scenarios Where ACL Setting Can Fail on Windows

1. **FAT32 / exFAT filesystem** — No NTFS ACL support. `SetNamedSecurityInfoW` returns `ERROR_NOT_SUPPORTED` (0x32) or `ERROR_INVALID_FUNCTION`.
2. **Network drive (SMB share)** — ACL manipulation may be blocked by server policy.
3. **Insufficient privileges** (rare) — User cannot modify ACLs on their own files (possible on locked-down corporate environments).
4. **OneDrive/cloud sync folder** — Virtual filesystem may not support NTFS ACLs.

### Options

**(a) Fail hard** — Return error, refuse to write the file. Verdict: **Do NOT use.** The vault is already encrypted; ACL hardening is defense-in-depth. Refusing to write credentials because ACL failed would cause complete loss of functionality for users on FAT32 USB drives or cloud sync folders.

**(b) Warn and continue** — Log a `tracing::warn!`, emit a non-fatal `AppError` that the frontend can surface as a dismissable notification. Verdict: **Best for export files** where the user should know their export has weaker protections.

**(c) Silent continue** — Log at `tracing::debug!`, swallow the error. Verdict: **Best for internal vault/profiles files.** The file is already encrypted (vault.json) or contains no secrets (profiles.json); the ACL is defense-in-depth. Silently continuing is acceptable and avoids alarming users who cannot act on the warning anyway.

### Recommendation

| File | On ACL failure | Reasoning |
|---|---|---|
| `vault.json` | Log `debug!`, continue silently | Already encrypted; ACL is defense-in-depth |
| `profiles.json` | Log `debug!`, continue silently | No passwords; ACL is best-effort |
| Export `.nexterm` (encrypted) | Log `warn!`, return a non-fatal warning to frontend | User should know their export file is world-readable |
| Export plain JSON | Log `warn!`, return non-fatal warning | Metadata visible |

**In `secure_write`**: `harden_file_permissions` returns `Result<(), std::io::Error>`. The caller in `vault.rs` / `profile.rs` should use:
```rust
if let Err(e) = harden_file_permissions(&path) {
    tracing::debug!("ACL hardening skipped (filesystem may not support ACLs): {e}");
    // Continue — write already completed atomically
}
```

---

## VII. Testing Plan (Unit + Integration) — TDD Compatible

### Existing Test Infrastructure

The codebase uses plain `cargo test` with `#[cfg(test)]` modules inline (not a separate `tests/` directory). `profile.rs` tests use `std::env::temp_dir()` with `uuid::Uuid::new_v4()` for unique temp directories (lines 375-405). `vault.rs` has **no tests at all** — this is a gap that should be addressed in this change.

`tempfile` crate (v3.27.0) is already a transitive dependency. It can be added as a `dev-dependency` to use `tempfile::TempDir` in tests.

### Test Plan

#### Unit Tests — `src-tauri/src/fs_secure.rs` (new module)

```rust
#[cfg(test)]
mod tests {
    // Test 1: basic write + read
    fn secure_write_creates_file_with_correct_content()
    
    // Test 2: atomicity — no .tmp file left after success
    fn secure_write_no_tmp_leftover_on_success()
    
    // Test 3: permissions on Unix
    #[cfg(unix)]
    fn secure_write_sets_0600_on_unix()
    
    // Test 4: permissions on Windows  
    #[cfg(windows)]
    fn secure_write_sets_owner_only_acl_on_windows()
    
    // Test 5: idempotent — calling twice doesn't break the file
    fn secure_write_idempotent()
}
```

#### Windows ACL Verification Helper (Windows-only test)

```rust
#[cfg(windows)]
fn assert_owner_only_acl(path: &Path) {
    // Use GetNamedSecurityInfoW to retrieve the DACL
    // Enumerate ACEs, assert only 1 ACE (current user SID, GENERIC_ALL)
    // Assert no ACE for S-1-1-0 (Everyone), S-1-5-32-545 (Users), S-1-5-11 (Authenticated Users)
}
```

#### Vault-level Tests — `src-tauri/src/vault.rs` (new test module)

```rust
#[cfg(test)]
mod tests {
    // Test 1: vault create sets correct permissions
    #[cfg(windows)]
    fn vault_create_sets_owner_only_acl()
    
    // Test 2: vault store updates file with correct permissions
    #[cfg(windows)]
    fn vault_store_preserves_owner_only_acl()
    
    // Test 3: change_master_password re-applies permissions
    #[cfg(windows)]
    fn vault_change_password_reapplies_acl()
}
```

#### Profile-level Tests — `src-tauri/src/profile.rs` (extend existing)

```rust
// Test: save_profiles_to_disk sets ACL on Windows
#[cfg(windows)]
fn save_profiles_sets_owner_only_acl()
```

### CI Strategy

The release workflow (`release.yml:64`) already runs `windows-latest` as part of the matrix. `cargo test` runs at line 110 for all platforms. Windows-only tests decorated with `#[cfg(windows)]` will run automatically on the Windows-latest runner.

**No additional CI configuration needed** — the Windows runner is already present.

### Fallback if No Windows Runner

If running tests locally on Linux/macOS:
- Windows-specific tests (`#[cfg(windows)]`) compile to nothing — no test failures
- Unix permission tests still run and verify the Unix path
- ACL correctness on Windows is only validated in CI or locally on Windows

---

## VIII. Dependency Plan

### New Direct Dependency

```toml
# Cargo.toml — new entry under [target.'cfg(windows)'.dependencies]
[target.'cfg(windows)'.dependencies]
windows = { version = "0.61", features = [
    "Win32_Foundation",
    "Win32_Security",
    "Win32_Security_Authorization",
    "Win32_System_Threading",
] }
```

**Why version 0.61?** It is already in the transitive dep graph (Cargo.lock shows `windows = "0.61.3"`). Declaring 0.61 as a direct dep with our needed features will unify with the existing transitive resolution — **zero additional compilation** on Windows builds.

**Compile-time impact:**
- On Windows: feature-gated symbols are already compiled; adding the features flag only exposes them to our code. Estimated overhead: <5 seconds added to incremental builds (only new feature-flag expansion).
- On macOS/Linux: `[target.'cfg(windows)'.dependencies]` means this dep is **not compiled at all** — zero impact on non-Windows builds.

### New Dev Dependency (for tests)

```toml
[dev-dependencies]
tempfile = "3"  # already in Cargo.lock at 3.27.0 — zero new download
```

### No Other New Dependencies

| Considered | Decision | Reason |
|---|---|---|
| `windows-acl` crate | ❌ Rejected | Unmaintained since 2020 |
| `winapi` crate | ❌ Rejected | Superseded by `windows` crate |
| `icacls` subprocess | ❌ Rejected as primary | Not testable; locale-sensitive |
| `file-owner` crate | ❌ Rejected | Unix-only; no Windows ACL support |

---

## IX. Open Questions for the Propose Phase

1. **`harden_file_permissions` public API**: Should it be `pub` (accessible from `commands/vault.rs` for migration) or `pub(crate)`? Given single-crate structure, `pub(crate)` is the right choice.

2. **Export path handling**: Should `secure_write` be called for user-chosen export paths (potentially FAT32 / network drives), or should a separate `best_effort_harden` wrapper be used that never fails? The design phase should decide the API boundary.

3. **Migration ordering**: The `vault_unlock` migration (Option B) hardens `vault.json` and `profiles.json` on every unlock. Should this be guarded by a one-time flag (e.g., a `acl_hardened = true` field in vault metadata) to avoid redundant Win32 calls? Probably not worth the complexity — `GetNamedSecurityInfoW` is fast (<1ms).

4. **`vault.json.tmp` → `vault.json` rename on Windows**: NTFS preserves ACLs through `rename()` when source and destination are on the same volume. This should be confirmed by an integration test. Cross-volume moves are not atomic and fall back to copy+delete — this edge case (vault on different drive than temp dir) would need handling.

5. **Directory ACL**: Should the `%APPDATA%\com.cognidevai.nexterm\` directory itself receive hardened ACLs? This would prevent other users from listing the directory. Currently not in scope — the file-level ACLs are sufficient for the stated goal.

6. **`profiles.backup.json`**: Created by `std::fs::copy` (profile.rs:204-208) during legacy migration — not via `secure_write`. Should this copy be replaced with a `secure_copy` that applies ACLs? It's a one-time file that only exists during migration from an old version. Low priority but worth noting.

7. **`harden_file_permissions` on inherited ACL reset**: When we call `SetNamedSecurityInfoW` with `PROTECTED_DACL_SECURITY_INFORMATION`, we remove inherited ACEs. If the user is on a domain where Group Policy reasserts ACLs via a GPO, our ACL might be overwritten. This is an edge case that cannot be mitigated at the app level — document it.

8. **Error type for `harden_file_permissions`**: Should it return `std::io::Error` or `AppError`? The `fs_secure` module should be low-level and return `std::io::Error` to avoid a circular dependency (AppError imports from vault/profile modules). Callers map to `AppError`.

---

## Affected Areas

- `src-tauri/src/vault.rs` — remove inline `#[cfg(unix)]` block from `save_to_disk()`, replace with `secure_write` call
- `src-tauri/src/profile.rs` — remove inline `#[cfg(unix)]` block from `save_profiles_to_disk()`, replace with `secure_write` call
- `src-tauri/src/commands/profile.rs` — add `secure_write` (or `best_effort_harden`) for export file writes (lines 317-323)
- `src-tauri/src/commands/vault.rs` — add migration logic in `vault_unlock` (after line 95) and `vault_create` (after line 79)
- `src-tauri/src/fs_secure.rs` — **new file**: `secure_write`, `harden_file_permissions`, platform impls
- `src-tauri/src/lib.rs` — add `pub mod fs_secure;`
- `src-tauri/Cargo.toml` — add `[target.'cfg(windows)'.dependencies]` block + `[dev-dependencies] tempfile`

---

## Recommendation

**Proceed to Propose phase.** The investigation is complete. All code paths are understood, the preferred implementation approach is clear (windows crate, already transitive), and the abstraction location is determined (`fs_secure.rs`). No blockers identified.

The total scope is small: 1 new module (~100 lines), modifications to 2 existing files (remove ~10 lines each, add 1 function call each), migration in vault_unlock (~10 lines). The Windows-specific unsafe block is approximately 50 lines of Win32 API code. Well within a single implementation task.
