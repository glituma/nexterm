# Exploration: profile-folder-grouping

**Date**: 2026-04-19  
**Change**: `profile-folder-grouping`  
**Phase**: explore  
**Model**: anthropic/claude-sonnet-4-6

---

## 1. Current State Analysis

### 1.1 `ConnectionProfile` Rust Struct Fields

File: `src-tauri/src/profile.rs` (lines 32–56)

```rust
pub struct ConnectionProfile {
    pub id: Uuid,                          // PK
    pub name: String,                      // display name
    pub host: String,                      // hostname/IP
    pub port: u16,                         // SSH port (default 22)
    pub username: Option<String>,          // legacy only — #[serde(skip_serializing)]
    pub auth_method: Option<AuthMethodConfig>, // legacy only — #[serde(skip_serializing)]
    pub users: Vec<UserCredential>,        // canonical multi-user array
    pub startup_directory: Option<String>, // optional remote startup path
    pub tunnels: Vec<TunnelConfig>,        // tunnels embedded in profile
    pub display_order: i32,                // sort key in profiles.json
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

**No `folder_id` or grouping field exists today.** Adding one is the primary model change.

**Serde convention**: `#[serde(rename_all = "camelCase")]` — all JSON keys are camelCase. New fields must follow this convention (e.g., `folder_id` → `"folderId"` in JSON).

### 1.2 Persistence — `profiles.json`

- **Path**: `{app_data_dir}/profiles.json` (via `app.path().app_data_dir()` on Tauri)
- **Format**: Plain JSON array of `ConnectionProfile` objects — e.g., `[{...}, {...}]`
- **Write path**: `fs_secure::secure_write` — atomic temp-file rename with owner-only ACL hardening (cross-platform)
- **Load + auto-migration**: `load_profiles_from_disk` deserializes, auto-migrates legacy `username`/`auth_method` → `users[]`, re-saves if migrated, creates `profiles.backup.json` on first migration

**Schema version**: There is **no explicit `version` field** in the JSON file or envelope today. Migration is inferred by field presence (`users.is_empty() && username.is_some()`). Adding folders requires either a new schema version field or a field-presence heuristic.

**No separate `folders.json`**: All data lives in `profiles.json` as a single file.

### 1.3 In-Memory State

`state.rs` line 29: `profiles: Mutex<Vec<ConnectionProfile>>`. A flat `Vec` in `AppState`. No folder structure in memory today.

### 1.4 Current Zustand Store Shape

File: `src/stores/profileStore.ts`

**State**:
```typescript
profiles: ConnectionProfile[]; // flat array, order matches display_order
loading: boolean;
error: string | null;
```

**Actions**:
```typescript
loadProfiles()       // GET from disk, sorts by display_order
saveProfile(p)       // CREATE or UPDATE, reloads after
deleteProfile(id)    // DELETE
storeCredential(...)  // vault write
reorderProfiles(ids) // optimistic reorder, persists display_order
exportProfiles(...)  // export to file
importProfiles(...)  // import from file
clearError()
```

**No folder state exists.** New store additions will be needed for: folder list, folder CRUD, folder expand/collapse state, move-profile-to-folder.

### 1.5 TypeScript `ConnectionProfile` Type

File: `src/lib/types.ts`

```typescript
export interface ConnectionProfile {
  id: string;
  name: string;
  host: string;
  port: number;
  users: UserCredential[];
  startupDirectory?: string;
  tunnels: TunnelConfig[];
  displayOrder?: number;
  createdAt: string;
  updatedAt: string;
}
```

**No `folderId` or folder-related field.** The TS type must stay in sync with the Rust struct — it's the IPC contract.

### 1.6 Sidebar Rendering Today

File: `src/components/layout/Sidebar.tsx`

**Current flow**:
1. `profiles` (flat array from store) → filtered by `searchQuery` → `filteredProfiles`
2. `filteredProfiles` rendered via `@dnd-kit` `SortableContext` with `verticalListSortingStrategy`
3. Each item: `SortableProfileCard` — drag handle, status dot, name, host, connect/edit/delete buttons
4. Nested sessions shown inline under expanded profiles

**Current DnD**: `@dnd-kit/core` + `@dnd-kit/sortable` for flat profile reorder. The `SortableContext` wraps a single flat list. Folder grouping will require rethinking this — either nested DnD contexts (complex) or a different interaction model.

**`@tanstack/react-virtual`**: Confirmed NOT used in Sidebar. It's only in `FileViewer.tsx` for large text files. The sidebar is a plain DOM list — no virtualization issue.

**Expand/collapse**: A `Set<string>` of profile IDs is kept in `useState`. Folder expand/collapse will need a parallel `Set<string>` for folder IDs.

**Search**: Currently filters `profiles` array in-place by name/host/username. Returns flat `filteredProfiles`.

### 1.7 Export/Import — Current Folder Relevance

File: `src-tauri/src/commands/profile.rs`

Current export format (`ExportEnvelope`):
```json
{
  "version": 2,
  "app": "NexTerm",
  "exported_at": "...",
  "profiles": [
    { "name": "...", "host": "...", "port": 22, "users": [...] }
  ]
}
```

**No folder information is exported today.** When we add folders, we must decide whether `ExportEnvelope` v3 includes folder data or whether `ExportedProfile` gets a `folder_name: Option<String>` field for portability. The current version bump is `2` — we control this versioning.

**Import duplicate check**: `existing.name == ep.name && existing.host == ep.host` — folder membership is not part of deduplication. This is fine.

### 1.8 Existing Commands (Tauri IPC surface)

Current profile commands registered in `lib.rs`:
- `save_profile`, `load_profiles`, `delete_profile`, `get_profile`
- `export_profiles`, `import_profiles`, `reorder_profiles`

**All need no change for folders** — new commands will be added alongside them.

### 1.9 i18n Structure

- `en.ts` defines `const en = {...} as const` → TypeScript infers `TranslationKey` type from keys
- `es.ts` is typed as `Record<TranslationKey, string>` — TypeScript **enforces** that every key in `en.ts` has a Spanish equivalent. Missing key = compile error.
- Pattern: keys are namespaced strings like `"sidebar.profiles"`, `"connection.name"`
- New keys for folders must be added to BOTH files simultaneously.

### 1.10 Existing Specs

`openspec/specs/vault-storage-security/spec.md` exists — covers the vault ACL hardening feature (archived). No capability spec yet exists for **profile organization**. This feature will create a new capability spec: likely `openspec/specs/profile-management/spec.md` or `openspec/specs/profile-organization/spec.md`.

---

## 2. Reference Analysis — Competitor UX Patterns

### 2.1 Termius — "Groups" model (commercial benchmark)

**What it does**: Termius uses "Groups" — not folders per se, but named collections that double as inheritance containers. A Group can be nested (multi-level). Hosts can be inside only one group at a time. The sidebar shows hosts in the current group AND all subgroups (not strictly folder-scoped).

**Key features**:
- Groups can define default protocol settings, credentials, terminal themes, proxy, etc. inherited by all hosts
- Drag-and-drop to move hosts between groups
- Right-click → "Move to group" via context menu
- Groups also enable "Connect to all hosts at once"
- **Tags** are orthogonal to groups — hosts can have multiple tags; tags filter search results
- Uncategorized hosts appear at the top level with no group

**What Termius does well**: Inheritance (defaults cascade down), per-group protocol settings, batch connect. **What it does poorly**: Groups and Folders are semantically confusing (they're called Groups but behave like folders). The UI can become complex with deep nesting + tags + workspaces.

**What NexTerm should steal**: Simple named folder concept, drag-and-drop host → folder. **Avoid**: Inheritance/defaults cascade (out of scope for NexTerm MVP), tags (separate paradigm decided out of scope).

### 2.2 Tabby — Connection Manager (direct open-source competitor)

**What it does**: Tabby (70k GitHub stars) has a Connection Manager with a tree-style sidebar. Hosts can be grouped by connection type (SSH, Telnet, Serial) but Tabby also supports manual groups/folders. Configuration is stored in `~/.config/tabby/config.yaml`.

**Persistence format**: YAML with a flat list of connections each having a `group: "GroupName"` string field. Groups are created implicitly when first referenced (no separate group entity in config). The folder structure is derived from the `group` string.

**What Tabby does well**: Simple YAML-based grouping via a string field — easy portability. Groups appear as collapsible headers in the connection list. **What it does poorly**: No drag-and-drop reorder within groups, no explicit group ordering (alphabetical), no color/icon customization. Derived (implicit) groups are fragile — renaming requires mass-update of all member profiles.

**What NexTerm should steal**: The `group` string field as a portability-friendly export approach. **Avoid**: Implicit/derived groups from string matching — they're fragile; use explicit `folder_id` references instead.

### 2.3 Royal TSX — Enterprise client (Windows/macOS)

**What it does**: Royal TSX is a paid enterprise client. It organizes connections into "Folders" within "Documents". Folders are tree-structured (unlimited nesting). Connections can only be in one folder. The default folder per document is the root.

**Key UX**: Folders in the tree sidebar, chevron expand/collapse, drag-and-drop between folders. Context menu: New Connection → goes into the current folder. Move via drag-and-drop or cut/paste.

**What Royal TSX does well**: Clear, simple folder metaphor users already understand from file managers. Explicit folder entity with its own properties (name, notes, color). **What it does poorly**: Unlimited nesting becomes a maze — users create 5-6 levels deep hierarchies they later can't navigate. The "Document" abstraction adds a layer most users don't need.

**What NexTerm should steal**: Single-level folder + explicit folder entity (not implicit). Chevron expand/collapse per folder. **Avoid**: Unlimited nesting (already decided out — 1 level only).

### 2.4 WindTerm — Modern SSH client

**What it does**: WindTerm has a Session Manager with groups/tags. Connections can belong to one group. Groups are shown as collapsible sections in the session tree. There is a "Default" group for ungrouped sessions. Drag-and-drop between groups is supported.

**Key UX**: Groups shown with count badge (e.g., "Proxmox (3)"). Expand/collapse. Search stays in folder context by default (shows folder structure), with a mode to flatten.

**What WindTerm does well**: Count badge per folder, "Default" group for ungrouped, search-in-folder-context vs. flat-search toggle. **What it does poorly**: Too many right-click menu options for basic folder operations — UX is cluttered.

**What NexTerm should steal**: Count badge per folder header, "Ungrouped" system folder concept. **Avoid**: Right-click-heavy UX (prefer inline actions or edit dialog for folder operations).

---

## 3. Technical Decision Points

### 3.1 Data Model: Where Does the Folder Reference Live?

**Option A — `folder_id: Option<Uuid>` on `ConnectionProfile`** (forward reference)

Each profile references its folder by UUID. `Folder` entities are stored separately (either in `profiles.json` alongside profiles or in a separate `folders.json`).

- Pros: Simple model, referential integrity possible, portable UUID reference, profile JSON is self-contained
- Cons: Requires lookup join to get folder name for display; folder entity must be managed separately
- Effort: Medium

**Option B — `folder_name: Option<String>` on `ConnectionProfile`** (denormalized string, Tabby-style)

Each profile stores the folder name directly. No separate folder entity — folders are derived from the set of unique names.

- Pros: Extremely simple, portable, no extra data structure, matches Tabby/export-friendly format
- Cons: Renaming a folder requires mass-update of all member profiles; no ordering for folders (alphabetical only); no room for future folder properties (color, icon); "Sin agrupar" identification relies on special string value
- Effort: Low

**Option C — Separate `Folder` entity list + `folder_id` on profile** (normalized)

`profiles.json` becomes an envelope: `{ "schema_version": 2, "folders": [...], "profiles": [...] }`. Folder list is a peer of profiles list.

- Pros: Clean separation; folders can have their own properties (order, color later); referential integrity; explicit schema version in the file
- Cons: Bigger change to persistence format; requires schema version detection; more complex migration
- Effort: Medium-High

**→ Recommended**: **Option C** — The JSON envelope restructure. Rationale: (1) this is the only design that cleanly supports a schema version field for forward/backward compatibility; (2) it enables future folder properties (color, icon) without another migration; (3) it makes the "Sin agrupar" default folder a proper first-class entity with a fixed UUID instead of a magic string. The migration from current flat array is straightforward.

### 3.2 "Sin agrupar" / "Ungrouped" Default Folder

**Option A — Virtual/computed folder** (no actual persistent entity)

At load time, profiles with `folder_id = None` are grouped under a computed "Sin agrupar" header. No folder entity stored.

- Pros: No storage overhead, backwards compatible
- Cons: No stable ID for the virtual folder; cannot rename it; cannot reorder it relative to real folders; edge cases when all profiles have folders (virtual folder still appears? disappears?)
- Effort: Low

**Option B — Real persistent folder with a fixed/well-known UUID**

"Sin agrupar" is a real `Folder` entity stored in `folders[]`. It has a sentinel/reserved UUID (e.g., `00000000-0000-0000-0000-000000000001`) or a boolean flag `is_system: true`. It cannot be renamed or deleted by the user.

- Pros: Uniform code path for all folders; stable ordering; can have `display_order` like other folders; TypeScript components don't need special-case logic
- Cons: Slightly more storage; sentinel UUID is a convention that must be documented
- Effort: Low (same code path as regular folders)

**→ Recommended**: **Option B** — system folder with `is_system: true` flag (avoids magic UUID). The UI blocks rename/delete when `is_system = true`. Migration puts all existing profiles into this folder.

### 3.3 `Folder` Entity — MVP vs Future Fields

**MVP fields** (ship in this change):
```rust
pub struct Folder {
    pub id: Uuid,
    pub name: String,
    pub display_order: i32,
    pub is_system: bool,      // true = "Sin agrupar" / cannot delete/rename
    pub is_expanded: bool,    // persisted expand/collapse state (default: true)
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

**Future fields** (out of scope for this change):
- `color: Option<String>` — folder color accent
- `icon: Option<String>` — folder icon name
- `description: Option<String>` — folder notes/label

**TypeScript mirror**:
```typescript
export interface ProfileFolder {
  id: string;
  name: string;
  displayOrder: number;
  isSystem: boolean;
  isExpanded: boolean;
  createdAt: string;
  updatedAt: string;
}
```

### 3.4 Ordering — Folders and Profiles Within Folders

**Folder ordering**: Each `Folder` has `display_order: i32` (same pattern as `ConnectionProfile.display_order`). Folders are sorted by this field on load.

**Profile ordering within a folder**: `ConnectionProfile.display_order` continues to serve as the profile's global position. Within-folder ordering is derived by filtering profiles by `folder_id` and sorting by `display_order`.

**Option A — Global `display_order` across all profiles** (current approach extended)

Profiles keep their global `display_order`. Within a folder, they're simply sorted by this global key. Moving a profile between folders doesn't change its `display_order` unless the user explicitly reorders.

- Pros: No change to existing `display_order` semantics; `reorder_profiles` command mostly unchanged
- Cons: When a profile moves folders, its position within the new folder is undefined (goes to "last" based on max_order)
- Effort: Low

**Option B — Per-folder `display_order`** (scoped ordering)

Each profile's `display_order` is relative within its folder (0, 1, 2, ... per folder independently).

- Pros: Clean per-folder ordering; reorder within folder doesn't affect other folders
- Cons: More complex `reorder_profiles` command; need to rebuild all `display_order` values when a profile moves folders
- Effort: Medium

**→ Recommended**: **Option A** for MVP — use global `display_order`, assign `max_order + 1` when moving to a new folder. Folder-scoped ordering can be added later. This avoids redesigning `reorder_profiles`.

### 3.5 Migration of Existing 15 Profiles

**Option A — All profiles → "Sin agrupar" (automatic, silent)**

On first load after the upgrade, if the JSON file is the old flat-array format (detected by absence of `schema_version` or `folders` key), all existing profiles are placed into the default "Sin agrupar" folder automatically. No user interaction required.

- Pros: Zero friction for the user; backwards compatible; idempotent (migration only runs once — backup created)
- Cons: User must manually organize from scratch (but that's the expected workflow)
- Effort: Low

**Option B — Name-prefix heuristic grouping**

Analyze profile names for common prefixes (e.g., "Proxmox-1", "Proxmox-2" → "Proxmox" group) and auto-create folders.

- Pros: Potentially useful first-run experience
- Cons: Heuristics are fragile; user has 15 profiles with arbitrary names; the AI guess may be wrong and confusing; hard to test; poor trust signal
- Effort: High, low value

**Option C — First-run wizard**

On first launch after upgrade, show a dialog: "We've added folder support! Your profiles are in 'Sin agrupar'. Would you like to create your first folder now?"

- Pros: Good UX, user-directed
- Cons: More complex, requires additional UI component, not strictly needed
- Effort: Medium

**→ Recommended**: **Option A** — automatic silent migration. The migration backup file pattern (profiles.backup.json) already exists in the codebase. We extend it: old flat-array JSON → backup → new envelope-format JSON with all profiles in system folder. The user then organizes from scratch, which is the expected interaction.

### 3.6 Backward Compatibility — Schema Version

**Problem**: If a user installs a newer NexTerm with folders on device A, then opens the app on device B with an older version of NexTerm, device B reads a `profiles.json` with `folders[]` + `profiles[]` envelope format it doesn't understand (it expects a flat array).

**Options**:
- **A — Accept breakage**: Old builds see a deserialization error on new format. This is acceptable if the project is pre-1.0 and users can be advised to upgrade both devices simultaneously. This is likely acceptable for NexTerm v0.2.
- **B — Top-level schema_version field**: The envelope includes `"schema_version": 2`. Old Rust code tries to parse a flat array and fails on the object. Not great.
- **C — Dual format detection**: `load_profiles_from_disk` checks if root JSON is an array (old format) or object (new format). This is the cleanest approach.

**→ Recommended**: **Option C** — dual format detection at load time. `serde_json::Value` deserialization, check `is_array()` vs `is_object()`, branch accordingly. Existing flat-array files continue to work exactly as today. New envelope files use the object format. No separate schema_version field needed — the structure itself encodes the version.

### 3.7 Expand/Collapse State — Where to Persist

**Option A — Persisted in `profiles.json` (inside Folder entity)**

`Folder.is_expanded: bool` stored in the JSON file. User's last expand/collapse state survives app restarts.

- Pros: State is durable; consistent experience after restart; trivial to implement (just another field)
- Cons: Any expand/collapse triggers a disk write (though `save_profiles_to_disk` is already called on every mutation)
- Effort: Low

**Option B — Session-only (in React useState)**

Expand/collapse state lives in `useState` in the Sidebar component, reset on app restart.

- Pros: No disk writes for UI state; simpler
- Cons: Frustrating UX — user collapses 10 folders, restarts app, all expanded again
- Effort: Zero

**Option C — LocalStorage / Tauri store**

Persist in browser localStorage or a Tauri KV store, separate from profiles.json.

- Pros: No disk write on every toggle
- Cons: Two stores to sync; added complexity
- Effort: Medium

**→ Recommended**: **Option A** — persist `is_expanded` in the Folder entity in `profiles.json`. The file already gets written on every profile change; one extra field is negligible. Consistent with the "all state in profiles.json" principle.

### 3.8 Search Behavior — In-Folder vs Flatten

**Option A — Search flattens to list (ignores folder structure)**

When a search query is active, show a flat list of matching profiles (current behavior). Folder headers hidden.

- Pros: Consistent with current behavior; simple to implement; familiar (VS Code, most IDE sidebars)
- Cons: User loses folder context while searching
- Effort: Zero (current behavior preserved)

**Option B — Search keeps folder structure (in-folder filtering)**

Folders that have matches stay visible (expanded) with only matching profiles shown. Empty folders hidden.

- Pros: Better context awareness; user sees which folder a result belongs to
- Cons: More complex render logic; harder to implement with DnD context
- Effort: Medium

**Option C — Toggle between modes**

A button to switch between "flat search" and "in-folder search" modes.

- Pros: Covers both use cases
- Cons: Additional UX complexity; button adds noise to already-dense sidebar
- Effort: High, marginal value

**→ Recommended**: **Option A for MVP** — flatten search results. This matches current behavior and VS Code/Termius conventions. When search is active, the sidebar enters "search mode" showing a flat list with a small folder badge/label next to each result to show which folder it belongs to. This gives context without in-folder filtering complexity.

### 3.9 Profile Creation — Which Folder?

When the user clicks "+ Nuevo", which folder does the new profile land in?

**Option A — Always "Sin agrupar"**

New profiles always start in the system default folder. User must move them explicitly.

- Pros: Predictable; no UI needed for folder selection at creation time
- Cons: If the user is "in" a folder (looking at it), they expect new profiles to go there
- Effort: Zero

**Option B — Last selected/active folder**

Track which folder the user was last viewing; new profiles go there.

- Pros: Natural UX ("I'm in Proxmox folder, I click +, new profile goes to Proxmox")
- Cons: Adds state tracking ("active folder"); needs to survive profile list reload
- Effort: Medium

**Option C — Folder picker in profile creation dialog**

Add a "Folder" dropdown to the `ConnectionDialog` form.

- Pros: Explicit, no ambiguity
- Cons: Adds UI element to already-complex form; folder selection is a secondary concern at creation time
- Effort: Medium

**→ Recommended**: **Option B** — track selected/active folder in Sidebar state. When a folder header is clicked, mark it as "active" (visual selection highlight). New profile goes to the active folder (defaults to "Sin agrupar" if no folder is selected). This is the most natural pattern. The "active folder" is session-only state (not persisted).

### 3.10 Move Profile Between Folders

**Option A — Drag-and-drop (profile card → folder header)**

Extend the existing `@dnd-kit` setup to support cross-container drops (profile cards drop onto folder headers or into folder containers).

- Pros: Intuitive, modern UX
- Cons: `@dnd-kit` multi-container DnD is significantly more complex than single-list DnD. Requires `DragOverlay`, multiple `SortableContext` instances, `over.data.current.type` discriminant. The current single-list DnD in Sidebar.tsx would need a major rewrite. Cross-container + within-folder reorder is complex.
- Effort: High

**Option B — Context menu "Move to folder"**

Right-click (or kebab menu) → "Move to folder" → dropdown/submenu showing folder list.

- Pros: Much simpler to implement; no DnD complexity; works on touch/keyboard
- Cons: Less intuitive than drag-and-drop; extra clicks
- Effort: Low-Medium

**Option C — Folder dropdown in Edit dialog**

In the `ConnectionDialog` (Edit Profile), add a "Folder" dropdown. User edits profile → changes folder → saves.

- Pros: Consistent with existing edit flow; no new UI patterns
- Cons: Requires opening full edit dialog just to move a folder; poor discoverability
- Effort: Low

**→ Recommended**: **Option B for MVP** + hint toward Option A later. Context menu approach for v1, with an inline "move to folder" affordance (a small dropdown directly on the profile card's action row). The `@dnd-kit` cross-container DnD can be added in a follow-up change once the basic folder feature ships. This is aligned with the decision to defer DnD in MVP.

### 3.11 Delete Folder With Profiles Inside

**Option A — Block deletion if non-empty**

Show error: "This folder has {n} profiles. Move them first."

- Pros: Prevents accidental data loss
- Cons: Frustrating if user wants to bulk-delete a folder and its profiles
- Effort: Low

**Option B — Cascade-delete profiles inside folder**

When a folder is deleted, all profiles inside are also deleted (with confirmation dialog).

- Pros: Clean, no orphan profiles
- Cons: Potentially dangerous; users might not realize 10 profiles will be deleted
- Effort: Low-Medium

**Option C — Move profiles to "Sin agrupar" on folder deletion**

When a non-empty folder is deleted, its profiles are automatically moved to the default folder.

- Pros: Non-destructive; no data loss; consistent with "orphan profiles not allowed" constraint
- Cons: User might be surprised where their profiles went
- Effort: Low

**→ Recommended**: **Option C** — move-to-ungrouped on folder deletion. The confirmation dialog explicitly states: "Deleting '{folder_name}' will move {n} profiles to 'Sin agrupar'. Confirm?" This is the non-destructive, user-friendly choice.

### 3.12 Rename/Delete "Sin agrupar" System Folder

**Decision (non-negotiable)**: The system folder CANNOT be renamed or deleted. The UI must:
- Hide the rename/delete actions when `folder.is_system === true`
- If the user attempts via keyboard/programmatic means, the backend rejects with an error
- The system folder is always the last in visual order (or first — TBD in design phase)

### 3.13 Export/Import — Folder Data

**Option A — Export includes folder structure (envelope v3)**

`ExportEnvelope.version` bumped to `3`. Add `folders` array to envelope. Import reconstructs both folders and profiles.

- Pros: Full fidelity export; backup/restore preserves organization
- Cons: Bigger change to export format; older NexTerm imports would see `version: 3` and may reject; `ExportedProfile` needs `folder_id` or `folder_name` field
- Effort: Medium

**Option B — Export adds `folder_name: Option<String>` to `ExportedProfile` (Tabby-style)**

Each exported profile includes the folder name as a string. Import reconstructs folders by name, creates them if missing, assigns profiles.

- Pros: Human-readable JSON; portable to tools that don't understand NexTerm's UUIDs; version bump stays manageable
- Cons: Folder ordering lost; folder properties (future color/icon) lost; folder name is the key (rename sensitivity)
- Effort: Low-Medium

**Option C — Export excludes folders (current behavior preserved)**

Imported profiles all land in "Sin agrupar". User re-organizes after import.

- Pros: No change to export format; safest backward compat
- Cons: Poor UX for users who export to back up their organization
- Effort: Zero

**→ Recommended**: **Option B** — add `folder_name: Option<String>` to `ExportedProfile`. This is the most portable and readable approach. Export version bumps to `3` only to add this field; the envelope structure itself doesn't change (still `profiles[]` at root, no top-level `folders[]`). Folder ordering is a "nice to have" not a requirement for export.

### 3.14 New Tauri Command Surface

Commands needed (in addition to current profile commands):

| Command | Input | Output | Notes |
|---------|-------|--------|-------|
| `create_folder` | `name: String` | `Uuid` | Creates folder, returns new ID |
| `rename_folder` | `folder_id: Uuid, name: String` | `()` | Fails if `is_system` |
| `delete_folder` | `folder_id: Uuid` | `()` | Moves profiles to system folder; fails if `is_system` |
| `reorder_folders` | `folder_ids: Vec<Uuid>` | `()` | Same pattern as `reorder_profiles` |
| `move_profile_to_folder` | `profile_id: Uuid, folder_id: Uuid` | `()` | Updates `folder_id` on profile, saves |
| `load_folders` | — | `Vec<Folder>` | Can be bundled with `load_profiles` or separate |

**Decision point**: Should `load_profiles` return both profiles AND folders in one call (envelope), or should there be a separate `load_folders` command? Loading together in one call (new `load_profiles_with_folders` or modified `load_profiles` returning an envelope) avoids a round-trip race condition where the frontend has profiles but not yet folders. **Recommended**: return both in a single command via a new return type `ProfilesResponse { folders: Vec<Folder>, profiles: Vec<ConnectionProfile> }`.

### 3.15 Frontend Virtualization — DnD Impact

**Finding**: `@tanstack/react-virtual` is NOT used in the Sidebar. The current sidebar is a plain DOM list. At 15 profiles, there's no virtualization concern.

At 100+ profiles across 10+ folders, the flat profile list could become long, but this is a future concern. The transition to folder grouping actually REDUCES the visible list at any one time (collapsed folders). No virtualization action required in this change.

**DnD complexity**: The current `SortableContext` wraps one flat list. Adding folders introduces the challenge of intra-folder reorder AND cross-folder move. `@dnd-kit` supports this via multiple `SortableContext` instances with a top-level `DndContext`. However, since we're deferring cross-folder DnD to a later change, the MVP can have:
- One `DndContext` per folder for within-folder profile reorder
- No cross-folder drag (move via context menu instead)
- A separate `DndContext` for folder reorder (drag folder headers)

This is manageable and does not require a full DnD rewrite for MVP.

### 3.16 Empty Folders

**Decision**: Empty folders ARE allowed (user creates "Proxmox" folder before having any Proxmox profiles yet). An empty folder shows with a count badge of "0" and an "Empty" hint when expanded. The "Sin agrupar" system folder may also be empty if all profiles have been moved to other folders — this is allowed (it should show the empty hint, not be hidden).

---

## 4. Risks & Unknowns

### 4.1 `profiles.json` Format Change — Hidden Coupling

**Risk**: The current `load_profiles_from_disk` assumes the JSON root is a `Vec<ConnectionProfile>`. Any code that directly reads/writes `profiles.json` outside of this function (scripts, backups, manual edits) will break. This is a known, contained risk — the file is only managed by NexTerm code.

**More dangerous**: The `fs_secure::secure_write` and `profiles.backup.json` pattern assumes the content is a JSON array. Moving to an envelope object requires updating the type signature of `save_profiles_to_disk` or replacing it with a new `save_profiles_data_to_disk` that accepts the envelope type.

**Investigation needed**: Confirm there are no other read paths for `profiles.json` in the codebase.

### 4.2 Upstream PR Sizing

**Context**: This project is `CogniDevAI/nexterm` (upstream). Issue #1 (vault ACL hardening) is pending maintainer response. This feature (folder grouping) is substantially larger than the vault hardening change. The vault change was ~200 lines of new code. Folder grouping will be:
- Rust: ~150 lines (Folder struct, new commands, migration logic)
- TypeScript: ~300 lines (store additions, Sidebar folder rendering)
- i18n: ~20 new keys per language
- Tests: ~20 new Rust unit tests

**Total**: ~700-900 lines net new code. This is large for a first feature PR from an external contributor. 

**Recommendation**: Split into two PRs:
1. **PR A (data model + commands)**: Rust-only changes — `Folder` struct, `profiles.json` envelope format, migration, new Tauri commands, tests. This is purely backend and has no UI risk.
2. **PR B (UI + i18n)**: Frontend changes — Sidebar folder rendering, store additions, i18n keys. Depends on PR A.

This mirrors the two-PR approach that would make review easier for maintainers and reduces the risk of one large PR being rejected or sitting for months.

### 4.3 Performance at 50+/100+ Profiles

**Risk**: With 50+ profiles across 10+ folders, the sidebar renders O(profiles + folders) DOM nodes. Each profile card is a non-trivial component with DnD hooks.

**Current baseline**: At 15 profiles, no performance issues reported. `@dnd-kit`'s PointerSensor with `distance: 5` constraint prevents accidental drags. The `useSortable` hook adds a small overhead per item.

**At 100 profiles**: Without virtualization, 100 `SortableProfileCard` instances each with `useSortable` hooks is manageable (similar to most chat apps or VS Code file trees). `@dnd-kit` does not require virtualization to function correctly.

**Actual risk**: If a user has 200+ profiles and each has multiple sessions, the `profileSessionMap` `useMemo` could become expensive. This is a future concern — document it.

**Mitigation for MVP**: Profile cards in collapsed folders are still mounted in the DOM (just hidden with CSS). Consider whether collapsed folder contents should be conditionally rendered (null) instead of display-none. This reduces DOM size significantly.

### 4.4 Accessibility — Keyboard Navigation

**Risk**: The current sidebar has keyboard support for DnD via `KeyboardSensor` (space to lift, arrow keys to move, space/enter to drop). Adding folder structure complicates keyboard navigation — pressing arrow keys should ideally navigate between profiles AND into/out of folders.

**What currently works**: The `KeyboardSensor` from `@dnd-kit` handles drag-only keyboard interaction. Focus management between profile cards and folder headers is not explicitly handled today.

**What needs design**: 
- Tab order through folder headers and their profiles
- Arrow key navigation (folder up/down, Expand/collapse on right/left arrow)
- Keyboard shortcut to move a profile to a different folder (no DnD equivalent in MVP)

**Mitigation**: For MVP, ensure folder headers are focusable (`tabindex=0`), have proper ARIA roles (`role="group"`, `aria-expanded`), and profiles within folders have `role="treeitem"`. Full keyboard navigation parity with DnD can be a follow-up accessibility task.

### 4.5 `@dnd-kit` Multi-Container Interaction

**Risk**: The planned MVP approach (separate `SortableContext` per folder for within-folder reorder, defer cross-folder DnD) creates a scenario where dragging a profile card that is inside one `SortableContext` starts a drag that cannot cross into another `SortableContext`. This is correct behavior for MVP. The risk is user confusion when they try to drag a profile to another folder and it "bounces back."

**Mitigation**: The context menu "Move to folder" (Option B in §3.10) must be clearly discoverable to avoid user confusion. A tooltip on the drag handle: "Drag to reorder within folder. Use ··· to move between folders."

---

## 5. Recommended Direction

### 5.1 Data Model

**Pick Option C** (envelope format) with **Option B** for "Sin agrupar" (system folder with `is_system: true`).

Concretely: `profiles.json` becomes:
```json
{
  "folders": [
    { "id": "...", "name": "Sin agrupar", "displayOrder": 999, "isSystem": true, "isExpanded": true, ... },
    { "id": "...", "name": "Proxmox", "displayOrder": 0, "isSystem": false, "isExpanded": true, ... }
  ],
  "profiles": [
    { "id": "...", "folderId": "...", ... }
  ]
}
```

`load_profiles_from_disk` detects array vs object root (dual format detection). Old flat-array format silently migrates all profiles to the system folder.

### 5.2 Migration

**Automatic silent migration** (Option A in §3.5). On first load of the new version with old format:
1. Detect root JSON is a flat array (old format)
2. Create the system "Sin agrupar" folder with a generated UUID
3. Set `folder_id` on all profiles to that UUID
4. Save new envelope format
5. Create `profiles.backup.json` (same as legacy migration pattern)

Zero user interaction. User sees all 15 profiles under "Sin agrupar" on first launch, then organizes.

### 5.3 Search

**Flatten in search mode** (Option A in §3.8). When `searchQuery.trim()` is non-empty, render flat filtered list with a small folder badge on each profile card (e.g., `[Proxmox]` prefix in subtitle). This requires reading `folderId` → folder name for each result. No folder headers shown during search. When search is cleared, folder structure returns.

### 5.4 Drag-and-Drop for MVP

**DEFER cross-folder DnD**. MVP ships with:
- Within-folder profile reorder via DnD (one `SortableContext` per folder)
- Folder reorder via DnD (drag folder header rows)
- Cross-folder move via context menu/inline action

Rationale: Within-folder DnD + folder header DnD are well within the current `@dnd-kit` patterns. Cross-container DnD requires `DragOverlay`, multiple `DndContext` interactions, and complex `onDragOver` logic — it's a separate feature that can ship in a follow-up PR.

### 5.5 PR Strategy

**Two PRs** (as analyzed in §4.2):
- PR A: Rust data model + commands + TDD tests + migration (backend only, ~300 lines)
- PR B: Frontend UI — store additions + Sidebar folder rendering + i18n (frontend only, ~500 lines)

This is a better contribution story for the upstream maintainers at CogniDevAI.

---

## 6. Out-of-Scope for This Change (Explicit)

The following items MUST NOT be included in this change:

| Item | Reason for exclusion |
|------|---------------------|
| **Nested folders** (folders within folders) | Explicitly decided out by orchestrator — 1 level only |
| **Folder color customization** | `color` field deferred to future `Folder` entity extension |
| **Folder icon customization** | Same as above |
| **Tags** | Different paradigm (many-to-many vs one-to-one folder assignment) |
| **Smart/dynamic folders** (filter-based) | Different feature entirely |
| **Cross-folder DnD** | Deferred to follow-up change; context menu covers MVP |
| **Group-level SSH settings inheritance** | Termius-style defaults cascade — out of NexTerm's current scope |
| **Connect to all profiles in folder** | Batch connect feature — out of scope |
| **Folder sharing/sync** | Cloud sync feature — out of scope |
| **Folder import/export with full order+properties** | Export includes `folder_name` string only (§3.13, Option B) |

---

## 7. Affected Files Summary

| File | Change type | Impact |
|------|------------|--------|
| `src-tauri/src/profile.rs` | Modify | Add `folder_id: Option<Uuid>` to `ConnectionProfile`; new `Folder` struct; new `ProfilesData` envelope type; update `load_profiles_from_disk` + `save_profiles_to_disk` to handle envelope format; migration logic |
| `src-tauri/src/commands/profile.rs` | Modify | Add folder commands (`create_folder`, `rename_folder`, `delete_folder`, `reorder_folders`, `move_profile_to_folder`); update `load_profiles` to return `ProfilesResponse`; update `import_profiles` for folder_name field |
| `src-tauri/src/state.rs` | Modify | Add `folders: Mutex<Vec<Folder>>` to `AppState` |
| `src-tauri/src/lib.rs` | Modify | Register new folder commands in `invoke_handler!` |
| `src/lib/types.ts` | Modify | Add `ProfileFolder` interface; add `folderId?: string` to `ConnectionProfile`; add `ProfilesResponse` type |
| `src/stores/profileStore.ts` | Modify | Add `folders` state; add folder CRUD actions; update `loadProfiles` to load envelope |
| `src/components/layout/Sidebar.tsx` | Modify | Add folder headers, expand/collapse, profile grouping, search flatten mode, active folder tracking, move-to-folder context action |
| `src/lib/i18n/en.ts` + `es.ts` | Modify | Add ~20 new i18n keys for folder UI |

**New files**: None anticipated (Folder struct lives in `profile.rs`).

---

## 8. Implementation Notes (TDD Constraint)

Per project convention, all Rust changes MUST follow strict TDD:
- Write failing tests FIRST (RED phase)
- Implement to make tests pass (GREEN phase)
- Refactor (REFACTOR phase)

New tests needed in `src-tauri/src/profile.rs`:
- `folder_migration_moves_all_profiles_to_system_folder`
- `load_profiles_dual_format_detects_old_flat_array`
- `load_profiles_dual_format_reads_new_envelope`
- `folder_save_and_load_roundtrip`
- `delete_folder_moves_profiles_to_system_folder`
- `system_folder_cannot_be_deleted`
- `system_folder_cannot_be_renamed`
- `create_folder_assigns_display_order`

New tests in `src-tauri/src/commands/profile.rs`:
- `move_profile_to_folder_updates_folder_id`
- `delete_folder_with_profiles_moves_to_ungrouped`

The test runner command is: `cd src-tauri && cargo test`

---

## Ready for Proposal

Yes — the exploration is complete. The proposal phase should focus on:
1. Formalizing the `profiles.json` envelope format (v2 object format with `folders[]` + `profiles[]`)
2. The migration strategy (auto-silent + backup)
3. The MVP scope boundary (context menu for cross-folder move, DnD within folder only)
4. Two-PR upstream contribution strategy

---

*Generated by sdd-explore agent — anthropic/claude-sonnet-4-6 — 2026-04-19*
