# Tasks: Windows Vault ACL Hardening

## Execution Notes

- **Strict TDD is ACTIVE for Rust**: every `implement X` task MUST be preceded by the corresponding `write failing test for X` task. RED → GREEN → refactor.
- **Phases 1–4** can be compiled and the non-Windows tests can be run locally on any OS. Phase 4 tests require Windows CI / local Windows runner for execution (they compile on all platforms via `#[cfg(windows)]`).
- **Phases 5–7** depend on Phases 1–4 being complete (they import from `fs_secure`).
- **Phases 8–9** are finalization and can be parallelized with Phases 5–7.

---

## Phase 0 — Preparation

- [x] P0.1 — Update `src-tauri/Cargo.toml`: add `[target.'cfg(windows)'.dependencies] windows = { version = "0.61", features = ["Win32_Foundation","Win32_Security","Win32_Security_Authorization","Win32_System_Threading"] }` and `[dev-dependencies] tempfile = "3"`.
- [x] P0.2 — Register module in `src-tauri/src/lib.rs`: add `pub(crate) mod fs_secure;`.
- [x] P0.3 — Create empty directory module `src-tauri/src/fs_secure/` with stub files: `mod.rs`, `unix.rs`, `windows.rs`, `fallback.rs` (each contains only a module-level comment and a `todo!()` placeholder function so it compiles).

---

## Phase 1 — Cross-Platform Foundation (TDD)

- [x] P1.1 — **[RED]** Write failing test in `fs_secure/mod.rs`: `secure_write_creates_file_with_correct_content` — uses `tempfile::TempDir`, writes known bytes, asserts file content matches.
- [x] P1.2 — **[GREEN]** Implement `secure_write(path, bytes)` skeleton in `fs_secure/mod.rs` (returns `unimplemented!()` or stub) until test passes with minimal logic; wire `tmp_path_for` call.
- [x] P1.3 — **[RED]** Write failing test: `tmp_path_for_derives_correct_path` — asserts `tmp_path_for(Path::new("/foo/bar.json"))` returns `"/foo/bar.json.tmp"`.
- [x] P1.4 — **[GREEN]** Implement `tmp_path_for(path: &Path) -> PathBuf` in `mod.rs`.
- [x] P1.5 — **[RED]** Write failing test: `secure_write_creates_parent_dir_if_missing` — destination path has a non-existent intermediate directory; assert it is created and write succeeds.
- [x] P1.6 — **[GREEN]** Implement `create_dir_all(parent)` step inside `secure_write`.
- [x] P1.7 — **[RED]** Write failing test: `secure_write_no_tmp_file_remains_on_success` — after a successful `secure_write`, asserts the `.tmp` path does NOT exist.
- [x] P1.8 — **[GREEN]** Implement atomic rename + tmp cleanup inside `secure_write`.
- [x] P1.9 — **[RED]** Write failing test: `secure_write_removes_tmp_and_propagates_on_harden_error` — inject a `harden_file_permissions` that returns an error; assert the `.tmp` file is removed AND the error is propagated.
- [x] P1.10 — **[GREEN]** Implement error-path cleanup in `secure_write`: on `harden` failure, call `remove_file(tmp)` (ignore its result), propagate original error.

---

## Phase 2 — Unix Platform (TDD, `#[cfg(unix)]`)

- [x] P2.1 — **[RED]** Write failing test `#[cfg(unix)]`: `unix_secure_write_sets_0600_mode` — after `secure_write`, assert `fs::metadata(path).permissions().mode() & 0o777 == 0o600`.
- [x] P2.2 — **[RED]** Write failing test `#[cfg(unix)]`: `unix_harden_applied_before_rename` — expose `harden_file_permissions` directly; write to a tmp path, call `harden_file_permissions(tmp)`, assert mode is `0o600` before any rename happens (verifies race is closed at the tmp level).
- [x] P2.3 — **[GREEN]** Implement `unix::harden(path)` in `fs_secure/unix.rs`: `PermissionsExt::from_mode(0o600)` via `set_permissions`.
- [x] P2.4 — **[RED]** Write failing test `#[cfg(unix)]`: `unix_harden_is_idempotent` — call `harden_file_permissions` twice on same file, assert no error and mode stays `0o600`.
- [x] P2.5 — Verify P2.4 passes with no additional code changes (idempotency is inherent to `set_permissions`).

---

## Phase 3 — Fallback Platform

- [x] P3.1 — **[RED]** Write test `#[cfg(not(any(unix, windows)))]`: `fallback_harden_returns_ok` — calls `fallback::harden` on a path, asserts `Ok(())`.
- [x] P3.2 — **[GREEN]** Implement `fallback::harden(_path: &Path) -> io::Result<()> { Ok(()) }` in `fs_secure/fallback.rs`.

---

## Phase 4 — Windows Platform (TDD, `#[cfg(windows)]`)

- [x] P4.1 — **[RED]** Write failing test `#[cfg(windows)]`: `test_helper_assert_owner_only_acl_can_read_dacl` — create a normal file, call `assert_owner_only_acl(path)`, assert it does not panic and returns ACE count ≥ 0 (smoke-test the helper itself before using it in real assertions).
- [x] P4.2 — **[GREEN]** Implement test helper `assert_owner_only_acl(path: &Path)` inside `#[cfg(test)]` in `windows.rs`: calls `GetNamedSecurityInfoW`, enumerates DACL ACEs, returns the ACE count (and the ACE details for further assertions).
- [x] P4.3 — **[RED]** Write failing test `#[cfg(windows)]`: `windows_secure_write_produces_single_ace` — after `secure_write`, assert `assert_owner_only_acl` finds exactly 1 ACE with `GENERIC_ALL` mask belonging to the current user SID.
- [x] P4.4 — **[RED]** Write failing test `#[cfg(windows)]`: `windows_secure_write_no_well_known_sids` — after `secure_write`, assert no ACE exists for `S-1-1-0` (Everyone), `S-1-5-32-545` (Users), or `S-1-5-11` (Authenticated Users).
- [x] P4.5 — **[RED]** Write failing test `#[cfg(windows)]`: `windows_secure_write_protected_dacl_set` — after `secure_write`, retrieve security descriptor, assert the `SE_DACL_PROTECTED` control bit is set (no inherited ACEs).
- [x] P4.6 — **[GREEN]** Implement `windows::get_current_user_sid() -> io::Result<Vec<u8>>` in `windows.rs`: `OpenProcessToken` + `GetTokenInformation(TokenUser)` + copy SID bytes; wrap token handle in `HandleGuard` RAII.
- [x] P4.7 — **[GREEN]** Implement `windows::build_explicit_access(sid: &[u8]) -> EXPLICIT_ACCESS_W`: `grfAccessPermissions=GENERIC_ALL`, `grfAccessMode=SET_ACCESS`, `grfInheritance=NO_INHERITANCE`, `Trustee{TrusteeForm=TRUSTEE_IS_SID, ptstrName=sid_ptr}`.
- [x] P4.8 — **[GREEN]** Implement `windows::harden(path: &Path) -> io::Result<()>`: wire SID + `SetEntriesInAclW` (wrap result in `LocalAllocGuard`) + wide-string path + `SetNamedSecurityInfoW` with `DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION`; map Win32 errors via `io::Error::from_raw_os_error`.
- [x] P4.9 — **[GREEN]** Implement `HandleGuard` (wraps `HANDLE`, `Drop` calls `CloseHandle`) and `LocalAllocGuard<T>` (wraps `*mut T`, `Drop` calls `LocalFree`) RAII helpers in `windows.rs`; annotate all `unsafe` blocks with `// SAFETY:` comments.
- [x] P4.10 — **[RED]** Write failing test `#[cfg(windows)]`: `windows_rename_preserves_dacl` — write a file with `secure_write`, then perform a second `std::fs::rename` to a new path in the same dir, assert `assert_owner_only_acl` still shows a single owner-only ACE on the renamed path.
- [x] P4.11 — Verify P4.10 passes with no additional code (relies on NTFS same-volume rename behavior). ✅ CONFIRMED: test passes, NTFS same-volume rename preserves DACL.
- [x] P4.12 — **[RED]** Write failing test `#[cfg(windows)]`: `is_unsupported_returns_true_for_unsupported_errors` — assert `is_unsupported(io::Error::from(io::ErrorKind::Unsupported))` is `true` and that raw OS errors `1` and `50` also return `true`.
- [x] P4.13 — **[GREEN]** Implement `is_unsupported(e: &io::Error) -> bool` in `mod.rs`: checks `ErrorKind::Unsupported` and raw OS codes `1` / `50`. Already implemented in batch 1; batch 2 adds test-seam `is_unsupported_pub_for_test`.
- [x] P4.14 — **[RED]** Write failing test: `best_effort_harden_returns_skipped_unsupported` — mock/force `harden_file_permissions` to return an `Unsupported` error; assert `best_effort_harden` returns `BestEffortOutcome::SkippedUnsupported`. Uses `best_effort_harden_with_result_for_test` test seam.
- [x] P4.15 — **[RED]** Write failing test: `best_effort_harden_returns_failed_on_other_error` — force `harden_file_permissions` to return a `PermissionDenied` error; assert `best_effort_harden` returns `BestEffortOutcome::Failed(e)`. Uses non-existent path to trigger real OS error.
- [x] P4.16 — **[RED]** Write failing test: `best_effort_harden_returns_hardened_on_success` — call `best_effort_harden` on a real writable file in a temp dir; assert `BestEffortOutcome::Hardened`.
- [x] P4.17 — **[GREEN]** Implement `best_effort_harden(path: &Path) -> BestEffortOutcome` in `mod.rs`; implement `BestEffortOutcome` enum; add `tracing::debug!` for `SkippedUnsupported`, `tracing::warn!` for `Failed`. Already implemented in batch 1; batch 2 adds test coverage.

---

## Phase 5 — Integration with `vault.rs` and `profile.rs` (TDD)

- [x] P5.1 — **[RED]** Write failing test `#[cfg(windows)]` in `vault.rs` test module (create it — first ever): `vault_save_to_disk_produces_owner_only_dacl` — creates a `TempDir`, initializes a vault at that path, calls `save_to_disk()`, calls `assert_owner_only_acl(vault.json)`. ✅ CONFIRMED RED: 6 ACEs vs 1 expected.
- [x] P5.2 — **[GREEN]** Refactor `vault.rs::save_to_disk()` (lines 321–337): replace `fs::write + rename + #[cfg(unix)]` block with a single call to `crate::fs_secure::secure_write(&path, &json_bytes)`. ✅ GREEN: 3/3 tests passing.
- [x] P5.3 — **[RED]** Write failing test `#[cfg(windows)]` in `profile.rs` test module: `save_profiles_to_disk_produces_owner_only_dacl` — creates `TempDir`, writes profiles, calls `assert_owner_only_acl(profiles.json)`. ✅ CONFIRMED RED: 6 ACEs vs 1 expected.
- [x] P5.4 — **[GREEN]** Refactor `profile.rs::save_profiles_to_disk()` (lines 237–251): replace `fs::write + rename + #[cfg(unix)]` block with `crate::fs_secure::secure_write(&path, &json_bytes)`. ✅ GREEN: 3/3 tests passing.
- [x] P5.5 — **[RED]** Write failing test: `legacy_migration_backup_is_best_effort_hardened` — simulate legacy migration path; assert `profiles.backup.json` exists after migration AND on Windows `assert_owner_only_acl` passes (on other platforms just assert the file exists). ✅ CONFIRMED RED: 6 ACEs vs 1 expected.
- [x] P5.6 — **[GREEN]** Update `profile.rs` legacy migration path (~line 206): after `fs::copy(old_path, backup_path)`, call `crate::fs_secure::best_effort_harden(&backup_path)`; match outcome to log appropriately; outcome is never propagated as error. ✅ GREEN: 2/2 tests passing.

---

## Phase 6 — Migration on Vault Unlock (TDD)

- [x] P6.1 — **[RED]** Write failing test `#[cfg(windows)]`: `vault_unlock_re_hardens_existing_files` — set up a `TempDir` with pre-existing `vault.json` and `profiles.json` written without ACL hardening; call the migration helper directly or through `vault_unlock`; assert both files have owner-only DACLs after the call. ✅ Extracted `harden_existing_credential_files(data_dir)` helper; tested directly (RED = compile error before helper existed).
- [x] P6.2 — **[GREEN]** Modify `commands/vault.rs::vault_unlock`: after `*vault_guard = Some(vault);`, calls `crate::vault::harden_existing_credential_files(&data_dir)`. ✅ GREEN: dead_code warning gone; no regressions.

---

## Phase 7 — Export Flow + Frontend Notification (TDD)

- [ ] P7.1 — Extend `ExportResult` struct (Rust side, in `commands/profile.rs` or its type file): add `warnings: Vec<String>` field; update all construction sites to include `warnings: vec![]` by default.
- [ ] P7.2 — **[RED]** Write failing test: `export_emits_acl_not_applied_warning_when_harden_skipped` — mock or force `best_effort_harden` to return `SkippedUnsupported`; call export logic; assert `result.warnings.contains(&"acl_not_applied".to_string())`.
- [ ] P7.3 — **[GREEN]** Update `commands/profile.rs::export_profiles`: after writing the export file, call `crate::fs_secure::best_effort_harden(&export_path)`; on `SkippedUnsupported` or `Failed`, push `"acl_not_applied"` into `result.warnings`.
- [ ] P7.4 — Update TypeScript export types (locate `src/features/export/` or equivalent): add `warnings: string[]` to the `ExportResult` TS interface / type.
- [ ] P7.5 — Update the export React component: after a successful export, check `result.warnings.includes("acl_not_applied")`; if true, display a non-fatal toast: "Export written, but the file system did not accept owner-only permissions."

---

## Phase 8 — Documentation

- [ ] P8.1 — Create or update `docs/security.md`: document the defense-in-depth model (vault is AES-256-GCM encrypted; ACL is an extra layer), Windows ACL behavior (owner-only DACL, stripped inheritance via `PROTECTED_DACL_SECURITY_INFORMATION`), FAT32 / network-share silent fallback for internal files (encrypted content still protected), GPO reassertion as known limitation (cannot be mitigated at app level), cross-volume rename not supported (`.tmp` co-located by design), and how to manually verify with `icacls %APPDATA%\com.cognidevai.nexterm\vault.json`.

---

## Phase 9 — Clean-Up and Verification Gates

- [ ] P9.1 — Run `cargo test` on a Unix dev machine; assert all non-Windows-gated tests pass.
- [ ] P9.2 — Run `cargo test` on a Windows runner (CI or local); assert all `#[cfg(windows)]` gated tests pass.
- [ ] P9.3 — Run `cargo clippy -- -D warnings` on both platforms; resolve any lint warnings including unsafe-related ones.
- [ ] P9.4 — Manual check: `rg "#\[cfg\((unix|windows)\)\]" src-tauri/src/vault.rs src-tauri/src/profile.rs` returns zero permission-related matches.
- [ ] P9.5 — Manual verification on Windows: `icacls %APPDATA%\com.cognidevai.nexterm\vault.json` shows only the current user with `(F)` and the `(I)` inherited-ACE indicator is absent.

---

## Task Summary

| Category         | Count |
|------------------|-------|
| **Test tasks**   | 27    |
| **Impl tasks**   | 22    |
| **Doc tasks**    | 1     |
| **Verify tasks** | 5     |
| **Total**        | **55**|

> Test tasks: P1.1, P1.3, P1.5, P1.7, P1.9, P2.1, P2.2, P2.4, P3.1, P4.1, P4.3, P4.4, P4.5, P4.10, P4.12, P4.14, P4.15, P4.16, P5.1, P5.3, P5.5, P6.1, P7.2, plus inline assertions in P2.5, P4.11, P9.1–P9.2.
> Implementation tasks: P0.1–P0.3, P1.2, P1.4, P1.6, P1.8, P1.10, P2.3, P2.5, P3.2, P4.2, P4.6–P4.9, P4.13, P4.17, P5.2, P5.4, P5.6, P6.2, P7.1, P7.3–P7.5.
> Documentation: P8.1.
> Verification: P9.1–P9.5.
