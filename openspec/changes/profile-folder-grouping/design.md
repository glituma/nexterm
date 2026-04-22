# Design: profile-folder-grouping

**Date**: 2026-04-19
**Phase**: design
**Sources**: `openspec/changes/profile-folder-grouping/proposal.md` (engram `sdd/profile-folder-grouping/proposal`) · `openspec/changes/profile-folder-grouping/explore.md` (engram `sdd/profile-folder-grouping/explore`)

---

## 1. Architecture Overview

```
 ┌──────────────────────────────────────────────────────────────────────┐
 │                              RENDERER (React)                         │
 │                                                                       │
 │   Sidebar.tsx  ──>  useProfileStore (Zustand)  ──>  tauriInvoke(...) │
 │   (FolderList /    { folders, profiles,             "create_folder"  │
 │    FolderRow /       expandedFolderIds,             "move_profile..." │
 │    ProfileList /     actions }                      ...               │
 │    ProfileRow)                                                        │
 └──────────────────────────────────┬───────────────────────────────────┘
                                    │ IPC (camelCase <-> snake_case)
 ┌──────────────────────────────────▼───────────────────────────────────┐
 │                               TAURI CORE (Rust)                       │
 │                                                                       │
 │   commands/profile.rs                                                 │
 │     create_folder / rename_folder / delete_folder / reorder_folders   │
 │     move_profile_to_folder / set_folder_expanded / load_profiles      │
 │               │                                                       │
 │               ▼                                                       │
 │   AppState { profiles: Mutex<Vec<CP>>, folders: Mutex<Vec<Folder>> }  │
 │               │                                                       │
 │               ▼                                                       │
 │   profile.rs                                                          │
 │     load_profiles_from_disk() — dual-format detection + migration     │
 │     save_profiles_envelope()  — atomic write via fs_secure::secure_write │
 │               │                                                       │
 │               ▼                                                       │
 │   {app_data_dir}/profiles.json   [+ profiles.backup.json on migrate]  │
 └──────────────────────────────────────────────────────────────────────┘
```

**Layers that change**
- Rust data model (`profile.rs`) — add `Folder`, `ProfilesEnvelope`, add `folder_id` + per-folder `display_order` to `ConnectionProfile`.
- Rust commands (`commands/profile.rs`) — new folder CRUD; `load_profiles` return type changes.
- Rust state (`state.rs`) — add `folders` mutex.
- Rust wiring (`lib.rs`) — register new commands in `invoke_handler!`.
- Frontend store, types, Sidebar, i18n.

**Layers that do NOT change**
- SSH session / terminal / SFTP / tunnel — folders are a sidebar concern only.
- Vault, fs_secure — reused as-is.
- Export format — only adds `folder_name: Option<String>` on `ExportedProfile` (version bump 2 → 3).

---

## 2. Data Model (Rust)

```rust
// profile.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: Uuid,
    pub name: String,
    pub display_order: i32,          // match existing ConnectionProfile convention
    #[serde(default)]
    pub is_system: bool,
    #[serde(default = "default_true")]
    pub is_expanded: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    // ...existing fields...
    #[serde(default)]
    pub folder_id: Option<Uuid>,     // post-migration invariant: always Some
    // NOTE: existing `display_order: i32` is REUSED as "order within folder"
    //       (see §2.2). No new field added.
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfilesEnvelope {
    pub folders: Vec<Folder>,
    pub profiles: Vec<ConnectionProfile>,
}
```

### 2.1 Why `Uuid` and not `String`
Consistency with every other ID in the codebase (`ConnectionProfile.id`, `UserCredential.id`, session IDs). `uuid::Uuid` already imported; serde `to_string`/`from_str` is automatic. Strings would force validation everywhere.

### 2.2 Why reuse `display_order: i32` instead of a new `order_in_folder: u32`
Proposal §5 explicitly says "reuse global `display_order`" (Option A). Rationale:
- Zero schema churn on the existing field.
- The existing `reorder_profiles` command keeps working — it just operates on a filtered view now.
- At move-to-folder time, new position is computed as `max(display_order among profiles.folder_id == target) + 1`.
- Accepting the small loss: no strict guarantee that numbers are contiguous within a folder (gaps possible). Doesn't matter — only ordering matters.

Rejected alternative: `(folder_id, order_in_folder)` tuple. Cleaner theoretically but forces a migration of every existing profile's order, touches more code, and breaks the assumption that `display_order` is a global sort key used in a dozen call sites.

### 2.3 Why `folder_id: Option<Uuid>` (not required)
Follows proposal explicitly. Rationale:
- **Serde compat**: old profiles (pre-migration in-memory state, legacy imports without `folderId`) deserialize to `None` without erroring — avoids a custom `#[serde(default)]` function.
- **Invariant enforced by code, not by type**: post-`load_profiles_from_disk`, every `Option<Uuid>` is guaranteed `Some` (migration assigns all to system folder). Tests assert this. If the type were non-optional, every serde error on malformed input would crash the load.
- Decision: type is `Option`, runtime is invariant-`Some`. Document in a doc-comment.

Task brief suggested non-optional; overridden here because proposal §5 locks `Option` and changing it would drift from spec.

### 2.4 System folder identity
**Choice**: a real persisted `Folder` row with `is_system: true` and a freshly-generated `Uuid`. NOT a fixed magic UUID.
- Pro: avoids hardcoding a sentinel in source; straightforward serde.
- Pro: `is_system` boolean is the single gatekeeper for rename/delete rejection.
- Con: system folder's UUID varies per install. Mitigation: never referenced by clients, only by the flag.
- Rejected: special-case by `name == "Ungrouped"` — brittle across locales. The display name is i18n-translated on the **frontend**; the Rust stored name is a fixed stable identifier, e.g. `"__system__"`, rendered as `t('sidebar.folders.ungroupedName')` in the UI. **Decision**: store name as `"__system__"` (ASCII, never shown raw), frontend maps to translation key when `is_system === true`.

### 2.5 Schema version field: intentionally absent
**Decision**: no `schema_version` field. Shape of the JSON root encodes the version (array = v1, object with `folders`+`profiles` = v2). Follows proposal §5 verbatim.
- Pro: zero ambiguity at detection (Rust's `serde_json::Value::is_array()`).
- Pro: no risk of a field drifting out of sync with code.
- Con: if we ever need v3 envelope shape change, we'll have to add the field then. Accepted — YAGNI now, we'll pay the price if/when v3 arrives.

Task brief suggested `schema_version: u32`; overridden because proposal locked "no schema_version" explicitly.

### 2.6 `is_expanded` semantics
Persisted on the `Folder`. Changed by a dedicated command (`set_folder_expanded`) that writes immediately. Alternative of keeping it purely frontend-local was rejected because the user asked that expand/collapse persist across restarts (success criteria §9).

---

## 3. Migration Strategy

### 3.1 Algorithm — `load_profiles_from_disk`

```text
1. path = profiles_file_path(app_data_dir)
2. if !path.exists(): return ProfilesEnvelope { folders: [<system>], profiles: [] }
3. contents = read_to_string(path)
4. raw = serde_json::from_str::<serde_json::Value>(contents)
5. match raw:
     Value::Array(_)   => legacy_migrate(raw, app_data_dir)
     Value::Object(o)  => modern_load(o)
     _                 => Err("Unrecognized profiles.json root")
6. post-process: existing per-profile legacy migration
   (migrate_legacy_fields: top-level username/authMethod → users[])
7. sort profiles by display_order, sort folders by display_order
8. return envelope
```

### 3.2 `legacy_migrate` steps

```text
a. Parse raw as Vec<ConnectionProfile> (old format)
b. For each profile: run migrate_legacy_fields() (existing logic, untouched)
c. Create one system Folder:
      Folder { id: Uuid::new_v4(), name: "__system__",
               display_order: 0, is_system: true, is_expanded: true,
               created_at: now, updated_at: now }
d. Assign folder_id = Some(system.id) to every profile
e. Write backup: copy original file → profiles.backup.json
   (see §3.3 for collision handling)
f. Write new envelope via save_profiles_envelope (atomic, hardened)
g. Return ProfilesEnvelope { folders: [system], profiles }
```

### 3.3 Backup file lifecycle

| Scenario | Behavior |
|---|---|
| Backup file does **not** exist | `fs::copy` source → `profiles.backup.json`, then `fs_secure::best_effort_harden`. |
| Backup file **already** exists | Do NOT overwrite. Rename existing to `profiles.backup.<timestamp>.json` (`YYYYMMDDTHHMMSS`) first, then create new backup. Worst case on rename failure: log at `warn!`, skip backup, **still proceed with migration** (durability > historical backup). |
| Migration write fails | `fs_secure::secure_write` is atomic (tmp + rename). Original `profiles.json` on disk is untouched because rename never happened. Surface error; don't flip in-memory state. User restarts, tries again. |

**Retention**: backups are kept forever. User-owned files in user's app-data dir. No automated rotation (too risky for a rescue artifact). Documented in the repo README follow-up.

### 3.4 Edge cases

| Input | Behavior |
|---|---|
| `profiles.json` missing | Create in-memory envelope with a fresh system folder; first write-triggering op persists it. No backup (nothing to back up). |
| `profiles.json` empty / 0 bytes | Treated as "missing" path. |
| `profiles.json` corrupted JSON | Return `Err(AppError::ProfileError("Failed to parse profiles.json: <e>"))`. DO NOT auto-overwrite. Leave corrupted file untouched, prompt user via error banner. |
| Object root with `folders` absent | Treat as v2 with empty folders; migration creates system folder on first save. |
| Object root with duplicate folder IDs or profiles pointing at missing folder_id | Re-heal: unknown `folder_id` → reassign to system folder; log at `warn!`. Duplicate IDs → return error (don't silently mangle). |

### 3.5 Idempotency
Running `load_profiles_from_disk` twice on an already-migrated file is a no-op (object root → `modern_load`; no write). Running on legacy triggers migration on the FIRST load; subsequent loads hit the modern path.

---

## 4. Tauri Command Surface

All commands **persist to disk immediately** (consistency > batched perf; vault precedent). Any failure returns `AppError::ProfileError(...)` → JS promise rejection.

| Command | Input | Output | New error variants |
|---|---|---|---|
| `load_profiles` *(CHANGED)* | – | `ProfilesResponse { folders: Vec<Folder>, profiles: Vec<ConnectionProfile> }` | — |
| `create_folder` | `name: String` | `Folder` | `ProfileError("Folder name cannot be empty")`, `ProfileError("Folder name too long (max 64 chars)")`, `ProfileError("Folder name already exists")` *(case-insensitive)* |
| `rename_folder` | `folder_id: Uuid`, `new_name: String` | `Folder` | `ProfileError("System folder cannot be renamed")`, empty/length errors, duplicate-name error |
| `delete_folder` | `folder_id: Uuid` | `DeleteFolderResult { moved_profile_count: u32 }` | `ProfileError("System folder cannot be deleted")`, `ProfileError("Folder not found: <id>")` |
| `reorder_folders` | `folder_ids: Vec<Uuid>` | `()` | `ProfileError("Folder IDs do not match current folder set")` if the list is a non-permutation |
| `move_profile_to_folder` | `profile_id: Uuid`, `folder_id: Uuid` | `()` | `ProfileError("Folder not found")`, `ProfileError("Profile not found")` |
| `reorder_profiles_in_folder` | `folder_id: Uuid`, `profile_ids: Vec<Uuid>` | `()` | folder-not-found; mismatched-set error |
| `set_folder_expanded` | `folder_id: Uuid`, `expanded: bool` | `()` | `ProfileError("Folder not found")` |

### 4.1 Validation rules summary
- **Name trim**: all folder names are `name.trim()`-ed before storage.
- **Name length**: `1..=64` chars (after trim). Same limit as profile name (implicit).
- **Duplicate name**: case-insensitive (`name.to_lowercase()`). Enforced in `create_folder` and `rename_folder`.
- **System folder protection**: reject `rename_folder` and `delete_folder` when `folder.is_system == true`.
- **`move_profile_to_folder`**: placement is "end of target folder" — backend computes `display_order = max_order_in_target + 1`. No explicit index parameter (simpler; reorder handles fine-grained control).
- **`delete_folder`**: reassigns all member profiles' `folder_id` to the system folder's id and bumps their `display_order` to the tail of the system folder.

### 4.2 Why `load_profiles` changes signature (not a new command)
Avoids two round-trips at cold-start. One IPC returning both arrays is strictly better than `load_folders` + `load_profiles`. Frontend impact: one-line store change (destructure the envelope).

### 4.3 Why `DeleteFolderResult` returns a count
Frontend shows "Moved 3 profiles to Ungrouped" toast. Without the count the UI has to diff old-vs-new state — wasteful.

---

## 5. Frontend State Shape (Zustand)

```typescript
// src/lib/types.ts
export interface ProfileFolder {
  id: string;
  name: string;           // "__system__" for the system folder; UI renders translation
  displayOrder: number;
  isSystem: boolean;
  isExpanded: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ConnectionProfile {
  // ...existing...
  folderId: string | null;  // null only transiently; post-load always set
}

// src/stores/profileStore.ts
interface ProfileStoreState {
  folders: ProfileFolder[];
  profiles: ConnectionProfile[];
  loading: boolean;
  error: string | null;

  // existing actions retained
  loadProfiles: () => Promise<void>;        // now populates folders too
  saveProfile: (p: ConnectionProfile) => Promise<string>;
  deleteProfile: (id: string) => Promise<void>;
  reorderProfiles: (ids: string[]) => Promise<void>;  // kept for compat; deprecated after rollout
  exportProfiles / importProfiles / storeCredential / clearError: (unchanged)

  // new actions
  createFolder: (name: string) => Promise<ProfileFolder>;
  renameFolder: (id: string, newName: string) => Promise<void>;
  deleteFolder: (id: string) => Promise<number>;               // returns moved count
  reorderFolders: (folderIds: string[]) => Promise<void>;
  moveProfileToFolder: (profileId: string, folderId: string) => Promise<void>;
  reorderProfilesInFolder: (folderId: string, profileIds: string[]) => Promise<void>;
  toggleFolderExpanded: (folderId: string) => Promise<void>;
}
```

### 5.1 Optimistic vs pessimistic policy

| Action | Strategy | Rationale |
|---|---|---|
| `reorderFolders`, `reorderProfilesInFolder` | **Optimistic** (update local, rollback on reject by reloading) | DnD feels dead without immediate feedback; follows existing `reorderProfiles` pattern |
| `toggleFolderExpanded` | **Optimistic** | Cheapest possible op; rollback on failure is invisible |
| `moveProfileToFolder` | **Optimistic** | Same as reorder; visual move confirms the drop |
| `createFolder` | **Pessimistic** (wait for backend to return folder with server-issued UUID) | Need the server ID to reference it for subsequent ops |
| `renameFolder` | **Optimistic** | Low stakes |
| `deleteFolder` | **Pessimistic** + **confirmation dialog** (always, even for empty) | Destructive; we want the user's deliberate click |

### 5.2 Expansion state: single source of truth
`folder.isExpanded` on each folder row is the truth. The previously-existing per-profile `expandedProfiles: Set<string>` in `Sidebar.tsx` is unrelated (it expands a profile to show its sessions) and remains untouched.

---

## 6. Sidebar UI Structure

### 6.1 Component tree

```
Sidebar
 ├─ SearchInput
 ├─ ActionsRow        (Import / Export / New Profile / New Folder [+])
 ├─ DndContext (root; single instance)
 │   ├─ SortableContext (folders, strategy = verticalListSortingStrategy)
 │   │   └─ FolderRow (xN)
 │   │       ├─ FolderHeader      (chevron, name, count badge, context menu)
 │   │       └─ SortableContext (profiles in this folder)
 │   │           └─ SortableProfileCard (existing component, reused)
 └─ Dialogs         (DeleteProfile, DeleteFolder, CreateFolder, RenameFolder,
                     Export, Import, ...)
```

### 6.2 DnD wiring

- **Single `DndContext`** at Sidebar root (required by @dnd-kit — multiple contexts don't share draggable IDs).
- **Folder-level `SortableContext`**: items = `folders.map(f => 'folder:' + f.id)` (prefix to disambiguate from profile IDs).
- **Profile-level `SortableContext`**: one per folder, items = profiles of that folder.
- `handleDragEnd` routes on prefix: `folder:` → `reorderFolders`, else → `reorderProfilesInFolder` (reject cross-folder drop in MVP — `if (activeProfile.folder_id !== overProfile.folder_id) return;`).

### 6.3 Cross-folder move (NOT drag-and-drop in MVP)
- Context menu on `ProfileRow` → **"Move to folder"** submenu → list of folders (with system folder pinned at top) → click → `moveProfileToFolder`.
- Follow-up ticket: enable real cross-folder DnD once `closestCenter` strategy + ghost-preview is validated.

### 6.4 Folder CRUD UI
- **Create**: `+` button next to "Folders" header opens inline dialog with text input.
- **Rename**: context menu on folder header → "Rename" → inline-edit or dialog (dialog for consistency with delete-confirm).
- **Delete**: context menu → "Delete folder" → confirmation dialog: `"Delete '{name}'? {count} profiles will be moved to 'Ungrouped'."` → on confirm, call `deleteFolder`.

### 6.5 Search behavior
- While `searchQuery.trim() !== ''`:
  - Render a **flat** list of matching profile cards.
  - Each card shows a `[Folder Name]` badge (or `[Ungrouped]`).
  - Folder headers hidden entirely.
  - DnD disabled during search (reordering within search results has no meaning).
- On `searchQuery` cleared: folder structure restored; previous expansion state preserved.

### 6.6 "Active folder" (for new-profile placement)
Kept in Sidebar local state (`activeFolderId: string | null`). Last clicked folder header becomes active. "+ New Profile" button passes `activeFolderId ?? systemFolderId` to the NewProfile flow; the `save_profile` command accepts a `folder_id` parameter (set from the frontend).

---

## 7. Internationalization

All new keys live under the `sidebar.folders.*` namespace.

```ts
// EN keys to add (es.ts MUST mirror — TS compile blocks drift)
"sidebar.folders.title": "Folders",
"sidebar.folders.ungroupedName": "Ungrouped",
"sidebar.folders.new": "New folder",
"sidebar.folders.newShort": "Folder",
"sidebar.folders.createTitle": "Create folder",
"sidebar.folders.createPlaceholder": "Folder name",
"sidebar.folders.renameTitle": "Rename folder",
"sidebar.folders.rename": "Rename",
"sidebar.folders.delete": "Delete folder",
"sidebar.folders.deleteConfirmTitle": "Delete folder",
"sidebar.folders.deleteConfirmMessage": "Delete '{name}'? {count} profile(s) will be moved to '{systemName}'.",
"sidebar.folders.deleteConfirmEmpty": "Delete '{name}'? This folder is empty.",
"sidebar.folders.moveTo": "Move to folder",
"sidebar.folders.moveToSubmenu": "Move '{profile}' to…",
"sidebar.folders.moveSuccess": "Moved to '{folder}'",
"sidebar.folders.emptyHint": "Drop profiles here or right-click a profile → Move to folder",
"sidebar.folders.systemProtected": "The system folder cannot be modified.",
"sidebar.folders.duplicateName": "A folder with this name already exists.",
"sidebar.folders.nameRequired": "Folder name is required.",
"sidebar.folders.nameTooLong": "Folder name is too long (max 64 characters).",
"sidebar.folders.searchBadge": "[{folder}]",
"sidebar.folders.countBadge": "{count}",
"sidebar.folders.collapse": "Collapse folder",
"sidebar.folders.expand": "Expand folder",
```

**Process rule**: update `en.ts` **first**. `es.ts` is typed as `Record<TranslationKey, string>`, so the compiler refuses to build until every EN key is mirrored in ES. This is the enforcement mechanism — do not invert the order (ES-first produces a sea of compile errors that hide the real additions).

---

## 8. Testing Strategy

### 8.1 Rust (strict TDD per project rule)

| Layer | Test (representative) | Location |
|---|---|---|
| Unit — data model | `folder_serialize_roundtrip`, `folder_validation_rejects_empty_name`, `folder_validation_rejects_long_name` | `profile.rs` |
| Unit — envelope | `envelope_serialize_roundtrip`, `envelope_sorts_profiles_by_display_order` | `profile.rs` |
| Unit — dual-format | `load_detects_legacy_array_root`, `load_detects_modern_object_root`, `load_rejects_unrecognized_root` | `profile.rs` |
| Unit — migration | `migration_creates_system_folder`, `migration_assigns_all_profiles_to_system`, `migration_creates_backup`, `migration_is_idempotent_on_modern_file`, `migration_rotates_existing_backup`, `migration_preserves_legacy_user_migration` | `profile.rs` |
| Unit — atomic rollback | `save_envelope_tmp_cleaned_on_harden_failure` (leverage existing `fs_secure` seam) | `profile.rs` |
| Unit — commands | `create_folder_validates_name`, `create_folder_rejects_duplicate`, `rename_folder_rejects_system`, `delete_folder_reassigns_to_system`, `delete_folder_rejects_system`, `move_profile_to_folder_updates_display_order`, `reorder_folders_persists`, `reorder_profiles_in_folder_scoped`, `set_folder_expanded_persists` | `commands/profile.rs` |
| Integration | full load → create folder → move profile → save → reload → assert shape | `profile.rs` (tempdir pattern, existing) |

**Test file locations**: `#[cfg(test)] mod tests` inline in `profile.rs` and `commands/profile.rs`, matching the existing vault-storage-security pattern.

### 8.2 TypeScript

No Vitest / Jest currently configured. Proposal does not authorize adding a JS test runner. Manual QA checklist (reuse as smoke test after implementation):

1. Fresh install → open app → system folder "Ungrouped" visible, expanded, empty.
2. Legacy install → relaunch → all existing profiles under Ungrouped, `profiles.backup.json` on disk.
3. Create folder "Proxmox" → appears at bottom with `(0)` badge.
4. Drag profile into Proxmox via context menu → Ungrouped count decrements, Proxmox `(1)`.
5. Collapse Proxmox → reopen app → still collapsed.
6. Drag folders to reorder → reopen → order preserved.
7. Delete Proxmox → confirmation shows "1 profile moved" → profile back in Ungrouped.
8. Rename/delete Ungrouped → UI actions hidden; attempt via devtools IPC → backend rejects with localized error.
9. Search "prod" → flat list with `[Folder Name]` badges; clear → structure returns.
10. ES locale → every new string localized.

---

## 9. Accessibility

- **Folder row**:
  - `role="group"` on the collapsible region.
  - `aria-expanded="true|false"` on the folder header button.
  - `aria-label` on header: `"{name}, {count} profiles"`.
  - `aria-controls` pointing to the children list id.
- **Profile row**: keeps existing semantics.
- **Keyboard**:
  - `ArrowDown` / `ArrowUp`: move focus across all rows (folder headers and profile rows) in visual order.
  - `ArrowRight`: expand a collapsed focused folder; if already expanded, move into first child profile.
  - `ArrowLeft`: collapse an expanded focused folder; if already collapsed, move focus to parent folder (no-op at top).
  - `Enter` on profile: trigger connect (existing behavior).
  - `Enter` on folder header: toggle expand.
  - `F2` on folder header: rename.
  - `Delete` on folder header: delete (with confirmation).
- **Focus ring**: every interactive element has a visible `:focus-visible` outline (reuse the token already present in `sidebar.css`).

---

## 10. Performance Considerations

| Scale | Behavior | Action |
|---|---|---|
| 10 folders · 100 profiles | Render cost negligible. One `DndContext` + 11 `SortableContext` instances well within @dnd-kit budget. | Ship as-is. |
| 50 folders · 500 profiles | Render may start to stutter on layout. | Keep as-is for MVP; follow-up ticket to add `@tanstack/react-virtual` per-folder (already a dependency in the repo — used by FileViewer). |
| 1000+ profiles | Not a target for this change. | Out of scope. |

**Persist debouncing**: NOT applied. Every mutation syncs to disk immediately. Rationale:
- Profile mutations are rare (user-initiated, single action).
- `fs_secure::secure_write` is atomic — a crash mid-write leaves the OLD file intact.
- Debouncing introduces a window where in-memory state differs from disk — the exact failure class the vault-hardening change just eliminated. Not worth it.

Only `set_folder_expanded` runs on a user gesture frequent enough to stress disk IO; even so, 100s of toggles/day is nothing compared to the vault's write volume.

---

## 11. Error Handling & Rollback

### 11.1 Backend failure
- `save_profiles_envelope` uses `fs_secure::secure_write`: atomic tmp + hardened rename. On write error, disk is unchanged, error surfaces as `AppError::ProfileError`.
- In-memory state mutation happens BEFORE the write inside each command. On write failure we **re-load from disk** to restore in-memory from the truth source. Ensures `AppState.folders` never diverges from `profiles.json`.
- Existing pattern in `reorder_profiles` (already in codebase) is the template.

### 11.2 Frontend failure
- Optimistic actions: on IPC reject, the store catches, sets `error`, and calls `loadProfiles()` to re-sync (same pattern as existing `reorderProfiles`).
- Pessimistic actions (create, delete): show error in dialog; user can retry or cancel. State is untouched on failure.
- Toast / inline banner (existing `sidebar-banner` component) surfaces localized error strings by mapping stable error codes when we add them (v-next; MVP just surfaces the raw message).

### 11.3 Corruption recovery
If `profiles.json` parse fails, the error banner instructs the user to either:
- Rename `profiles.backup.json` → `profiles.json` and relaunch (manual rollback per proposal §11).
- File a bug with the corrupted JSON attached.

We DO NOT auto-replace a corrupted file with the backup. A silent overwrite could wipe days of legitimate work if the corruption was transient (e.g. an aborted write on an older build). User consent required.

---

## 12. Upstream Contribution Plan

### PR A — Backend (~300 LOC)
**Changed files**:
- `src-tauri/src/profile.rs` (+Folder struct, +ProfilesEnvelope, migration, dual-format load, envelope save)
- `src-tauri/src/commands/profile.rs` (new folder commands + load_profiles signature)
- `src-tauri/src/state.rs` (+folders mutex)
- `src-tauri/src/lib.rs` (invoke_handler! additions)
- No frontend touched.
- Full TDD suite (~20 new tests).
**Shipping signal**: `cd src-tauri && cargo test` green + `cargo build --release` clean.
**Risk**: zero UI breakage (load_profiles changed signature, but no UI consumer exists yet — PR B wires it).

**Gap between PRs**: after PR A merges, mainline's frontend (`loadProfiles`) is broken because it calls `load_profiles` and expects `Vec<ConnectionProfile>` but receives `ProfilesResponse`. To avoid a broken main, one of:

**Decision**: ship PR A with a **backward-compat IPC shim** — `load_profiles` returns `Vec<ConnectionProfile>` when the request has no schema marker, `ProfilesResponse` when it does. Simplest implementation: keep `load_profiles` returning `Vec<CP>` and add a NEW command `load_profiles_with_folders` that returns `ProfilesResponse`. PR B migrates the frontend to the new command; a follow-up deprecation removes the old one.

This keeps PR A independently shippable — an open-source maintainer can merge it without the frontend change.

### PR B — Frontend (~500 LOC)
**Changed files**:
- `src/lib/types.ts` (+ProfileFolder, +folderId on ConnectionProfile)
- `src/stores/profileStore.ts` (state + actions)
- `src/components/layout/Sidebar.tsx` (refactor)
- `src/lib/i18n/en.ts`, `src/lib/i18n/es.ts` (new keys, lock-step)
- Switch `loadProfiles` to call `load_profiles_with_folders`.
**Depends on**: PR A merged.
**Shipping signal**: manual QA checklist §8.2 green on both EN and ES locales.

### Upstream issue
File a NEW issue on `CogniDevAI/nexterm` titled "Profile folder grouping". Link this SDD folder (`openspec/changes/profile-folder-grouping/`) for transparency. Do NOT reuse issue #1 (vault ACL).

---

## 13. Open Technical Questions

Flag for maintainer in the upstream issue.

- [ ] **Export schema bump**: the proposal added `folder_name: Option<String>` on `ExportedProfile` (envelope v3). Do we also export folder colors/ordering if we ever add them? **Recommendation**: keep export intentionally minimal (`folder_name` string only) — avoids coupling export to internal identity shape.
- [ ] **System folder render position**: proposal open-question §7.2. **Recommendation**: render first (home). Users instinctively look at the top when everything is uncategorized; moving "Ungrouped" to the bottom hides the default drop zone.
- [ ] **Capability spec name**: proposal open-question §7.3. **Recommendation**: `profile-organization` (narrower; `profile-management` is claimed by CRUD concerns).
- [ ] **Cross-folder DnD**: deferred follow-up. Sketch: add a sentinel `SortableContext` for the folder drop zones, detect `activeProfile.folder_id !== overFolder.id` in `handleDragEnd`, call `moveProfileToFolder(id, folder_id)`. Not MVP.
- [ ] **Stable error codes for frontend localization**: today errors surface as raw Rust strings. When we add stable codes (`"folder.duplicate_name"`, `"folder.system_protected"`), the frontend maps them to `sidebar.folders.*` strings. MVP acceptable without, but tech debt.
- [ ] **@dnd-kit accessible drag announcements**: `DndContext` supports `accessibility.announcements` — are we wiring localized strings for SR users? **Recommendation**: yes, add in PR B with keys under `sidebar.folders.dnd.*`.

---

*End of design.*
