# Apply Progress: profile-folder-grouping

**Model**: anthropic/claude-sonnet-4-6
**Date**: 2026-04-20
**Mode**: Strict TDD (RED → GREEN → REFACTOR)

---

## Batch 1 — Phase 0 + Phase 1

### Status: COMPLETE
- Tests before: 75 (74 passing, 1 failing — environmental)
- Tests after: 79 (78 passing, 1 failing — same environmental)
- Delta: +4 new tests
- Clippy: clean (`cargo clippy -- -D warnings` → 0 warnings)

---

## Phase 0 Audit

### profiles.json readers
- `src-tauri/src/vault.rs:335` — `harden_existing_credential_files()` iterates `["vault.json", "profiles.json"]` to harden them. **Does NOT read profile data** — only hardens ACLs. Safe; does not bypass `load_profiles_from_disk`.
- `src-tauri/src/vault.rs:409,417,425,442–447` — Test code only (writes `b"[]"` and reads DACL). Not production readers.
- `src-tauri/src/profile.rs:3,169,173` — Comments and `profiles_file_path()` constructor. These ARE the canonical path source.
- `src-tauri/src/profile.rs:370–530` — Test code only.
- `src-tauri/src/fs_secure/windows.rs:530` — Test code only.

**Verdict**: Only ONE production path constructs the `profiles.json` path: `profile::profiles_file_path()`. All callers go through `load_profiles_from_disk` / `save_profiles_to_disk`. No direct readers bypass the helper. ✅ Safe to proceed with Phase 2.

### load_profiles_from_disk callers
- `src-tauri/src/commands/profile.rs:114` — `load_profiles` command (production)
- `src-tauri/src/profile.rs:216` — self-call inside migration path (production)
- `src-tauri/src/profile.rs:496,532,578,591` — test code only

**Phase 2 impact**: `load_profiles_from_disk` signature will change to return `ProfilesEnvelope` instead of `Vec<ConnectionProfile>`. The call at `commands/profile.rs:114` must be updated in Phase 2/4.

### save_profiles_to_disk callers
- `src-tauri/src/commands/profile.rs:103` — `create_profile` command
- `src-tauri/src/commands/profile.rs:156` — `update_profile` command
- `src-tauri/src/commands/profile.rs:199` — `delete_profile` command
- `src-tauri/src/commands/profile.rs:538` — `import_profiles` command
- `src-tauri/src/profile.rs:216` — internal migration path

**Phase 2 impact**: All 4 command callers will need updating in Phase 4 to use `save_profiles_envelope()`. They are EXPLICITLY excluded from this batch (batch instructions: "Do NOT touch commands/profile.rs").

### Baseline test count
- Total: 75 (74 passing, 1 failing, 0 ignored)
- Known failing (environmental): `ssh::keys::tests::list_keys_handles_missing_ssh_dir` — `.ssh` dir found in this environment

---

## Phase 1 TDD Log

| Task | Test Name | RED Confirmation | GREEN Implementation | Status |
|------|-----------|-----------------|---------------------|--------|
| P1.1 | `folder_serialize_roundtrip` | `error[E0422]: cannot find struct Folder` | Defined `Folder` struct with all fields | ✅ |
| P1.2 | (implementation) | — | `Folder { id, name, display_order, is_system, is_expanded, created_at, updated_at }` + `SYSTEM_FOLDER_NAME` const | ✅ |
| P1.3 | `envelope_serialize_roundtrip` | `error[E0422]: cannot find struct ProfilesEnvelope` | Defined `ProfilesEnvelope { folders, profiles }` | ✅ |
| P1.4 | (implementation) | — | `ProfilesEnvelope` + `PartialEq` on `ConnectionProfile`, `UserCredential`, `AuthMethodConfig`, `TunnelConfig` | ✅ |
| P1.5 | `connection_profile_has_folder_id` | `error[E0609]: no field folder_id` (×5) | Added `folder_id: Option<Uuid>` with `#[serde(default)]` | ✅ |
| P1.6 | (implementation) | — | Field added + `ConnectionProfile::default()` updated + `commands/profile.rs:495` fixed | ✅ |
| P1.7 | REFACTOR | — | `PartialEq` added to all needed types; clippy clean | ✅ |
| P1.8 | `system_folder_name_constant_is_stable` | — (was GREEN/DESIGN task) | `SYSTEM_FOLDER_NAME = "__system__"` already in place; test added | ✅ |
| P1.9 | (AppState.folders) | — | `AppState.folders: Mutex<Vec<Folder>>` added + initialized | ✅ |

---

## Tests Final Count

| Metric | Before | After | Delta |
|--------|--------|-------|-------|
| Total tests | 75 | 79 | +4 |
| Passing | 74 | 78 | +4 |
| Failing | 1 | 1 | 0 |
| Ignored | 0 | 0 | 0 |

New tests added:
1. `profile::tests::folder_serialize_roundtrip`
2. `profile::tests::envelope_serialize_roundtrip`
3. `profile::tests::connection_profile_has_folder_id`
4. `profile::tests::system_folder_name_constant_is_stable`

---

## Files Changed

| File | Action | Lines Added | What |
|------|--------|-------------|------|
| `src-tauri/src/profile.rs` | Modified | ~90 | `Folder` struct, `ProfilesEnvelope` struct, `SYSTEM_FOLDER_NAME` const, `folder_id` on `ConnectionProfile`, `PartialEq` derives, 4 new tests |
| `src-tauri/src/state.rs` | Modified | ~3 | `folders: Mutex<Vec<Folder>>` on `AppState`, `PartialEq` on `TunnelConfig`, import `Folder` |
| `src-tauri/src/commands/profile.rs` | Modified | 1 | Added `folder_id: None` to struct initializer at line 495 |
| `openspec/changes/profile-folder-grouping/tasks.md` | Modified | 0 | P0.1–P0.3, P1.1–P1.9 marked `[x]` |

---

## Clippy Status

```
cargo clippy -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.60s
```

✅ Zero warnings. Zero errors.

---

## Discoveries / Risks

1. **`commands/profile.rs:495` struct initializer regression**: Adding `folder_id` to `ConnectionProfile` broke compilation of the `import_profiles` command — it used an explicit struct literal without `folder_id`. Fixed by adding `folder_id: None`. This is an expected pattern: any future fields added to `ConnectionProfile` will require auditing all explicit constructors. The `Default::default()` spread (`..ConnectionProfile::default()`) protects against this in most test helpers, but explicit production constructors do NOT.

2. **`TunnelConfig` needed `PartialEq`**: Adding `PartialEq` to `ConnectionProfile` cascaded to `TunnelConfig` in `state.rs` because it's a field of `ConnectionProfile`. This is correct behavior — added `PartialEq` to `TunnelConfig`. No functional impact.

3. **`vault.rs` references `profiles.json` but does NOT bypass `load_profiles_from_disk`**: The vault's `harden_existing_credential_files` iterates both files for ACL hardening only. It does NOT read or parse profile data. Safe — no coupling to the dual-format migration.

---

## Checklist

- [x] P0.1 — profiles.json audit complete
- [x] P0.2 — load/save coupling audit complete  
- [x] P0.3 — Baseline captured (75 tests, 74 passing)
- [x] P1.1 — RED: `folder_serialize_roundtrip` compile error confirmed
- [x] P1.2 — GREEN: `Folder` struct defined, test passes
- [x] P1.3 — RED: `envelope_serialize_roundtrip` compile error confirmed
- [x] P1.4 — GREEN: `ProfilesEnvelope` defined, test passes
- [x] P1.5 — RED: `connection_profile_has_folder_id` compile error confirmed
- [x] P1.6 — GREEN: `folder_id: Option<Uuid>` added, test passes
- [x] P1.7 — REFACTOR: PartialEq consistent, clippy clean, 74→78 passing
- [x] P1.8 — DESIGN: `AppState.folders` added, constant stability test written
- [x] P1.9 — `SYSTEM_FOLDER_NAME` constant added (done as part of P1.2 GREEN)
- [x] P2.1 — RED: `migration_detect_format_legacy_array` compile error confirmed
- [x] P2.2 — RED: `migration_detect_format_envelope` compile error confirmed
- [x] P2.3 — RED: `migration_legacy_to_envelope_produces_system_folder` compile error confirmed
- [x] P2.4 — RED: `migration_envelope_format_is_idempotent` compile error confirmed (E0609 no field `folders` on Vec)
- [x] P2.5 — RED: `migration_missing_file_returns_system_folder_envelope` compile error confirmed
- [x] P2.6 — RED: `migration_legacy_writes_backup_before_envelope` compile error confirmed
- [x] P2.7 — RED: `migration_backup_collision_uses_timestamped_variant` compile error confirmed
- [x] P2.8 — RED: `migration_corrupted_json_returns_error_no_modification` compile error confirmed
- [x] P2.9 — GREEN: `ProfilesFormat` enum + `detect_profiles_format` implemented
- [x] P2.10 — GREEN: `migrate_legacy_to_envelope` implemented (pure function)
- [x] P2.11 — GREEN: `backup_path_for` + `write_backup` + `save_profiles_envelope` implemented
- [x] P2.12 — GREEN: `load_profiles_from_disk` rewritten to return `ProfilesEnvelope`
- [x] P2.13 — GREEN: `save_profiles_envelope` implemented (atomic via `secure_write`)
- [x] P2.14 — GREEN: `load_profiles` command updated to populate `AppState.folders`; returns `Vec<ConnectionProfile>` shim
- [x] P2.15 — REFACTOR: `save_profiles_to_disk` kept as Phase 4 shim; all tests green; clippy clean

---

## Batch 2 — Phase 2 Migration

### Status: COMPLETE
- Tests before: 79 (78 passing, 1 failing — environmental)
- Tests after: 87 (86 passing, 1 failing — same environmental)
- Delta: +8 new tests
- Clippy: clean

---

### Phase 2 RED Tests (P2.1–P2.8)

All 8 RED tests written FIRST and confirmed failing with compile errors:

```
error[E0425]: cannot find function `detect_profiles_format` in this scope
error[E0425]: cannot find function `migrate_legacy_to_envelope` in this scope
error[E0609]: no field `folders` on type `Vec<profile::ConnectionProfile>`
error[E0609]: no field `profiles` on type `Vec<profile::ConnectionProfile>`
error[E0433]: cannot find type `ProfilesFormat` in this scope
```

| Task | Test Name | RED Confirmation | GREEN Implementation | Status |
|------|-----------|-----------------|---------------------|--------|
| P2.1 | `migration_detect_format_legacy_array` | E0425 `detect_profiles_format` not found + E0433 `ProfilesFormat` not found | `detect_profiles_format` + `ProfilesFormat` enum | ✅ |
| P2.2 | `migration_detect_format_envelope` | Same E0425/E0433 | Same impl | ✅ |
| P2.3 | `migration_legacy_to_envelope_produces_system_folder` | E0425 `migrate_legacy_to_envelope` not found | `migrate_legacy_to_envelope` pure function | ✅ |
| P2.4 | `migration_envelope_format_is_idempotent` | E0609 no field `folders` on Vec | `load_profiles_from_disk` rewritten to return `ProfilesEnvelope` | ✅ |
| P2.5 | `migration_missing_file_returns_system_folder_envelope` | E0609 no field `folders` on Vec | Same | ✅ |
| P2.6 | `migration_legacy_writes_backup_before_envelope` | E0609 no field `folders` on Vec | `write_backup` + migration path in `load_profiles_from_disk` | ✅ |
| P2.7 | `migration_backup_collision_uses_timestamped_variant` | E0609 no field `folders` on Vec | `backup_path_for` with timestamp collision detection | ✅ |
| P2.8 | `migration_corrupted_json_returns_error_no_modification` | E0609 no field `folders` on Vec | Detection returns Err; file untouched | ✅ |

---

### Phase 2 GREEN Implementations (P2.9–P2.14)

**P2.9 — `ProfilesFormat` enum + `detect_profiles_format`**
- Peeks at JSON root via `serde_json::Value`; Array→LegacyArray, Object→Envelope, other→Error

**P2.10 — `migrate_legacy_to_envelope`**
- Pure function: creates system folder via `make_system_folder()`, assigns all profiles `folder_id = sys.id`, assigns sequential `display_order`

**P2.11 — Backup helpers**
- `backup_path_for()`: returns `profiles.backup.json` if absent, else `profiles.backup.{YYYYMMDD_HHMMSS}.json`
- `write_backup()`: writes backup bytes, applies best-effort ACL hardening

**P2.12 — `load_profiles_from_disk` rewrite**
- Signature: `fn load_profiles_from_disk(app_data_dir: Option<&PathBuf>) -> Result<ProfilesEnvelope, AppError>`
- Missing/empty file → return envelope with system folder + 0 profiles
- LegacyArray → per-profile `migrate_legacy_fields()` + `migrate_legacy_to_envelope()` + `write_backup()` + `save_profiles_envelope()`
- Envelope → deserialize + auto-heal missing system folder
- Corrupted → return Err, file untouched, no backup

**P2.13 — `save_profiles_envelope`**
- Atomic write via `crate::fs_secure::secure_write`

**P2.14 — `commands/profile.rs:load_profiles` adapter**
- Calls `load_profiles_from_disk()` → gets `ProfilesEnvelope`
- Populates `AppState.profiles` AND `AppState.folders` from envelope
- Returns `envelope.profiles` (Vec<ConnectionProfile>) for backward compat

**P2.15 — `save_profiles_to_disk` shim decision**
- Kept as thin legacy-format writer (flat array) for Phase 4 callers
- Documented with "Phase 4 will remove this" comment
- Pre-existing tests updated to use `ProfilesEnvelope` return type

---

### Updated Pre-existing Tests (Phase 2 impact)
- `disk_persistence_roundtrip` — updated to use `envelope.profiles.len()` instead of `loaded.len()`
- `load_from_nonexistent_returns_empty` — updated to assert `envelope.profiles.is_empty()` + `envelope.folders.len() == 1`
- `legacy_migration_backup_exists_after_migration` — updated to use `envelope.profiles.len()`

---

### Files Changed in Batch 2

| File | Action | Lines Added | What |
|------|--------|-------------|------|
| `src-tauri/src/profile.rs` | Modified | ~200 | `ProfilesFormat` enum, `detect_profiles_format`, `make_system_folder`, `migrate_legacy_to_envelope`, `backup_path_for`, `write_backup`, `save_profiles_envelope`, rewrote `load_profiles_from_disk`, 8 new migration tests, updated 3 old tests |
| `src-tauri/src/commands/profile.rs` | Modified | ~10 | `load_profiles` command updated to populate `AppState.folders` from envelope |
| `openspec/changes/profile-folder-grouping/tasks.md` | Modified | 0 | P2.1–P2.15 marked `[x]` |

---

### Tests Count

| Metric | Before | After | Delta |
|--------|--------|-------|-------|
| Total tests | 79 | 87 | +8 |
| Passing | 78 | 86 | +8 |
| Failing | 1 | 1 | 0 |
| Ignored | 0 | 0 | 0 |

New tests added (Batch 2):
1. `profile::tests::migration_detect_format_legacy_array`
2. `profile::tests::migration_detect_format_envelope`
3. `profile::tests::migration_legacy_to_envelope_produces_system_folder`
4. `profile::tests::migration_envelope_format_is_idempotent`
5. `profile::tests::migration_missing_file_returns_system_folder_envelope`
6. `profile::tests::migration_legacy_writes_backup_before_envelope`
7. `profile::tests::migration_backup_collision_uses_timestamped_variant`
8. `profile::tests::migration_corrupted_json_returns_error_no_modification`

---

### Clippy Status: CLEAN
```
cargo clippy -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.96s
```

---

### Discoveries / Risks (Batch 2)

1. **`save_profiles_to_disk` wrapper invariant**: The Phase 4 callers (`save_profile`, `delete_profile`, `reorder_profiles`, `import_profiles`) still write flat-array JSON after every operation. This means: if a user opens the app on Phase 2 code and creates/updates a profile, the next time `load_profiles_from_disk` runs it will see a LegacyArray and re-migrate (creating another backup). This is non-destructive (backup collision strategy handles it) but produces multiple backup files. Phase 4 MUST update all 4 callers to use `save_profiles_envelope` to eliminate this re-migration cycle.

2. **`load_profiles` command repopulates `AppState.folders` on every call**: This is intentional and safe. The frontend always gets a fresh folder list on `load_profiles`. This means folder state is eventually consistent with disk. Phase 3/4 folder CRUD commands will need to maintain `AppState.folders` in-memory as well (not just reload).

3. **Backup timestamp resolution**: If two migrations happen in the same second (e.g., in tests), `backup_path_for` could return the same timestamped filename and `write_backup` would fail. Low-risk in production (migrations are rare), but test isolation via `TempDir` prevents collision in tests.

---

## Batch 3 — Phase 3 Folder CRUD

### Status: COMPLETE
- Tests before: 87 (86 passing, 1 failing — environmental)
- Tests after: 116 (115 passing, 1 failing — same environmental)
- Delta: +29 new tests
- Clippy: clean (`cargo clippy -- -D warnings` → 0 warnings)

---

### Phase 3 RED Tests (P3.1–P3.28) — All confirmed failing before GREEN

Confirmed compile errors across all 29 new test functions:
```
error[E0599]: no method named `create_folder` found for struct `profile::ProfilesEnvelope`
error[E0599]: no method named `rename_folder` found for struct `profile::ProfilesEnvelope`
error[E0599]: no method named `delete_folder` found for struct `profile::ProfilesEnvelope`
error[E0599]: no method named `reorder_folders` found for struct `profile::ProfilesEnvelope`
error[E0599]: no method named `move_profile_to_folder` found for struct `profile::ProfilesEnvelope`
error[E0599]: no method named `reorder_profiles_in_folder` found for struct `profile::ProfilesEnvelope`
error[E0599]: no method named `set_folder_expanded` found for struct `profile::ProfilesEnvelope`
error[E0433]: cannot find type `ProfileError` in this scope
```

| Task | Test Name | RED | GREEN | Status |
|------|-----------|-----|-------|--------|
| P3.1 | `crud_create_folder_happy_path` | E0599 no method `create_folder` | `impl ProfilesEnvelope::create_folder` | ✅ |
| P3.2 | `crud_create_folder_rejects_empty_name` | E0599 + E0433 | `ProfileError::InvalidName` | ✅ |
| P3.3 | `crud_create_folder_rejects_whitespace_name` | E0599 + E0433 | Trim before validate | ✅ |
| P3.4 | `crud_create_folder_rejects_name_over_64_chars` | E0599 + E0433 | `validate_folder_name` checks len>64 | ✅ |
| P3.5 | `crud_create_folder_rejects_duplicate_name_case_insensitive` | E0599 + E0433 | `name_conflicts` case-insensitive | ✅ |
| P3.6 | `crud_rename_folder_happy_path` | E0599 | `impl ProfilesEnvelope::rename_folder` | ✅ |
| P3.7 | `crud_rename_folder_not_found` | E0599 + E0433 | `find_folder_mut` + FolderNotFound | ✅ |
| P3.8 | `crud_rename_folder_system_protected` | E0599 + E0433 | `SystemFolderProtected` guard | ✅ |
| P3.9 | `crud_rename_folder_invalid_name` | E0599 + E0433 | `validate_folder_name` | ✅ |
| P3.10 | `crud_rename_folder_duplicate_name_and_own_name_allowed` | E0599 + E0433 | `name_conflicts` excludes own id | ✅ |
| P3.11 | `crud_delete_folder_empty` | E0599 + E0433 | `impl ProfilesEnvelope::delete_folder` | ✅ |
| P3.12 | `crud_delete_folder_with_profiles_moves_to_system` | E0599 + E0433 | Cascade move + relative order | ✅ |
| P3.13 | `crud_delete_folder_system_protected` | E0599 + E0433 | `SystemFolderProtected` guard | ✅ |
| P3.14 | `crud_delete_folder_not_found` | E0599 + E0433 | `FolderNotFound` | ✅ |
| P3.15 | `crud_reorder_folders_happy_path` | E0599 | `impl ProfilesEnvelope::reorder_folders` | ✅ |
| P3.16 | `crud_reorder_folders_missing_id` | E0599 + E0433 | `IncompleteReorder` if len mismatch | ✅ |
| P3.17 | `crud_reorder_folders_unknown_id` | E0599 + E0433 | `FolderNotFound` for unknown | ✅ |
| P3.18 | `crud_move_profile_to_folder_shifts_siblings` | E0599 | Cross-folder: shift siblings ≥ new_order | ✅ |
| P3.19 | `crud_move_profile_to_folder_unknown_folder` | E0599 + E0433 | `FolderNotFound`, state unchanged | ✅ |
| P3.20 | `crud_move_profile_to_folder_unknown_profile` | E0599 + E0433 | `ProfileNotFound` | ✅ |
| P3.21 | `crud_move_profile_same_folder_reorder` | E0599 | Same-folder: sorted reinsert | ✅ |
| P3.22 | `crud_reorder_profiles_in_folder_happy_path` | E0599 | `impl ProfilesEnvelope::reorder_profiles_in_folder` | ✅ |
| P3.23 | `crud_reorder_profiles_in_folder_missing_id` | E0599 + E0433 | `IncompleteReorder` | ✅ |
| P3.24 | `crud_reorder_profiles_in_folder_unknown_id` | E0599 + E0433 | `ProfileNotFound` | ✅ |
| P3.25 | `crud_reorder_profiles_in_folder_cross_folder_profile` | E0599 + E0433 | `ProfileFolderMismatch` | ✅ |
| P3.26 | `crud_set_folder_expanded_happy_path` | E0599 | `impl ProfilesEnvelope::set_folder_expanded` | ✅ |
| P3.27 | `crud_set_folder_expanded_not_found` | E0599 + E0433 | `FolderNotFound` | ✅ |
| P3.28 | `crud_clone_before_op_proves_no_aliased_state` | E0599 | Clone semantics: snapshot unaffected | ✅ |
| P3.28b | `crud_failed_create_folder_leaves_state_unchanged` | E0599 | Error path: no mutation | ✅ |

---

### GREEN Implementations

**ProfileError enum** (in `src-tauri/src/error.rs`):
- 7 variants: `FolderNotFound`, `ProfileNotFound`, `SystemFolderProtected`, `InvalidName`, `DuplicateName`, `IncompleteReorder`, `ProfileFolderMismatch`
- `impl From<ProfileError> for AppError` (maps to `AppError::ProfileError(msg)`)
- `DeleteFolderResult { moved_profile_count: usize }` in `profile.rs`

**impl ProfilesEnvelope** (in `src-tauri/src/profile.rs`):

Private helpers extracted (P3.29 REFACTOR):
- `find_folder_mut(id) -> Result<&mut Folder, ProfileError>` — lookup by UUID
- `validate_folder_name(name: &str) -> Result<String, ProfileError>` — trim, 1–64 chars
- `name_conflicts(name, exclude_id) -> bool` — case-insensitive, excludes own id for rename
- `system_folder_id() -> Uuid` — finds the system folder (invariant: always present)

Public API methods:
- `create_folder(name) -> Result<Folder, ProfileError>` — returns owned clone
- `rename_folder(folder_id, new_name) -> Result<Folder, ProfileError>` — returns owned clone
- `delete_folder(folder_id) -> Result<DeleteFolderResult, ProfileError>` — cascade to system
- `reorder_folders(ordered_ids) -> Result<(), ProfileError>` — full permutation required
- `move_profile_to_folder(profile_id, target_folder_id, new_order) -> Result<(), ProfileError>` — cross-folder: shift siblings; same-folder: sorted reinsert
- `reorder_profiles_in_folder(folder_id, ordered_profile_ids) -> Result<(), ProfileError>` — scoped permutation
- `set_folder_expanded(folder_id, expanded) -> Result<(), ProfileError>` — idempotent

---

### Design Decisions (Batch 3)

1. **Return type: owned `Folder` clone vs `&Folder`**: Methods `create_folder` and `rename_folder` return `Folder` (owned clone) instead of `&Folder`. Using `&Folder` creates a borrow lifetime conflict in tests and Phase 4 call sites where the caller needs to read the returned value AND further mutate the envelope (e.g. to persist it). Owned clone is ~50 bytes and zero-copy for the UUID — this is the right tradeoff.

2. **`reorder_folders` validation order**: `FolderNotFound` is checked BEFORE `IncompleteReorder`. Rationale: "you gave me an ID I don't know" is more actionable than "you forgot some IDs". Callers get the most specific error.

3. **`reorder_profiles_in_folder` validation order**: `ProfileNotFound` checked before `ProfileFolderMismatch`. If the ID doesn't exist at all, we report that first; if it exists but is in the wrong folder, we report the mismatch.

4. **Rollback contract (P3.28)**: `impl ProfilesEnvelope` methods mutate `&mut self` directly. Callers (Phase 4 Tauri commands) MUST clone before calling if they need rollback-on-persist-failure. The P3.28 test proves that clones are independent (no aliased state). This is documented in the impl block comment.

5. **`move_profile_to_folder` same-folder path**: Uses a sorted-list reinsert approach (collect, remove, insert at new_order clamped to len, reassign sequential display_orders). This compacts display_orders as a side effect — acceptable per design note "gaps allowed, Phase 4 may compact".

---

### REFACTOR Notes (P3.29)

- Extracted 4 private helpers to eliminate code duplication across CRUD methods
- Fixed 2 warnings from unused variables in tests (`_f2`, `_p2_id`)
- Removed `profile_ids_in_folder` unnecessary pre-collection in `move_profile_to_folder`
- `cargo clippy -- -D warnings`: 0 warnings, 0 errors ✅

---

### Files Changed (Batch 3)

| File | Action | Lines Added | What |
|------|--------|-------------|------|
| `src-tauri/src/profile.rs` | Modified | ~310 | `DeleteFolderResult`, `impl ProfilesEnvelope` (7 methods + 4 helpers), 29 new tests |
| `src-tauri/src/error.rs` | Modified | ~45 | `ProfileError` enum (7 variants), `From<ProfileError> for AppError` |
| `openspec/changes/profile-folder-grouping/tasks.md` | Modified | ~20 | P3.1–P3.29 marked [x] |

---

### Tests Count (Batch 3)

| Metric | Before | After | Delta |
|--------|--------|-------|-------|
| Total tests | 87 | 116 | +29 |
| Passing | 86 | 115 | +29 |
| Failing | 1 | 1 | 0 |
| Ignored | 0 | 0 | 0 |

New tests added (Batch 3):
1. `crud_create_folder_happy_path`
2. `crud_create_folder_rejects_empty_name`
3. `crud_create_folder_rejects_whitespace_name`
4. `crud_create_folder_rejects_name_over_64_chars`
5. `crud_create_folder_rejects_duplicate_name_case_insensitive`
6. `crud_rename_folder_happy_path`
7. `crud_rename_folder_not_found`
8. `crud_rename_folder_system_protected`
9. `crud_rename_folder_invalid_name`
10. `crud_rename_folder_duplicate_name_and_own_name_allowed`
11. `crud_delete_folder_empty`
12. `crud_delete_folder_with_profiles_moves_to_system`
13. `crud_delete_folder_system_protected`
14. `crud_delete_folder_not_found`
15. `crud_reorder_folders_happy_path`
16. `crud_reorder_folders_missing_id`
17. `crud_reorder_folders_unknown_id`
18. `crud_move_profile_to_folder_shifts_siblings`
19. `crud_move_profile_to_folder_unknown_folder`
20. `crud_move_profile_to_folder_unknown_profile`
21. `crud_move_profile_same_folder_reorder`
22. `crud_reorder_profiles_in_folder_happy_path`
23. `crud_reorder_profiles_in_folder_missing_id`
24. `crud_reorder_profiles_in_folder_unknown_id`
25. `crud_reorder_profiles_in_folder_cross_folder_profile`
26. `crud_set_folder_expanded_happy_path`
27. `crud_set_folder_expanded_not_found`
28. `crud_clone_before_op_proves_no_aliased_state`
29. `crud_failed_create_folder_leaves_state_unchanged`

---

### Clippy Status: CLEAN
```
cargo clippy -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.95s
```

Zero warnings. Zero errors. ✅

---

### Discoveries / Risks (Batch 3)

1. **Borrow checker + returning `&Folder`**: The natural API `fn create_folder(...) -> Result<&Folder, ProfileError>` is unusable in practice because the returned borrow prevents any further use of `&mut self` in the same scope. The solution is to return an owned `Folder` clone. Phase 4 must be written with this in mind — callers should use the returned `Folder` only for its UUID (to build a response), not as a long-lived reference.

2. **`reorder_profiles_in_folder` validation ordering is an API contract**: The validation order (ProfileNotFound before ProfileFolderMismatch before IncompleteReorder) will be observed by Phase 4 tests. Changing this order in the future is a breaking change to error semantics. Document in Phase 4 PR description.

3. **`move_profile_to_folder` compacts display_orders on same-folder path**: The same-folder reorder path reassigns sequential display_orders (0,1,2...) via sorted-list reinsert. This differs from the cross-folder path which only shifts siblings ≥ new_order. Phase 4 should be aware of this asymmetry — the frontend should always reflect what the backend persists.

---

## Batch 4 — Phase 4 + Phase 5 Tauri Commands + PR A Gate

### Status: COMPLETE
- Tests before: 116 (115 passing, 1 failing — environmental)
- Tests after: 122 (121 passing, 1 failing — same environmental)
- Delta: +6 new tests
- Clippy: clean (`cargo clippy -- -D warnings` → 0 warnings, 0 errors)

---

### Phase 4 TDD Log

| Task | What | RED | GREEN | Status |
|------|------|-----|-------|--------|
| P4.1 | DeleteFolderResult + Serialize | Missing `Serialize` derive on `DeleteFolderResult` | Added `Serialize, Deserialize` + `#[serde(rename_all = "camelCase")]` | ✅ |
| P4.2 | `load_profiles_with_folders` command | Not yet compiled | Implemented with lazy-load pattern | ✅ |
| P4.3 | `load_profiles` backward-compat | Already existed | No change needed | ✅ |
| P4.4 | 7 folder CRUD commands | Not yet compiled | All 7 commands implemented with rollback contract | ✅ |
| P4.5 | Register in invoke_handler | Not registered | All 8 new commands added to `lib.rs` | ✅ |
| P4.6 | Shim removal (4 callers) | save_profiles_to_disk used in commands | All 4 callers refactored to save_profiles_envelope | ✅ |
| P4.7 | Integration test round-trip | Test written first (library layer — immediately GREEN) | `integration_full_round_trip_create_move_delete_folder` | ✅ |
| P4.8 | Persistence invariant test | Test written first (immediately GREEN) | `integration_persisted_json_is_envelope_format` + `integration_delete_folder_result_serializes` | ✅ |

### Phase 5 Status

| Gate | Status | Detail |
|------|--------|--------|
| P5.1 cargo test | ✅ | 122 tests, 121 pass, 1 fail (environmental) |
| P5.2 cargo clippy | ✅ | 0 warnings, 0 errors |
| P5.3 command surface | ✅ | 15 commands in profile.rs, all in invoke_handler |
| P5.4 doc comments | ✅ | All 8 new commands have `///` docs |
| P5.5 no frontend | ✅ | git diff shows only src-tauri/ files |
| P5.6 manual smoke | 🔲 | Pending deploy |

### GREEN Implementations (Batch 4)

**P4.1 — `DeleteFolderResult` Serialize**
- Added `Serialize, Deserialize` derives + `#[serde(rename_all = "camelCase")]`
- `moved_profile_count` → `"movedProfileCount"` in JSON

**P4.2 — `load_profiles_with_folders` command**
- Returns `ProfilesEnvelope` (full folder + profile tree)
- Lazy-load: if state empty, triggers `load_profiles_from_disk` + populates AppState
- If state already loaded, returns in-memory clone

**P4.4 — 7 folder CRUD Tauri commands**
All follow the same pattern:
1. Lock `state.profiles` + `state.folders`
2. Build `ProfilesEnvelope` from current state
3. Clone snapshot for rollback
4. Call pure `impl ProfilesEnvelope` method
5. On success: `save_profiles_envelope` (persist)
6. On persist failure: restore state from snapshot, propagate error
7. Write updated envelope back to AppState
8. Return result

Commands: `create_folder`, `rename_folder`, `delete_folder`, `reorder_folders`, `move_profile_to_folder`, `reorder_profiles_in_folder`, `set_folder_expanded`

**P4.6 — Shim removal**
Refactored all 4 callers:
- `save_profile`: now builds `ProfilesEnvelope{folders, profiles}` + calls `save_profiles_envelope`; also assigns `folder_id = system_folder.id` for new profiles with no folder
- `delete_profile`: same envelope pattern; acquires folders lock alongside profiles lock
- `reorder_profiles`: same envelope pattern
- `import_profiles`: acquires folders lock inside the `imported > 0` block; builds envelope; persists

`save_profiles_to_disk` function kept (it has its own passing tests that validate its behavior). It is no longer called from any command.

### Test Helper Added
`pub fn make_system_folder_for_test() -> Folder` in `profile.rs` (cfg(test) only) — allows command-layer tests to build envelopes without depending on private `make_system_folder()`.

### Files Changed (Batch 4)

| File | Action | Lines Added | What |
|------|--------|-------------|------|
| `src-tauri/src/profile.rs` | Modified | ~85 | `DeleteFolderResult` Serialize derive, 3 integration tests, `make_system_folder_for_test` test helper |
| `src-tauri/src/commands/profile.rs` | Modified | ~280 | `load_profiles_with_folders` + 7 folder CRUD commands + 3 command smoke tests; 4 shim callers refactored |
| `src-tauri/src/lib.rs` | Modified | ~10 | 8 new commands registered in invoke_handler |
| `openspec/changes/profile-folder-grouping/tasks.md` | Modified | — | P4.1–P4.8, P5.1–P5.5 marked [x] |
| `openspec/changes/profile-folder-grouping/apply-progress.md` | Modified | — | Batch 4 section added |

---

### Tests Count (Batch 4)

| Metric | Before | After | Delta |
|--------|--------|-------|-------|
| Total tests | 116 | 122 | +6 |
| Passing | 115 | 121 | +6 |
| Failing | 1 | 1 | 0 |
| Ignored | 0 | 0 | 0 |

New tests added (Batch 4):
1. `profile::tests::integration_full_round_trip_create_move_delete_folder`
2. `profile::tests::integration_persisted_json_is_envelope_format`
3. `profile::tests::integration_delete_folder_result_serializes`
4. `commands::profile::tests::command_pattern_create_folder_produces_serializable_folder`
5. `commands::profile::tests::command_pattern_delete_folder_result_camel_case`
6. `commands::profile::tests::command_pattern_profiles_envelope_serializes`

---

### Clippy Status: CLEAN
```
cargo clippy -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.54s
```

Zero warnings. Zero errors. ✅

---

### Discoveries / Risks (Batch 4)

1. **Lock acquisition order in folder commands**: Each folder command acquires `state.profiles` THEN `state.folders` in that order. This order must be consistent across ALL commands to prevent deadlocks. Currently consistent — but Phase 6 frontend store actions that call multiple backend commands in sequence must not hold cross-command locks.

2. **`import_profiles` needs folders lock inside the `imported > 0` guard**: The original import flow dropped the profiles lock before persisting. With the envelope pattern, we need to re-acquire `state.folders` inside the `imported > 0` block (after the import loop). This is safe since the import loop operates on `profiles` guard only, and we drop it before vault operations anyway.

3. **`save_profile` new-profile path**: Added auto-assignment of `folder_id = system_folder.id` for new profiles when no `folder_id` is specified. This ensures the post-load invariant ("every profile has a folder_id") is maintained even for profiles created through the Tauri command surface.

---

## Cumulative Checklist
- P0.1 ✅, P0.2 ✅, P0.3 ✅
- P1.1 ✅, P1.2 ✅, P1.3 ✅, P1.4 ✅, P1.5 ✅, P1.6 ✅, P1.7 ✅, P1.8 ✅, P1.9 ✅
- P2.1 ✅, P2.2 ✅, P2.3 ✅, P2.4 ✅, P2.5 ✅, P2.6 ✅, P2.7 ✅, P2.8 ✅
- P2.9 ✅, P2.10 ✅, P2.11 ✅, P2.12 ✅, P2.13 ✅, P2.14 ✅, P2.15 ✅
- P3.1 ✅, P3.2 ✅, P3.3 ✅, P3.4 ✅, P3.5 ✅
- P3.6 ✅, P3.7 ✅, P3.8 ✅, P3.9 ✅, P3.10 ✅
- P3.11 ✅, P3.12 ✅, P3.13 ✅, P3.14 ✅
- P3.15 ✅, P3.16 ✅, P3.17 ✅
- P3.18 ✅, P3.19 ✅, P3.20 ✅, P3.21 ✅
- P3.22 ✅, P3.23 ✅, P3.24 ✅, P3.25 ✅
- P3.26 ✅, P3.27 ✅, P3.28 ✅, P3.29 ✅ (REFACTOR)
- P4.1 ✅, P4.2 ✅, P4.3 ✅, P4.4 ✅, P4.5 ✅, P4.6 ✅, P4.7 ✅, P4.8 ✅
- P5.1 ✅, P5.2 ✅, P5.3 ✅, P5.4 ✅, P5.5 ✅
- P6.1 ✅, P6.2 ✅, P6.3 ✅, P6.4 ✅, P6.5 ✅, P6.6 ✅, P6.7 ✅, P6.8 ✅
- P6.9 ✅, P6.10 ✅, P6.11 ✅, P6.12 ✅, P6.13 ✅, P6.14 ✅, P6.15 ✅, P6.16 ✅

---

## Batch 5 — Phase 6 Zustand Store + Types

### Status: COMPLETE
- TSC errors before: 0
- TSC errors after: 0
- Delta: 0

---

### P6.0 — Patterns read from existing code

| File | Pattern found |
|------|--------------|
| `src/lib/types.ts` | Types use camelCase (matching Rust `#[serde(rename_all = "camelCase")]`). All types are `interface`, no `type` aliases for structs. |
| `src/lib/tauri.ts` | `tauriInvoke<T>(cmd, args)` is the only entry point. Named exported async functions wrapping `tauriInvoke`. `AppError` class holds `command` + `message`. |
| `src/stores/profileStore.ts` | `create<ProfileStoreState>((set) => ...)` — Zustand standard (no middleware: no persist, no devtools). `set` only in outer fn. **Added `get`** for `loadAll()` calls inside actions. |
| Callers of `loadProfiles` | Only in `Sidebar.tsx` (line 379 — `void loadProfiles()`). Store-internal uses replaced with `get().loadAll()`. App.tsx does NOT call `loadProfiles` directly. |

---

### P6.1 — Types (`src/lib/types.ts`)

Added:
- `Folder` interface with: `id`, `name`, `displayOrder`, `isSystem`, `isExpanded`, `createdAt`, `updatedAt`
- `ProfilesEnvelope` interface: `{ folders: Folder[]; profiles: ConnectionProfile[] }`
- `DeleteFolderResult` interface: `{ movedProfileCount: number }`
- `folderId?: string` on `ConnectionProfile` — optional for backward compat during transition; backend guarantees non-null after migration

---

### P6.2 — Constant (`src/lib/folders.ts` — NEW FILE)

Created with:
- `SYSTEM_FOLDER_MARKER = "__system__"` — matches Rust's `SYSTEM_FOLDER_NAME`
- `isSystemFolder(folder)` — checks `folder.isSystem` (backend-authoritative)
- `displayFolderName(folder, t)` — returns `t("sidebar.folders.ungroupedName")` for system folder, `folder.name` for others

---

### P6.3 — Tauri wrappers (`src/lib/tauri.ts`)

Added imports of `Folder`, `ProfilesEnvelope`, `DeleteFolderResult` from `./types`.

Added 8 typed wrapper functions:
| Function | Rust command | Args |
|----------|-------------|------|
| `loadProfilesWithFolders()` | `load_profiles_with_folders` | none |
| `createFolder(name)` | `create_folder` | `{ name }` |
| `renameFolder(folderId, newName)` | `rename_folder` | `{ folderId, newName }` |
| `deleteFolder(folderId)` | `delete_folder` | `{ folderId }` |
| `reorderFolders(orderedIds)` | `reorder_folders` | `{ orderedIds }` |
| `moveProfileToFolder(profileId, targetFolderId, newOrder)` | `move_profile_to_folder` | `{ profileId, targetFolderId, newOrder }` |
| `reorderProfilesInFolder(folderId, orderedProfileIds)` | `reorder_profiles_in_folder` | `{ folderId, orderedProfileIds }` |
| `setFolderExpanded(folderId, expanded)` | `set_folder_expanded` | `{ folderId, expanded }` |

Arg names verified against Rust `#[tauri::command]` fn signatures in `src-tauri/src/commands/profile.rs`.

---

### P6.4 — Store state shape diff

Added to `ProfileStoreState`:
```typescript
// New state fields:
folders: Folder[];
expandedFolderIds: Set<string>;

// New actions:
loadAll: () => Promise<void>;
createFolder: (name: string) => Promise<Folder>;
renameFolder: (folderId: string, newName: string) => Promise<void>;
deleteFolder: (folderId: string) => Promise<DeleteFolderResult>;
reorderFolders: (orderedIds: string[]) => Promise<void>;
moveProfileToFolder: (profileId: string, targetFolderId: string, newOrder: number) => Promise<void>;
reorderProfilesInFolder: (folderId: string, orderedProfileIds: string[]) => Promise<void>;
toggleFolderExpanded: (folderId: string) => Promise<void>;
```

`loadProfiles` kept as `@deprecated` compat alias delegating to `loadAll()`.

---

### P6.5 — Optimistic / Pessimistic strategy applied

| Action | Strategy | Rollback |
|--------|---------|---------|
| `createFolder` | Pessimistic | N/A — call backend first |
| `renameFolder` | Pessimistic | N/A |
| `deleteFolder` | Pessimistic | N/A |
| `reorderFolders` | Optimistic | snapshot → restore on throw |
| `moveProfileToFolder` | Pessimistic (refetch) | Reload from backend |
| `reorderProfilesInFolder` | Optimistic | snapshot → restore on throw |
| `toggleFolderExpanded` | Optimistic (no rollback) | — expand state is UI-only |

---

### P6.6 — Selectors added (exported plain functions)

```typescript
export function profilesByFolder(state): Map<string, ConnectionProfile[]>
export function sortedFolders(state): Folder[]
export function systemFolder(state): Folder | undefined
```

---

### P6.7 — Callers of loadProfiles updated

- `saveProfile` → now calls `get().loadAll()` instead of re-invoking `load_profiles`
- `deleteProfile` → same
- `importProfiles` → same
- `reorderProfiles` rollback → same
- `loadProfiles` action → delegates to `get().loadAll()`

Sidebar.tsx NOT touched (out of scope for Batch 5). It still calls `loadProfiles()` which now correctly delegates to `loadAll()`.

---

### TSC Gate (P6.8)

```
pnpm tsc --noEmit
exit code: 0 — zero errors
```

Before: 0 errors | After: 0 errors | Delta: 0 ✅

---

### Files Changed (Batch 5)

| File | Action | What |
|------|--------|------|
| `src/lib/types.ts` | Modified | Added `Folder`, `ProfilesEnvelope`, `DeleteFolderResult` interfaces; added `folderId?: string` to `ConnectionProfile` |
| `src/lib/tauri.ts` | Modified | Added imports for 3 new types; added 8 typed Tauri wrapper functions |
| `src/lib/folders.ts` | Created | `SYSTEM_FOLDER_MARKER`, `isSystemFolder`, `displayFolderName` |
| `src/stores/profileStore.ts` | Modified | Added `get` param; `folders` + `expandedFolderIds` state; `loadAll` + 7 folder actions; `loadProfiles` compat shim; updated internal callers; selectors |
| `openspec/changes/profile-folder-grouping/tasks.md` | Modified | P6.1–P6.16 marked [x] |

---

### Discoveries / Risks (Batch 5)

1. **Sidebar.tsx calls `loadProfiles`, not `loadAll`**: Resolved by making `loadProfiles` a compat shim that delegates to `loadAll`. Sidebar.tsx unchanged. Batch 6 should eventually update Sidebar.tsx to call `loadAll` directly and remove the `@deprecated` shim.

2. **`reorderProfilesInFolder` optimistic filter uses `folderId !== folderId`**: The "other profiles" filter relies on `p.folderId !== folderId` — if a profile somehow has `folderId: undefined` it will land in the "other" bucket and not be affected by the reorder. This is safe — such a profile wouldn't be in the optimistic reorder list either.

3. **`expandedFolderIds` is a `Set<string>` — Zustand shallow equality**: Zustand's default `shallow` comparison does NOT deep-compare Sets. Components subscribing to `expandedFolderIds` with a shallow selector will NOT re-render on toggle. Components must either subscribe to the full store slice or use `useShallow` from zustand/react/shallow. This should be documented for Batch 6 UI work.

4. **TSC baseline was 0 errors, not 6**: The batch instructions mentioned 6 pre-existing errors (@dnd-kit + Tauri plugin type decls). These appear to have been resolved before Batch 5. The actual baseline is 0 errors, and this batch keeps it at 0.

---

## Batch 6 — Phase 7 + Phase 8 Sidebar UI + i18n

### Status: COMPLETE
- TSC errors before: 0
- TSC errors after: 0
- Delta: 0

---

### Phase 8 — i18n Keys

**P8.1 — EN keys added to `src/lib/i18n/en.ts`**

25 new keys added under `sidebar.folders.*` namespace:
- `sidebar.folders.ungroupedName`, `newFolder`, `createFolder`, `createFolderPlaceholder`
- `sidebar.folders.create`, `cancel`, `rename`, `renameFolder`, `save`
- `sidebar.folders.delete`, `deleteFolder`
- `sidebar.folders.deleteConfirmTitle`, `deleteConfirmBody`, `deleteConfirmEmpty`
- `sidebar.folders.emptyHint`, `moveTo`, `moveToSubmenu`
- `sidebar.folders.systemProtected`, `duplicateName`, `invalidName`, `errorGeneric`
- `sidebar.folders.profileCount_one`, `profileCount_other` (two plain keys — no `{{count}}` interpolation, uses `{count}` which matches existing i18n system)
- `sidebar.folders.dragHandle`

**P8.2 — ES translations added to `src/lib/i18n/es.ts`**

Same 25 keys with Spanish (Rioplatense) values. Full parity with EN.

**P8.3 — TSC parity check**: PASSED — 0 errors after adding both EN and ES keys.

---

### Phase 7 — Sidebar UI Decisions

#### Architecture (P7.1)
- Single-file approach: all components in `Sidebar.tsx` (~1732 LOC)
- Components: `FloatingContextMenu`, `SortableProfileCard`, `FolderRow`, `CreateFolderDialog`, `RenameFolderDialog`, `DeleteFolderDialog`, main `Sidebar`
- DnD: Single `DndContext` wrapping a `SortableContext` for folders, with nested `SortableContext` per folder for profiles

#### DnD ID Prefixing (P7.2)
- Folder IDs: `folder:{uuid}`
- Profile IDs: `profile:{uuid}`
- `onDragEnd` splits on `:` to discriminate folder vs profile drag
- Cross-folder profile drag: silent no-op (no toast — cleaner UX)

#### `useShallow` for store subscriptions (P7.14)
- Actions extracted via `useShallow` selector on `useProfileStore`
- `sortedFolders`, `profilesByFolder`, `expandedFolderIds` as separate subscriptions
- `expandedFolderIds` subscribed directly — Batch 5 store already uses `new Set(...)` on every toggle, so reference equality works correctly
- Zustand v5.0.11 confirmed; `useShallow` from `zustand/react/shallow` available

#### Context Menu pattern (P7.4)
- Reused `sftp-context-menu` + `sftp-context-item` + `sftp-context-danger` CSS classes from existing FileContextMenu pattern
- Profile right-click → shows "Move to folder" submenu + Edit + Delete
- Folder ⋯ button → inline dropdown (position: absolute relative to button) with Rename + Delete (hidden for system folder)

#### Search behavior (P7.8)
- During search: all folders effectively expanded, DnD disabled, folders with 0 matching profiles hidden
- `filteredProfileIds` is a `Set<string> | null` — null means "no filter"
- visibleFolders computed: `folders.filter(f => folderProfiles[f.id].length > 0)` during search

#### System folder (P7.11)
- `displayFolderName(folder, t)` used everywhere — never reads `folder.name` for display
- `folder.isSystem` gates rename/delete ⋯ menu visibility
- System folder: no ⋯ menu rendered at all

#### App.tsx (P7.12)
- App.tsx does NOT call `loadProfiles` or `loadAll` directly — Sidebar handles loading on mount
- Sidebar now calls `loadAll()` directly (removed `@deprecated loadProfiles` call)
- `@deprecated loadProfiles` shim in store still present for any other callers

#### toggleFolderExpanded / new Set guarantee (P7.13/P7.14)
- Verified in `profileStore.ts`: `set((state) => { const next = new Set(state.expandedFolderIds); ... return { expandedFolderIds: next }; })`
- Always creates a NEW Set object → reference equality works → components re-render correctly

#### Deviations from plan
- No `role="treeitem"` on FolderRow main div (applied to the header `div` inside instead — avoids nesting issues)
- Keyboard nav (Arrow Left/Right) NOT implemented — noted as nice-to-have in spec, skipped per "don't block if hard" guidance
- `+ Carpeta` button placed in the same toolbar row as Import/Export/New — cleaner layout
- `useShallow` not used for `expandedFolderIds` subscription (direct subscription is fine since Set reference changes on every toggle)

---

### Files Changed (Batch 6)

| File | Action | LOC Delta | What |
|------|--------|-----------|------|
| `src/lib/i18n/en.ts` | Modified | +28 | 25 new `sidebar.folders.*` keys |
| `src/lib/i18n/es.ts` | Modified | +28 | Same 25 keys in Spanish |
| `src/components/layout/Sidebar.tsx` | Rewritten | +1732 (was 1077) | Full folder-grouped sidebar with DnD, context menus, 3 CRUD dialogs |
| `openspec/changes/profile-folder-grouping/tasks.md` | Modified | — | P7.1–P7.15, P8.1–P8.3 marked [x] |

---

### TSC Gate

```
pnpm tsc --noEmit
(no output — 0 errors)
```

Before: 0 errors | After: 0 errors ✅

---

### Open Issues / Known Limitations

1. **Keyboard navigation (Arrow Left/Right)** not implemented — P7.9 nice-to-have, spec says "don't block if hard".
2. **Cross-folder profile drag** silently no-ops — user must right-click → "Move to folder". This is intentional per design (MVP scope).
3. **`sidebar-folder-header` / `sidebar-folder-group` / `sidebar-folder-content`** CSS classes are new and need to be styled in the project's CSS. The components use existing DaisyUI/Tailwind-compatible class names where possible but the folder-specific classes will show unstyled until CSS is added. Manual QA should verify visual appearance.
4. **`sidebar-folder-chevron`** uses inline styles for the animation — this is sufficient but could be moved to CSS class if the project has a design system preference.
5. **Profile count badge** uses two separate keys (`profileCount_one` / `profileCount_other`) since the i18n system uses `{param}` interpolation (not `{{param}}` or ICU plural). Works correctly in both EN and ES.

---

### Cumulative Checklist Update

- P7.1 ✅, P7.2 ✅, P7.3 ✅, P7.4 ✅, P7.5 ✅, P7.6 ✅, P7.7 ✅, P7.8 ✅
- P7.9 ✅ (partial — keyboard nav skipped), P7.10 ✅, P7.11 ✅, P7.12 ✅, P7.13 ✅, P7.14 ✅, P7.15 ✅
- P8.1 ✅, P8.2 ✅, P8.3 ✅
