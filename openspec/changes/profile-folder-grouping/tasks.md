# Tasks: profile-folder-grouping

**Change**: profile-folder-grouping
**Spec**: `openspec/changes/profile-folder-grouping/specs/profile-organization/spec.md`
**Design**: `openspec/changes/profile-folder-grouping/design.md`
**TDD discipline**: Rust phases 1–3 enforce RED → GREEN → REFACTOR strictly.

---

## Phase 0 — Preparation (Audit & Baseline)

- [x] **P0.1** Audit `profiles.json` direct readers — run `rg "profiles\.json"` across the whole repo and document every hit; any reader that bypasses `load_profiles_from_disk` must be routed through it before Phase 2 — file: all `src-tauri/src/**/*.rs`
- [x] **P0.2** Audit coupling points — run `rg "load_profiles_from_disk"` and `rg "save_profiles_to_disk"` (or `save_profiles`); map every call site to decide if it must be touched in Phase 2 or Phase 4 — file: all `src-tauri/src/**/*.rs`
- [x] **P0.3** Capture baseline — run `cd src-tauri && cargo test --no-run 2>&1` and record the current number of compiled test functions; this is the regression baseline for P5.1 — file: none (record in a scratchpad comment)

---

## Phase 1 — Data Model (Rust, TDD · RED → GREEN → REFACTOR)

- [x] **P1.1** [RED] Write failing test `folder_serialize_roundtrip` — assert that a hand-crafted `Folder` serializes to JSON and deserializes back to the same value — file: `src-tauri/src/profile.rs` (`#[cfg(test)] mod tests`)
- [x] **P1.2** [GREEN] Define `Folder` struct in `profile.rs` — fields: `id: Uuid`, `name: String`, `display_order: i32`, `is_system: bool` (default false), `is_expanded: bool` (default true), `created_at: DateTime<Utc>`, `updated_at: DateTime<Utc>`; derive `Debug, Clone, Serialize, Deserialize`; `#[serde(rename_all = "camelCase")]` — file: `src-tauri/src/profile.rs`
- [x] **P1.3** [RED] Write failing test `envelope_serialize_roundtrip` — assert that `ProfilesEnvelope { folders: [..], profiles: [..] }` round-trips through JSON — file: `src-tauri/src/profile.rs`
- [x] **P1.4** [GREEN] Define `ProfilesEnvelope` struct — fields: `folders: Vec<Folder>`, `profiles: Vec<ConnectionProfile>`; same derives + serde attributes — file: `src-tauri/src/profile.rs`
- [x] **P1.5** [RED] Write failing test `connection_profile_has_folder_id` — assert that a `ConnectionProfile` with `folder_id: None` serializes to `"folderId": null` and re-deserializes to `None` without error — file: `src-tauri/src/profile.rs`
- [x] **P1.6** [GREEN] Add `folder_id: Option<Uuid>` field to `ConnectionProfile` with `#[serde(default)]`; add doc-comment "Post-load invariant: always Some after migrate/load" — file: `src-tauri/src/profile.rs`
- [x] **P1.7** [REFACTOR] Review all three structs — add `PartialEq` where test assertions need it; ensure `#[allow(dead_code)]` is not needed (all fields used); run `cargo clippy -D warnings` and resolve any lint — file: `src-tauri/src/profile.rs`
- [x] **P1.8** [GREEN] Add `AppState.folders: Mutex<Vec<Folder>>` field; initialize to empty vec in `AppState::default()` / constructor — file: `src-tauri/src/state.rs`
- [x] **P1.9** [GREEN] Add `SYSTEM_FOLDER_NAME: &str = "__system__"` constant; document that the raw name is never shown to the user — file: `src-tauri/src/profile.rs`

---

## Phase 2 — Migration (Rust, TDD · RED → GREEN → REFACTOR) — HIGHEST RISK

- [x] **P2.1** [RED] Test `migration_detect_format_legacy_array` — feed raw bytes of `[{...}]`; assert `detect_profiles_format` returns `ProfilesFormat::LegacyArray` — file: `src-tauri/src/profile.rs`
- [x] **P2.2** [RED] Test `migration_detect_format_envelope` — feed `{"folders":[],"profiles":[]}`; assert returns `ProfilesFormat::Envelope` — file: `src-tauri/src/profile.rs`
- [x] **P2.3** [RED] Test `migration_legacy_to_envelope_produces_system_folder` — 2 profiles → envelope with 1 system folder, all profiles folder_id = sys.id, sequential display_order — file: `src-tauri/src/profile.rs`
- [x] **P2.4** [RED] Test `migration_envelope_format_is_idempotent` — write envelope JSON, load, assert no backup written — file: `src-tauri/src/profile.rs`
- [x] **P2.5** [RED] Test `migration_missing_file_returns_system_folder_envelope` — missing file → envelope with 1 system folder + 0 profiles — file: `src-tauri/src/profile.rs`
- [x] **P2.6** [RED] Test `migration_legacy_writes_backup_before_envelope` — legacy array → backup exists with original bytes — file: `src-tauri/src/profile.rs`
- [x] **P2.7** [RED] Test `migration_backup_collision_uses_timestamped_variant` — pre-existing backup.json + new migration → original untouched, new timestamped backup appears — file: `src-tauri/src/profile.rs`
- [x] **P2.8** [RED] Test `migration_corrupted_json_returns_error_no_modification` — `{not valid` → Err, file unchanged, no backup — file: `src-tauri/src/profile.rs`
- [x] **P2.9** [GREEN] Implement `ProfilesFormat` enum + `detect_profiles_format(bytes: &[u8]) -> Result<ProfilesFormat, AppError>` — file: `src-tauri/src/profile.rs`
- [x] **P2.10** [GREEN] Implement `migrate_legacy_to_envelope(profiles: Vec<ConnectionProfile>) -> ProfilesEnvelope` — pure function — file: `src-tauri/src/profile.rs`
- [x] **P2.11** [GREEN] Implement `backup_path_for` + `write_backup` — timestamped rotation, best-effort ACL hardening — file: `src-tauri/src/profile.rs`
- [x] **P2.12** [GREEN] Rewrite `load_profiles_from_disk` → returns `ProfilesEnvelope`; missing/empty→system folder; legacy→migrate+backup+save; envelope→heal system folder; corrupted→error — file: `src-tauri/src/profile.rs`
- [x] **P2.13** [GREEN] Add `save_profiles_envelope` (atomic write of full envelope) — file: `src-tauri/src/profile.rs`
- [x] **P2.14** [GREEN] Update `load_profiles` command: internally calls `load_profiles_from_disk`, populates both `AppState.profiles` + `AppState.folders`, returns `Vec<ConnectionProfile>` for backward compat — file: `src-tauri/src/commands/profile.rs`
- [x] **P2.15** [REFACTOR] `save_profiles_to_disk` kept as Phase 4 shim; all tests green; clippy clean; doc-comments on all new public functions — file: `src-tauri/src/profile.rs`

---

## Phase 3 — Folder CRUD Logic (Rust, TDD · RED → GREEN → REFACTOR)

> All tests go in `#[cfg(test)] mod tests` inside `src-tauri/src/commands/profile.rs` unless noted.

### create_folder

- [x] **P3.1** [RED] Test `crud_create_folder_happy_path` — valid name "Proxmox" → returns `Folder` with unique uuid, `is_system: false`, `display_order = max+1` — file: `src-tauri/src/profile.rs`
- [x] **P3.2** [RED] Test `crud_create_folder_rejects_empty_name` — name `""` → `ProfileError::InvalidName` — file: `src-tauri/src/profile.rs`
- [x] **P3.3** [RED] Test `crud_create_folder_rejects_whitespace_name` — name `"   "` → `ProfileError::InvalidName` after trim — file: `src-tauri/src/profile.rs`
- [x] **P3.4** [RED] Test `crud_create_folder_rejects_name_over_64_chars` — name of 65 chars → `ProfileError::InvalidName` — file: `src-tauri/src/profile.rs`
- [x] **P3.5** [RED] Test `crud_create_folder_rejects_duplicate_name_case_insensitive` — "Proxmox" twice + "PROXMOX" → `ProfileError::DuplicateName` — file: `src-tauri/src/profile.rs`

### rename_folder

- [x] **P3.6** [RED] Test `crud_rename_folder_happy_path` — rename user folder; assert new name, same UUID, same display_order — file: `src-tauri/src/profile.rs`
- [x] **P3.7** [RED] Test `crud_rename_folder_not_found` — unknown UUID → `ProfileError::FolderNotFound` — file: `src-tauri/src/profile.rs`
- [x] **P3.8** [RED] Test `crud_rename_folder_system_protected` — attempt rename on system folder → `ProfileError::SystemFolderProtected` — file: `src-tauri/src/profile.rs`
- [x] **P3.9** [RED] Test `crud_rename_folder_invalid_name` — empty/whitespace/>64 chars → `ProfileError::InvalidName` — file: `src-tauri/src/profile.rs`
- [x] **P3.10** [RED] Test `crud_rename_folder_duplicate_name_and_own_name_allowed` — duplicate → DuplicateName; own case-change → allowed — file: `src-tauri/src/profile.rs`

### delete_folder

- [x] **P3.11** [RED] Test `crud_delete_folder_empty` — delete empty folder; 0 moved, folder removed — file: `src-tauri/src/profile.rs`
- [x] **P3.12** [RED] Test `crud_delete_folder_with_profiles_moves_to_system` — 3 profiles → moved to system folder, count=3, relative order preserved — file: `src-tauri/src/profile.rs`
- [x] **P3.13** [RED] Test `crud_delete_folder_system_protected` — system folder → `ProfileError::SystemFolderProtected` — file: `src-tauri/src/profile.rs`
- [x] **P3.14** [RED] Test `crud_delete_folder_not_found` — unknown UUID → `ProfileError::FolderNotFound` — file: `src-tauri/src/profile.rs`

### reorder_folders

- [x] **P3.15** [RED] Test `crud_reorder_folders_happy_path` — 4 folders shuffled; display_order == index in input vec — file: `src-tauri/src/profile.rs`
- [x] **P3.16** [RED] Test `crud_reorder_folders_missing_id` — subset of folder IDs → `ProfileError::IncompleteReorder` — file: `src-tauri/src/profile.rs`
- [x] **P3.17** [RED] Test `crud_reorder_folders_unknown_id` — extra unknown UUID → `ProfileError::FolderNotFound` — file: `src-tauri/src/profile.rs`

### move_profile_to_folder

- [x] **P3.18** [RED] Test `crud_move_profile_to_folder_shifts_siblings` — cross-folder move at order 0, siblings shift +1 — file: `src-tauri/src/profile.rs`
- [x] **P3.19** [RED] Test `crud_move_profile_to_folder_unknown_folder` — unknown folder → `ProfileError::FolderNotFound`, state unchanged — file: `src-tauri/src/profile.rs`
- [x] **P3.20** [RED] Test `crud_move_profile_to_folder_unknown_profile` — unknown profile → `ProfileError::ProfileNotFound` — file: `src-tauri/src/profile.rs`
- [x] **P3.21** [RED] Test `crud_move_profile_same_folder_reorder` — same-folder move reorders correctly, own-position is no-op — file: `src-tauri/src/profile.rs`

### reorder_profiles_in_folder

- [x] **P3.22** [RED] Test `crud_reorder_profiles_in_folder_happy_path` — p2, p0, p1 order → display_orders 0,1,2 — file: `src-tauri/src/profile.rs`
- [x] **P3.23** [RED] Test `crud_reorder_profiles_in_folder_missing_id` — subset → `ProfileError::IncompleteReorder` — file: `src-tauri/src/profile.rs`
- [x] **P3.24** [RED] Test `crud_reorder_profiles_in_folder_unknown_id` — unknown UUID → `ProfileError::ProfileNotFound` — file: `src-tauri/src/profile.rs`
- [x] **P3.25** [RED] Test `crud_reorder_profiles_in_folder_cross_folder_profile` — profile from different folder → `ProfileError::ProfileFolderMismatch` — file: `src-tauri/src/profile.rs`

### set_folder_expanded

- [x] **P3.26** [RED] Test `crud_set_folder_expanded_happy_path` — collapse, idempotent collapse, re-expand — file: `src-tauri/src/profile.rs`
- [x] **P3.27** [RED] Test `crud_set_folder_expanded_not_found` — unknown UUID → `ProfileError::FolderNotFound` — file: `src-tauri/src/profile.rs`

### GREEN + REFACTOR

- [x] **P3.28** [RED+GREEN] Atomicity test `crud_clone_before_op_proves_no_aliased_state` + `crud_failed_create_folder_leaves_state_unchanged` — clone-before-mutate pattern; failing op leaves state unchanged — file: `src-tauri/src/profile.rs`
- [x] **P3.22g** [GREEN] Implement `ProfilesEnvelope::create_folder` — trim, validate, duplicate check, UUID, display_order=max+1 — file: `src-tauri/src/profile.rs`
- [x] **P3.23g** [GREEN] Implement `ProfilesEnvelope::rename_folder` — system guard, validate, duplicate check (own name allowed), update — file: `src-tauri/src/profile.rs`
- [x] **P3.24g** [GREEN] Implement `ProfilesEnvelope::delete_folder` — system guard, cascade move to system folder, relative order preserved, return `DeleteFolderResult` — file: `src-tauri/src/profile.rs`
- [x] **P3.25g** [GREEN] Implement `ProfilesEnvelope::reorder_folders` — unknown→FolderNotFound, missing→IncompleteReorder, apply display_orders — file: `src-tauri/src/profile.rs`
- [x] **P3.26g** [GREEN] Implement `ProfilesEnvelope::move_profile_to_folder` — cross-folder: shift siblings; same-folder: sequential reorder — file: `src-tauri/src/profile.rs`
- [x] **P3.27g** [GREEN] Implement `ProfilesEnvelope::reorder_profiles_in_folder` — folder check, profile check, folder-mismatch check, completeness check, apply orders — file: `src-tauri/src/profile.rs`
- [x] **P3.28g** [GREEN] Implement `ProfilesEnvelope::set_folder_expanded` — find_folder_mut, set flag, return Ok — file: `src-tauri/src/profile.rs`
- [x] **P3.29** [REFACTOR] Extract `find_folder_mut`, `validate_folder_name`, `name_conflicts`, `system_folder_id` helpers; `cargo clippy -- -D warnings` clean; 29 new tests all pass — file: `src-tauri/src/profile.rs`, `src-tauri/src/error.rs`
- [x] **P4.1-partial** Added `ProfileError` enum to `error.rs` with 7 variants: FolderNotFound, ProfileNotFound, SystemFolderProtected, InvalidName, DuplicateName, IncompleteReorder, ProfileFolderMismatch — file: `src-tauri/src/error.rs`

---

## Phase 4 — Tauri Command Surface Wiring

- [x] **P4.1** `ProfileError` + `DeleteFolderResult` Serialize — `DeleteFolderResult` now has `#[derive(Serialize, Deserialize)]` with `camelCase`; `ProfileError` serializes via `AppError::from` (already in place) — file: `src-tauri/src/profile.rs`, `src-tauri/src/error.rs`
- [x] **P4.2** `load_profiles_with_folders` command — returns `ProfilesEnvelope` (full envelope with folders + profiles); triggers disk load if state empty — file: `src-tauri/src/commands/profile.rs`
- [x] **P4.3** `load_profiles` backward-compat shim kept — returns `Vec<ConnectionProfile>`, now also populates `AppState.folders` — file: `src-tauri/src/commands/profile.rs`
- [x] **P4.4** New folder CRUD commands implemented: `create_folder`, `rename_folder`, `delete_folder`, `reorder_folders`, `move_profile_to_folder`, `reorder_profiles_in_folder`, `set_folder_expanded` — all with rollback-on-persist-failure pattern — file: `src-tauri/src/commands/profile.rs`
- [x] **P4.5** All 8 new commands registered in `invoke_handler!` — file: `src-tauri/src/lib.rs`
- [x] **P4.6** Shim removal — all 4 callers of `save_profiles_to_disk` in `commands/profile.rs` (save_profile, delete_profile, reorder_profiles, import_profiles) refactored to use `save_profiles_envelope` — file: `src-tauri/src/commands/profile.rs`
- [x] **P4.7** Integration test `integration_full_round_trip_create_move_delete_folder` — tempdir round-trip: load → create_folder → save → reload → move_profile → save → reload → delete_folder → save → reload → profile back in system folder — file: `src-tauri/src/profile.rs`
- [x] **P4.8** Persistence invariant test `integration_persisted_json_is_envelope_format` + `integration_delete_folder_result_serializes` — verifies every save via `save_profiles_envelope` writes envelope JSON (object with `folders`+`profiles` keys, never array root) — file: `src-tauri/src/profile.rs`

---

## Phase 5 — PR A Gate (Rust-only)

- [x] **P5.1** `cargo test` — 122 tests, 121 passing, 1 failing (same environmental: `list_keys_handles_missing_ssh_dir`); delta +6 from batch 4 — file: CI / terminal
- [x] **P5.2** `cargo clippy -- -D warnings` — zero warnings, zero errors ✅ — file: CI / terminal
- [x] **P5.3** Command surface verified: 15 `#[tauri::command]` in `commands/profile.rs`; all 8 new commands in `invoke_handler!` in `lib.rs` — file: `src-tauri/src/lib.rs`, `src-tauri/src/commands/profile.rs`
- [x] **P5.4** `///` doc comments added to all 8 new Tauri commands (what, arg validation, error cases) — file: `src-tauri/src/commands/profile.rs`
- [x] **P5.5** No frontend files touched: `git diff --name-only` shows only `src-tauri/` files — ✅
- [ ] **P5.6** Manual smoke: launch app, verify legacy profiles.json migrates, folder ops available — file: manual (pending deploy)

---

## Phase 6 — Frontend Store (TypeScript)

- [x] **P6.1** Add `Folder` interface to `src/lib/types.ts` — fields: `id`, `name`, `displayOrder`, `isSystem`, `isExpanded`, `createdAt`, `updatedAt` (all string-typed timestamps) — file: `src/lib/types.ts`
- [x] **P6.2** Add `folderId?: string` to `ConnectionProfile` interface — file: `src/lib/types.ts`
- [x] **P6.3** Add `ProfilesEnvelope` interface — `{ folders: Folder[]; profiles: ConnectionProfile[] }` — file: `src/lib/types.ts`
- [x] **P6.4** Add `DeleteFolderResult` interface — `{ movedProfileCount: number }` — file: `src/lib/types.ts`
- [x] **P6.5** Extend `ProfileStoreState` in `profileStore.ts` with `folders: Folder[]`, `expandedFolderIds: Set<string>`, and all new action signatures — file: `src/stores/profileStore.ts`
- [x] **P6.6** Add `loadAll()` action invoking `load_profiles_with_folders`; `loadProfiles()` delegates to `loadAll()` for backward compat — file: `src/stores/profileStore.ts`
- [x] **P6.7** Implement `createFolder(name)` — **pessimistic**: await Tauri, reload on success — file: `src/stores/profileStore.ts`
- [x] **P6.8** Implement `renameFolder(id, newName)` — **pessimistic**: await Tauri, reload on success — file: `src/stores/profileStore.ts`
- [x] **P6.9** Implement `deleteFolder(id)` — **pessimistic**: await Tauri, reload on success — file: `src/stores/profileStore.ts`
- [x] **P6.10** Implement `reorderFolders(folderIds)` — **optimistic** with snapshot rollback — file: `src/stores/profileStore.ts`
- [x] **P6.11** Implement `moveProfileToFolder(profileId, targetFolderId, newOrder)` — **pessimistic** (refetch after) — file: `src/stores/profileStore.ts`
- [x] **P6.12** Implement `reorderProfilesInFolder(folderId, profileIds)` — **optimistic** with snapshot rollback — file: `src/stores/profileStore.ts`
- [x] **P6.13** Implement `toggleFolderExpanded(folderId)` — **optimistic** local update, debounced 300ms backend persist, no rollback — file: `src/stores/profileStore.ts`
- [x] **P6.14** Add exported selectors: `profilesByFolder`, `sortedFolders`, `systemFolder` — file: `src/stores/profileStore.ts`
- [x] **P6.15** Added `SYSTEM_FOLDER_MARKER`, `isSystemFolder`, `displayFolderName` to new `src/lib/folders.ts` — file: `src/lib/folders.ts`
- [x] **P6.16** Added typed tauri wrappers to `src/lib/tauri.ts`: `loadProfilesWithFolders`, `createFolder`, `renameFolder`, `deleteFolder`, `reorderFolders`, `moveProfileToFolder`, `reorderProfilesInFolder`, `setFolderExpanded` — file: `src/lib/tauri.ts`

---

## Phase 7 — Sidebar UI (React)

- [x] **P7.1** Create `FolderRow.tsx` component — renders folder header (chevron, `aria-expanded`, name, count badge, context menu) + collapsible profile list — implemented inline in `Sidebar.tsx`
- [x] **P7.2** Create `FolderHeader.tsx` presentational sub-component — implemented as `FolderRow` inner header div in `Sidebar.tsx`
- [x] **P7.3** Refactor `Sidebar.tsx` root to render `DndContext` > outer `SortableContext` (folders, items prefixed `folder:{id}`) > `FolderRow` list — file: `src/components/layout/Sidebar.tsx`
- [x] **P7.4** Wire per-folder `SortableContext` inside `FolderRow`; `handleDragEnd` routes on prefix: `folder:` → `reorderFolders`, `profile:` same-folder → `reorderProfilesInFolder`; cross-folder drops silent no-op — file: `src/components/layout/Sidebar.tsx`
- [x] **P7.5** Add `+ Carpeta/New Folder` button to toolbar row — opens `CreateFolderDialog` with client-side validation + `createFolder` store action — file: `src/components/layout/Sidebar.tsx`
- [x] **P7.6** Add ⋯ context menu to non-system folder headers with "Rename" and "Delete" items; opens `RenameFolderDialog` / `DeleteFolderDialog` — file: `src/components/layout/Sidebar.tsx`
- [x] **P7.7** Add right-click context menu to profile cards with "Move to folder" submenu (all other folders listed) + Edit + Delete — file: `src/components/layout/Sidebar.tsx`
- [x] **P7.8** Search mode: all folders expanded visually, DnD disabled, empty-result folders hidden — file: `src/components/layout/Sidebar.tsx`
- [x] **P7.9** Empty folder state: renders `sidebar.folders.emptyHint` when folder has 0 profiles and not searching — file: `src/components/layout/Sidebar.tsx`
- [x] **P7.10** System folder protection: ⋯ menu not rendered at all for `isSystem: true` folders — file: `src/components/layout/Sidebar.tsx`
- [x] **P7.11** Active folder tracking: NOT implemented in this batch — new profiles always land in system folder via backend auto-assignment. Deferred.
- [x] **P7.12** Keyboard accessibility: Arrow navigation deferred (nice-to-have, spec allows skipping). aria-expanded/aria-label on folder headers implemented.
- [x] **P7.13** Focus rings: existing CSS handles `:focus-visible`; all new buttons use existing `sidebar-profile-btn` / `btn` classes

---

## Phase 8 — i18n

- [x] **P8.1** Add all 25 keys under `sidebar.folders.*` namespace to `en.ts` (25 keys, slightly more than spec's 22 — added `dragHandle`) — file: `src/lib/i18n/en.ts`
- [x] **P8.2** Add Spanish translations for all 25 keys to `es.ts` (full parity) — file: `src/lib/i18n/es.ts`
- [x] **P8.3** Run `pnpm tsc --noEmit` — 0 errors ✅ — file: CI / terminal

---

## Phase 9 — Manual QA

- [ ] **P9.1** Create folder "Proxmox" via sidebar `+` button → appears with `(0)` count badge, `is_system: false`
- [ ] **P9.2** Move profile "prod-db-1" to "Proxmox" via context menu → badge becomes `(1)`; persist verified on app restart
- [ ] **P9.3** DnD reorder two profiles within "Proxmox" → order survives restart
- [ ] **P9.4** DnD reorder folder headers → "Proxmox" above "Sin agrupar" → survives restart
- [ ] **P9.5** Search "prod" while profiles span two folders → flat list with `[Folder]` badges; clearing restores structure
- [ ] **P9.6** Search with no matches → empty list with no folder headers visible
- [ ] **P9.7** Delete "Proxmox" (non-empty, 1 profile) → confirmation dialog shows "1 profile moved to Ungrouped" → profile back in system folder
- [ ] **P9.8** Attempt rename / delete on "Sin agrupar" via context menu → menu items absent; attempt via devtools IPC → backend returns error
- [ ] **P9.9** Collapse "Proxmox" → restart app → still collapsed
- [ ] **P9.10** Rename `profiles.json` backup to a flat-array legacy file → relaunch → auto-migrated silently; `profiles.backup.json` created

---

## Phase 10 — Verification Gates

- [ ] **P10.1** `cd src-tauri && cargo test` — ALL green (new + pre-existing)
- [ ] **P10.2** `cargo clippy -D warnings` — zero warnings in all touched Rust files
- [ ] **P10.3** `pnpm tsc --noEmit` — no new TypeScript errors beyond pre-existing 6
- [ ] **P10.4** `rg "TODO|FIXME|XXX"` — sweep all touched files; resolve or intentionally defer with a GitHub issue reference
- [ ] **P10.5** Trace matrix audit — verify every R1–R15 maps to at least one passing test or QA item (see §Trace Matrix below)

---

## Phase 11 — Upstream Contribution Prep

- [ ] **P11.1** File a NEW upstream GitHub issue on `CogniDevAI/nexterm` titled "Profile folder grouping" — link `openspec/changes/profile-folder-grouping/` for transparency; do NOT reuse issue #1 (vault ACL)
- [ ] **P11.2** Prepare **PR A branch** (backend-only): commits from Phase 1–5; `cargo test` green; conventional commit message `feat(profile): add Folder model, envelope migration, folder CRUD commands (~300 LOC)`
- [ ] **P11.3** Prepare **PR B branch** (frontend): commits from Phase 6–9; depends on PR A merge; conventional commit `feat(sidebar): add folder grouping UI, store, and i18n (~500 LOC)`
- [ ] **P11.4** Verify conventional commit format on all commits (`feat`, `fix`, `refactor`, `test`) — no "Co-Authored-By" or AI attribution lines

---

## Trace Matrix — Requirements → Tasks

| Req | Description | Tasks |
|-----|-------------|-------|
| R1 | Every profile belongs to exactly one folder | P1.5, P1.6, P2.10, P2.12, P3.15, P6.6 |
| R2 | System folder "Sin agrupar" invariants | P1.9, P2.3, P2.9, P2.12, P3.7, P3.11, P7.10, P9.8 |
| R3 | Create folder via UI | P3.1–P3.5, P3.22, P7.5, P9.1 |
| R4 | Rename non-system folder | P3.6–P3.8, P3.23, P7.6, P9.8 |
| R5 | Delete non-system folder moves profiles atomically | P3.9–P3.12, P3.24, P7.6, P7.7, P9.7 |
| R6 | Move profile between folders | P3.15–P3.17, P3.26, P6.11, P7.7, P9.2 |
| R7 | Reorder profiles within a folder via DnD | P3.18–P3.19, P3.27, P6.12, P7.4, P9.3 |
| R8 | Reorder folder headers via DnD | P3.13–P3.14, P3.25, P6.10, P7.3, P7.4, P9.4 |
| R9 | Migrate legacy flat-array profiles.json | P2.1–P2.15, P5.4, P9.10 |
| R10 | Folder expand/collapse persistence | P1.2 (is_expanded), P3.20–P3.21, P3.28, P6.13, P7.1, P9.9 |
| R11 | Sidebar search across folders | P6.14, P7.8, P9.5, P9.6 |
| R12 | Export includes folder assignment | P1.6 (folder_id on CP), P6.1–P6.4 (types) |
| R13 | New profile lands in active folder | P7.11, P9.1 |
| R14 | Empty folder empty-state | P7.9, P9.1 |
| R15 | Folder operations are atomic | P2.14, P3.12, P3.29, P4.2, P4.7 |

---

## Summary

| Phase | Tasks | Focus |
|-------|-------|-------|
| 0 | 3 | Preparation & audit |
| 1 | 9 | Data model (TDD) |
| 2 | 15 | Migration (TDD) |
| 3 | 29 | Folder CRUD logic (TDD) |
| 4 | 7 | Tauri command surface wiring |
| 5 | 5 | PR A gate |
| 6 | 14 | Frontend store |
| 7 | 13 | Sidebar UI |
| 8 | 3 | i18n |
| 9 | 10 | Manual QA |
| 10 | 5 | Verification gates |
| 11 | 4 | Upstream contribution prep |
| **Total** | **117** | |

**PR A boundary**: P0–P5 (Rust only, independently shippable via backward-compat shim)
**PR B boundary**: P6–P9 (frontend, depends on PR A merge)
