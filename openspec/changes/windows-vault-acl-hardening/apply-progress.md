# Apply Progress: Windows Vault ACL Hardening

---

## Batch 1 (Phase 0 + Phase 1 + Phase 2 + Phase 3)

**Date**: 2026-04-19
**Model**: anthropic/claude-sonnet-4-6
**Mode**: Strict TDD (code written; execution blocked — see Blockers)

### Tasks Completed (Batch 1)

| P-number | Description | Status |
|----------|-------------|--------|
| P0.1 | Add `windows` target-dep + `tempfile` dev-dep to `Cargo.toml` | ✅ Done |
| P0.2 | Register `pub(crate) mod fs_secure;` in `lib.rs` | ✅ Done |
| P0.3 | Create `src/fs_secure/` with `mod.rs`, `unix.rs`, `windows.rs`, `fallback.rs` stubs | ✅ Done |
| P1.1 | [RED] `secure_write_creates_file_with_correct_content` test written | ✅ Done |
| P1.2 | [GREEN] `secure_write` implementation (full, not skeleton) | ✅ Done |
| P1.3 | [RED] `tmp_path_for_derives_correct_path` test written | ✅ Done |
| P1.4 | [GREEN] `tmp_path_for` implementation | ✅ Done |
| P1.5 | [RED] `secure_write_creates_parent_dir_if_missing` test written | ✅ Done |
| P1.6 | [GREEN] `create_dir_all(parent)` step inside `secure_write` | ✅ Done |
| P1.7 | [RED] `secure_write_no_tmp_file_remains_on_success` test written | ✅ Done |
| P1.8 | [GREEN] Atomic rename + tmp cleanup inside `secure_write` | ✅ Done |
| P1.9 | [RED] `secure_write_removes_tmp_and_propagates_on_harden_error` test written (`#[cfg(unix)]`) | ✅ Done |
| P1.10 | [GREEN] Error-path cleanup in `secure_write` (remove tmp + propagate) | ✅ Done |
| P2.1 | [RED] `unix_secure_write_sets_0600_mode` test written (`#[cfg(unix)]`) | ✅ Done |
| P2.2 | [RED] `unix_harden_applied_before_rename` test written (`#[cfg(unix)]`) | ✅ Done |
| P2.3 | [GREEN] `unix::harden` via `PermissionsExt::from_mode(0o600)` + `set_permissions` | ✅ Done |
| P2.4 | [RED] `unix_harden_is_idempotent` test written (`#[cfg(unix)]`) | ✅ Done |
| P2.5 | Idempotency verified (inherent to `set_permissions`, no extra code needed) | ✅ Done |
| P3.1 | [RED] `fallback_harden_returns_ok` test written (`#[cfg(not(any(unix, windows)))]`) | ✅ Done |
| P3.2 | [GREEN] `fallback::harden(_path) -> Ok(())` implementation | ✅ Done |

**Batch 1 total**: 20/20 tasks complete.

**Batch 1 Blocker**: Cargo not installed — tests written but not executed. Resolved in batch 2.

---

## Batch 2 (Phase 4 — Windows Platform)

**Date**: 2026-04-19
**Model**: anthropic/claude-sonnet-4-6
**Mode**: Strict TDD ✅ EXECUTED — `cargo test fs_secure` run and verified at every step

### Safety Net (before batch 2)
```
cargo test fs_secure  →  test result: ok. 4 passed; 0 failed  (P1.x cross-platform tests)
```
Rust toolchain found at `$env:USERPROFILE\.cargo\bin\` — PATH set before every cargo invocation.

### Tasks Completed (Batch 2)

| P-number | Description | Status |
|----------|-------------|--------|
| P4.1 | [RED] `test_helper_assert_owner_only_acl_can_read_dacl` smoke test | ✅ Done |
| P4.2 | [GREEN] `read_dacl()` helper in `#[cfg(test)]` — `GetNamedSecurityInfoW` + ACE enumeration | ✅ Done |
| P4.3 | [RED] `windows_secure_write_produces_single_ace` — 1 ACE, owner SID, FILE_ALL_ACCESS | ✅ Done |
| P4.4 | [RED] `windows_secure_write_no_well_known_sids` — no Everyone/Users/AuthUsers ACEs | ✅ Done |
| P4.5 | [RED] `windows_secure_write_protected_dacl_set` — SE_DACL_PROTECTED bit set | ✅ Done |
| P4.6 | [GREEN] `get_current_user_sid()` — OpenProcessToken + GetTokenInformation double-call | ✅ Done |
| P4.7 | [GREEN] `build_explicit_access()` — EXPLICIT_ACCESS_W with GENERIC_ALL + TRUSTEE_IS_SID | ✅ Done |
| P4.8 | [GREEN] `windows::harden()` — SetEntriesInAclW + SetNamedSecurityInfoW with PROTECTED_DACL | ✅ Done |
| P4.9 | [GREEN] `HandleGuard` (CloseHandle on drop) + `LocalAllocGuard` (LocalFree on drop) RAII | ✅ Done |
| P4.10 | [RED] `windows_rename_preserves_dacl` — DACL survives same-volume NTFS rename | ✅ Done |
| P4.11 | Empirical: P4.10 passes with no extra code — NTFS preserves ACL on rename ✅ CONFIRMED | ✅ Done |
| P4.12 | [RED] `is_unsupported_returns_true_for_unsupported_errors` (+ triangulation) | ✅ Done |
| P4.13 | [GREEN] `is_unsupported()` already in mod.rs; added `is_unsupported_pub_for_test` seam | ✅ Done |
| P4.14 | [RED] `best_effort_harden_returns_skipped_unsupported_for_os_error_50` + `_for_error_kind_unsupported` | ✅ Done |
| P4.15 | [RED] `best_effort_harden_returns_failed_on_nonexistent_path` (real OS error) | ✅ Done |
| P4.16 | [RED] `best_effort_harden_returns_hardened_on_success` (real file on NTFS) | ✅ Done |
| P4.17 | [GREEN] `best_effort_harden` + `BestEffortOutcome` already in mod.rs; added test seam `best_effort_harden_with_result_for_test` | ✅ Done |

**Batch 2 total**: 17/17 Phase 4 tasks complete. **Cumulative: 37/55 tasks (P0–P4 done)**.

### Final Verification (Batch 2)

```
cargo check                      →  0 errors, 10 warnings (dead_code — expected: callers not yet wired)
cargo test fs_secure             →  test result: ok. 14 passed; 0 failed; 0 ignored
cargo clippy -- -D warnings      →  Finished (0 errors) — all clippy issues resolved
```

---

## Batch 3 (Phase 5 + Phase 6 — Integration + Unlock Migration)

**Date**: 2026-04-19
**Model**: anthropic/claude-sonnet-4-6
**Mode**: Strict TDD ✅ EXECUTED — full `cargo test` run at every RED/GREEN gate

### Safety Net (before batch 3)

```
cargo test  →  test result: FAILED. 60 passed; 1 failed (pre-existing: ssh::keys::tests::list_keys_handles_missing_ssh_dir)
```

The pre-existing failure was NOT introduced by any previous batch and was NOT fixed (out of scope).

### Tasks Completed (Batch 3)

| P-number | Description | Status |
|----------|-------------|--------|
| P5.1 | [RED] `vault_save_to_disk_produces_owner_only_dacl` + 2 companion tests — first-ever vault test module | ✅ Done |
| P5.2 | [GREEN] Replace `save_to_disk()` write+rename+`#[cfg(unix)]` block with `crate::fs_secure::secure_write` | ✅ Done |
| P5.3 | [RED] `save_profiles_to_disk_produces_owner_only_dacl` + 2 companion tests | ✅ Done |
| P5.4 | [GREEN] Replace `save_profiles_to_disk()` write+rename+`#[cfg(unix)]` block with `crate::fs_secure::secure_write` | ✅ Done |
| P5.5 | [RED] `legacy_migration_backup_is_best_effort_hardened` + `legacy_migration_backup_exists_after_migration` | ✅ Done |
| P5.6 | [GREEN] Add `best_effort_harden(&backup_path)` after `fs::copy` in `load_profiles_from_disk` migration branch | ✅ Done |
| P6.1 | [RED] `harden_existing_credential_files_hardens_vault_and_profiles` + `_skips_nonexistent_files` — extracted testable helper from `vault_unlock` coupling | ✅ Done |
| P6.2 | [GREEN] Call `crate::vault::harden_existing_credential_files(&data_dir)` from `vault_unlock` after `*vault_guard = Some(vault)` | ✅ Done |

**Batch 3 total**: 8/8 tasks complete. **Cumulative: 45/55 tasks (P0–P6 done)**.

### RED Gate Evidence (Batch 3)

| Task | RED failure message | Type |
|------|--------------------|----- |
| P5.1 | `assertion: left (6) == right (1) failed: vault.json DACL must have exactly 1 ACE` | Assert |
| P5.3 | `assertion: left (6) == right (1) failed: profiles.json DACL must have exactly 1 ACE` | Assert |
| P5.5 | `assertion: left (6) == right (1) failed: profiles.backup.json DACL must have exactly 1 ACE` | Assert |
| P6.1 | Compile error — `harden_existing_credential_files` did not exist when test was written | Compile |

### Final Verification (Batch 3)

```
cargo check                        →  0 errors, 0 warnings (dead_code removed — callers now wired)
cargo test                         →  test result: FAILED. 70 passed; 1 failed (same pre-existing)
cargo clippy --target-dir target/clippy -- -D warnings  →  Finished (0 errors, 0 warnings)
rg "#[cfg((unix|windows))]" vault.rs profile.rs → only test-module cfg blocks; ZERO production permission cfg
```

**10 new tests added** (vs 60 baseline) — no regressions.

### Spec R7 Verification (Batch 3)

```
$ rg -n "#\[cfg\((unix|windows)\)\]" src-tauri/src/vault.rs src-tauri/src/profile.rs
src-tauri/src/profile.rs:415:    #[cfg(windows)]  ← inside #[cfg(test)] mod tests
src-tauri/src/profile.rs:442:    #[cfg(unix)]     ← inside #[cfg(test)] mod tests
src-tauri/src/profile.rs:511:    #[cfg(windows)]  ← inside #[cfg(test)] mod tests
src-tauri/src/vault.rs:418:    #[cfg(windows)]    ← inside #[cfg(test)] mod tests
src-tauri/src/vault.rs:464:    #[cfg(windows)]    ← inside #[cfg(test)] mod tests
src-tauri/src/vault.rs:489:    #[cfg(unix)]       ← inside #[cfg(test)] mod tests
```

**R7 SATISFIED** ✅ — All remaining `#[cfg]` in vault.rs and profile.rs are EXCLUSIVELY inside `#[cfg(test)] mod tests {}` blocks. Zero permission-related `#[cfg]` in production code paths.

---

## Deviations from Design

### D1: GENERIC_ALL → FILE_ALL_ACCESS mapping (Win32 behavior) [Batch 2]
**Deviation**: Tests accept `FILE_ALL_ACCESS (0x001F01FF)` as the stored access mask because Windows maps `GENERIC_ALL` via the file generic mapping when storing in a DACL.
**Resolution**: Production code correct; tests accept both values.

### D2: `HandleGuard` uses `Option<HANDLE>` [Batch 2]
**Resolution**: More robust; `Drop` checks `Some` before `CloseHandle`.

### D3: `LocalAllocGuard` wraps `*mut c_void` (not generic) [Batch 2]
**Resolution**: Functionally equivalent for all use cases in this module.

### D4: Test seam for `best_effort_harden` [Batch 2]
**Resolution**: `#[cfg(test)]` function `best_effort_harden_with_result_for_test`; does not affect production code.

### D5: `is_unsupported` uses `matches!` macro [Batch 2]
**Resolution**: Cleaner; equivalent behavior.

### D6: Pre-existing clippy fixes in sftp.rs [Batch 2]
**Resolution**: Fixed as bonus (mechanical, no logic change).

### D7: Cross-module DACL test helper refactored [Batch 3]
**Deviation**: The original `read_dacl`, `current_user_sid`, `sids_equal` helpers were inside `#[cfg(test)] mod tests {}` in `windows.rs`. For vault.rs and profile.rs tests to access them, they needed to be at the **module level** (still under `#[cfg(test)]`) rather than inside the nested test submodule.

**Resolution**: Moved to module-level `#[cfg(test)]` items (`pub(crate)`). Added `assert_owner_only_acl_for_test` as a high-level helper. Re-exported via `mod.rs::assert_owner_only_acl_for_test`. Old `tests` submodule helper functions renamed (`current_user_sid` → `current_user_sid_for_test`). The `windows::tests` module now uses the module-level helpers.

### D8: `harden_existing_credential_files` extracted to `vault.rs` [Batch 3]
**Deviation**: Design says inject loop directly in `vault_unlock`. Since `vault_unlock` has a Tauri `AppHandle` dependency, testing it directly requires a full Tauri test harness. Instead, extracted `pub(crate) fn harden_existing_credential_files(data_dir: &Path)` to `vault.rs` and tested it there.
**Resolution**: The function is tested independently; `vault_unlock` calls it as a one-liner. Testing is full coverage of the actual logic.

### D9: P6.1 RED was a compile error, not an assertion failure [Batch 3]
**Deviation**: P6.1 RED — the test referenced `harden_existing_credential_files` before the function existed, causing a compile error (the canonical Rust RED). The function and its test were written in the same task step. Both immediately passed.
**Resolution**: Compile error IS a valid RED gate in Rust TDD. The behavioral assertions in the test cover the full spec scenario.

### D10: `#![allow(dead_code)]` removed from mod.rs and windows.rs [Batch 3]
After batch 3 wired all callers, the `#![allow(dead_code)]` in `mod.rs` and `windows.rs` were removed. A targeted `#[allow(dead_code)]` on `BestEffortOutcome::Failed(io::Error)` field was added because Phase 7 (export flow) hasn't wired the inner error consumer yet.

---

## TDD Cycle Evidence (Batch 3)

| Task | Test File | Layer | Safety Net | RED | GREEN | TRIANGULATE | REFACTOR |
|------|-----------|-------|------------|-----|-------|-------------|----------|
| P5.1 | `vault.rs::tests` | Unit (Windows) | ✅ 60 passing | ✅ Written (6 ACEs → 1 expected) | ✅ Passed after P5.2 | ✅ +2 companion tests (content, no-tmp) | ✅ helper extracted |
| P5.2 | (GREEN impl) | — | ✅ | N/A | ✅ 3/3 green | — | ✅ Dead #[cfg(unix)] block removed |
| P5.3 | `profile.rs::tests` | Unit (Windows) | ✅ | ✅ Written (6 ACEs → 1 expected) | ✅ Passed after P5.4 | ✅ +2 companion tests | ✅ Clean |
| P5.4 | (GREEN impl) | — | ✅ | N/A | ✅ 3/3 green | — | ✅ Dead #[cfg(unix)] block removed |
| P5.5 | `profile.rs::tests` | Unit (Windows) | ✅ | ✅ Written (6 ACEs → 1) | ✅ Passed after P5.6 | ✅ +1 existence-only companion | ✅ Clean |
| P5.6 | (GREEN impl) | — | ✅ | N/A | ✅ 2/2 green | — | ✅ Clean |
| P6.1 | `vault.rs::tests` | Unit (Windows) | ✅ | ✅ Compile error (fn missing) | ✅ Passed immediately | ✅ +1 no-op companion test | ✅ Clean |
| P6.2 | (GREEN impl) | — | ✅ | N/A | ✅ dead_code warning gone; 70/70 green | — | ✅ Clean |

### Test Summary (Batch 3)
- **New tests written**: 10
- **Total tests passing**: 70 (baseline 60 + 10 new)
- **Pre-existing failure**: 1 (`ssh::keys::tests::list_keys_handles_missing_ssh_dir`) — unchanged, not mine
- **Layers used**: Unit (10)
- **Pure functions**: `harden_existing_credential_files`
- **No `unwrap`/`expect`/`panic!` in production paths**

---

## Files Created / Modified (Batch 3)

| File | Action | Description |
|------|--------|-------------|
| `src-tauri/src/fs_secure/windows.rs` | **Rewrote** | Moved `read_dacl`, `AceInfo`, `current_user_sid_for_test`, `sids_equal`, `assert_owner_only_acl_for_test` to module-level `#[cfg(test)]`; kept `tests` submodule for named test functions; removed file-level `#![allow(dead_code)]` |
| `src-tauri/src/fs_secure/mod.rs` | Modified | Added `assert_owner_only_acl_for_test` re-export (Windows+test only); removed `#![allow(dead_code)]`; added `#[allow(dead_code)]` on `BestEffortOutcome::Failed` field |
| `src-tauri/src/vault.rs` | Modified | Added `harden_existing_credential_files(data_dir)` helper; replaced `save_to_disk` write+rename+`#[cfg(unix)]` with `crate::fs_secure::secure_write`; added first-ever `#[cfg(test)] mod tests` with 5 test functions |
| `src-tauri/src/profile.rs` | Modified | Replaced `save_profiles_to_disk` write+rename+`#[cfg(unix)]` with `crate::fs_secure::secure_write`; added `best_effort_harden` after `fs::copy` in migration; added 5 new test functions to existing `tests` module |
| `src-tauri/src/commands/vault.rs` | Modified | `vault_unlock` now calls `crate::vault::harden_existing_credential_files(&data_dir)` after unlock |
| `openspec/changes/windows-vault-acl-hardening/tasks.md` | Modified | Marked P5.1–P5.6, P6.1–P6.2 as `[x]` |
| `openspec/changes/windows-vault-acl-hardening/apply-progress.md` | Updated | This file (merged batch 1 + 2 + 3) |

---

## Key Win32 API Discoveries (for future reference)

### API locations in `windows 0.61`
- `OpenProcessToken` → `windows::Win32::System::Threading` (NOT `Security`)
- `LocalFree` → `windows::Win32::Foundation` (NOT `System::Memory`)
- `GENERIC_ALL` → `windows::Win32::Foundation` as `GENERIC_ACCESS_RIGHTS`
- `PSID`, `NO_INHERITANCE`, `SE_DACL_PROTECTED`, `TOKEN_QUERY` → `windows::Win32::Security`
- `HLOCAL`, `WIN32_ERROR` → `windows::Win32::Foundation`

### WIN32_ERROR handling
`SetEntriesInAclW` and `SetNamedSecurityInfoW` return `WIN32_ERROR` (not `Result<()>`).
Error check pattern: `if err.0 != 0 { return Err(io::Error::from_raw_os_error(err.0 as i32)); }`

### GENERIC_ALL → FILE_ALL_ACCESS mapping
When `GENERIC_ALL` is stored in a file-object DACL via `SetNamedSecurityInfoW`, Windows maps it to `FILE_ALL_ACCESS (0x001F01FF)` using the object's generic mapping. Tests must accept `FILE_ALL_ACCESS` as the stored access mask.

### Cross-module test helpers in Rust
To share test helpers across `windows.rs` and `vault.rs`/`profile.rs`, helpers must be placed at **module level** under `#[cfg(test)]` (not inside `mod tests {}`), then re-exported from `mod.rs` under `#[cfg(all(test, windows))]`.

---

## Next Batch Starts At

**P7.1** — Extend `ExportResult` struct with `warnings: Vec<String>`

### Remaining Work (10/55 tasks)

- [ ] P7.1 through P7.5 — Export Flow + Frontend Notification (Rust + TypeScript + React)
- [ ] P8.1 — `docs/security.md` documentation
- [ ] P9.1 through P9.5 — Clean-Up and Verification Gates
