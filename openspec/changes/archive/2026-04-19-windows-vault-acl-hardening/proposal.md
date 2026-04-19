# Proposal: Windows Vault ACL Hardening

## Intent

The vault is AES-256-GCM encrypted, but on Windows the ciphertext + salt + nonce + KDF params of `vault.json` and `profiles.json` are readable by other local users via the inherited `Users` group ACL. This enables offline attacks against weak master passwords. Add **defense-in-depth** by writing these files with an owner-only DACL and centralizing all platform permission logic in a single `fs_secure` module, which also closes a pre-existing Unix race window where `chmod 0o600` was applied AFTER rename.

## Scope

### In Scope
- New module `src-tauri/src/fs_secure.rs` exposing `pub(crate) secure_write(path, bytes)`, `pub(crate) harden_file_permissions(path)`, and `pub(crate) best_effort_harden(path)`.
- Refactor `vault.rs::save_to_disk()` and `profile.rs::save_profiles_to_disk()` to use `secure_write` (removes inline `#[cfg(unix)]` blocks from both).
- Fix Unix race: ACL applied to `.tmp` BEFORE rename.
- Windows ACL via `windows` crate target-dep with features `Win32_Foundation`, `Win32_Security`, `Win32_Security_Authorization`, `Win32_System_Threading`.
- Migration: on `vault_unlock` success, re-harden existing `vault.json` + `profiles.json` (idempotent, <1ms).
- Harden `profiles.backup.json` after legacy `fs::copy` in `profile.rs:204`.
- Export flow uses `best_effort_harden` + emits non-fatal frontend warning on failure.
- Tests: new `fs_secure` module tests, new `vault.rs` test module, extended `profile.rs` tests, Windows-only integration test verifying DACL contains ONLY the current-user SID with `GENERIC_ALL` (no `Everyone`/`Users`/`Authenticated Users`).
- Docs: `docs/security.md` note covering ACL behavior, FAT32/network-share silent continue, GPO limitation.

### Out of Scope
- Directory-level ACL on `%APPDATA%\com.cognidevai.nexterm\`.
- Cross-volume rename fallback (copy+delete).
- Any changes to crypto primitives.
- UI toggles for auto-updater or internal-file warnings.

## Capabilities

### New Capabilities
- `vault-storage-security`: secure-by-default file writing of persistent vault/profile state with platform-native owner-only permissions and race-free rename.

### Modified Capabilities
- None.

## Approach

Use the already-transitive `windows` crate (zero new compile cost) with `SetNamedSecurityInfoW` and the `PROTECTED_DACL_SECURITY_INFORMATION` flag to strip inherited ACEs. The DACL grants only `GENERIC_ALL` to the current user SID, resolved via `GetCurrentProcessToken` + `GetTokenInformation(TokenUser)`. All `#[cfg]` branching lives in `fs_secure.rs`; callers remain platform-agnostic. `harden_file_permissions` returns `std::io::Error` (low-level); callers map to `AppError`.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `src-tauri/src/fs_secure.rs` | New | `secure_write`, `harden_file_permissions`, `best_effort_harden` |
| `src-tauri/src/vault.rs` | Modified | Use `secure_write`; add test module |
| `src-tauri/src/profile.rs` | Modified | Use `secure_write`; harden `profiles.backup.json` |
| `src-tauri/src/commands/vault.rs` | Modified | Migration re-harden on `vault_unlock` |
| `src-tauri/src/commands/profile.rs` | Modified | `best_effort_harden` + non-fatal frontend warning for exports |
| `src-tauri/src/lib.rs` | Modified | `pub mod fs_secure` |
| `src-tauri/Cargo.toml` | Modified | Windows target dep + `tempfile` dev-dep |
| `docs/security.md` | New/Modified | Document ACL behavior, FAT32/network/GPO limits |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| FAT32/network silent continue leaves file readable | Med | Content is encrypted; documented limitation |
| ~50 lines of `unsafe` Win32 | Low | Dedicated module + Windows integration test asserting DACL |
| Domain GPO overrides our ACL | Low | Impossible to mitigate at app level; documented |
| Migration re-applies ACL each unlock | Low | Idempotent Win32 call, <1ms |

## Rollback Plan

Remove the `windows` target dep from `Cargo.toml`, restore the `#[cfg(unix)]` blocks in `vault.rs` and `profile.rs`, delete `fs_secure.rs`, revert command-layer edits. No data migration needed — ACLs are metadata only; existing hardened files remain accessible to the owner (benign).

## Dependencies

- `[target.'cfg(windows)'.dependencies] windows = "0.61"` with listed features (already transitive).
- `[dev-dependencies] tempfile = "3"` (already in `Cargo.lock`).

## Success Criteria

- [ ] `cargo test` on Windows passes a DACL verification test asserting `vault.json` and `profiles.json` grant access only to the current-user SID.
- [ ] `vault.rs` and `profile.rs` contain zero `#[cfg(unix)]`/`#[cfg(windows)]` permission blocks.
- [ ] `icacls %APPDATA%\com.cognidevai.nexterm\vault.json` shows only current user with full access, no inherited ACEs.
- [ ] App continues to work on FAT32/network-share (non-fatal export warning at most).

## Next Phase

Split into `sdd-spec` + `sdd-design` — both depend only on this proposal and can run in parallel.
