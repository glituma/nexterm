# Proposal: profile-folder-grouping

**Date**: 2026-04-19
**Phase**: propose
**Source exploration**: `openspec/changes/profile-folder-grouping/explore.md` / engram `sdd/profile-folder-grouping/explore`

---

## 1. Why

The sidebar currently renders a flat list of `ConnectionProfile` rows sorted by `display_order`. At today's 15 profiles it already saturates the visible area, forcing vertical scrolling and visual scanning to find a specific host. The user's fleet is heterogeneous (Proxmox nodes, financial-core servers, lab boxes) and growth pressure from onboarding more servers makes the flat list untenable. The user explicitly asked: *"me gustaria poder agrupar en tipo carpetas por ejemplo todos mis servidores proxmox, en otra carpeta servidores del core financiero"*. Competitor clients (Termius, Tabby, Royal TSX, WindTerm) all solve this with named collections; NexTerm needs parity.

## 2. What Changes

User-visible capabilities introduced:

- Named folders (e.g., "Proxmox", "Core Financiero") group related profiles in the sidebar.
- Every folder shows a count badge and chevron expand/collapse (state persisted across restarts).
- A system folder **"Sin agrupar" / "Ungrouped"** always exists, cannot be renamed or deleted, and collects any profile without an explicit folder.
- Create / rename / delete folders via context menu + confirmation dialog. Deleting a non-empty folder moves its profiles to "Sin agrupar" (non-destructive).
- Move a profile between folders via a context menu dropdown on the profile card.
- Drag-and-drop reorder **within** a folder, and drag-and-drop reorder of folder headers themselves.
- Search flattens results across folders and shows a small folder badge per match.
- Existing 15 profiles migrate silently to "Sin agrupar" on first launch (backup created).

## 3. Scope — IN

- Rust `Folder` struct + `folder_id: Option<Uuid>` on `ConnectionProfile` (`src-tauri/src/profile.rs`).
- `profiles.json` envelope format `{ folders: [], profiles: [] }` with dual-format detection at load time.
- Automatic silent migration from legacy flat-array JSON → envelope + `profiles.backup.json`.
- New Tauri commands: `create_folder`, `rename_folder`, `delete_folder`, `reorder_folders`, `move_profile_to_folder`; `load_profiles` returns `ProfilesResponse { folders, profiles }`.
- `AppState.folders: Mutex<Vec<Folder>>`.
- Zustand store additions: `folders` state, folder CRUD actions, active-folder tracking.
- Sidebar render: folder headers, count badge, chevron expand/collapse, per-folder `SortableContext`, folder header `SortableContext`, move-to-folder context action, search flatten mode.
- EN + ES i18n keys (both files in lock-step per existing `Record<TranslationKey, string>` enforcement).
- Full TDD Rust test suite (migration, dual-format load, system folder invariants, folder lifecycle, move-profile command).

## 4. Scope — OUT

- Nested folders (>1 level).
- Folder colors / icons / descriptions.
- Tags (orthogonal many-to-many paradigm).
- Smart / dynamic / filter-based folders.
- Cross-folder drag-and-drop (deferred follow-up; MVP uses context menu).
- Folder sharing / cloud sync.
- Folder-level batch actions (e.g., "connect to all in folder").
- Group-level SSH setting inheritance (Termius-style cascade).
- Full-fidelity folder export (export adds `folder_name: Option<String>` only — folder order + properties not preserved).

## 5. Approach (High-Level)

Follows the exploration's **Recommended Direction** verbatim.

**Data model — Option C + Option B:** `profiles.json` becomes an envelope with peer `folders` and `profiles` arrays. "Sin agrupar" is a real `Folder` entity with `is_system: true`, not a magic UUID or virtual computed group. Backend uniformly handles all folders; UI hides rename/delete on system folder; backend rejects those ops server-side.

**Migration — dual-format detection:** `load_profiles_from_disk` deserializes to `serde_json::Value` and branches on `is_array()` (legacy) vs `is_object()` (envelope). Legacy path: create system folder, assign all profiles to it, write envelope, create `profiles.backup.json`. No schema version field — structural shape encodes the version. Zero user interaction.

**Command surface:** Keep existing commands untouched. Add folder CRUD + `move_profile_to_folder`. Modify `load_profiles` to return `ProfilesResponse` so folders and profiles arrive in a single IPC round-trip (no race condition).

**Ordering:** Reuse global `display_order` on profiles (Option A from §3.4). Moving a profile into a folder assigns `max_order + 1` within that folder. Folders get their own `display_order`. Keeps `reorder_profiles` unchanged.

**UI strategy:** One `DndContext` at sidebar root; per-folder `SortableContext` for within-folder reorder; separate `SortableContext` for folder header reorder. Cross-folder move uses a context-menu dropdown on the profile card's action row. Search mode renders a flat list with folder badges (not nested filter).

**Expand/collapse:** Persisted as `is_expanded: bool` on `Folder`, saved with profiles on every toggle.

**Active folder for profile creation:** Session-only Sidebar state; new profile lands in the currently highlighted folder (defaults to "Sin agrupar").

## 6. Upstream Contribution Strategy

Following the vault-hardening precedent (issue #1 → PR pattern), but split into **two sequential PRs** against `CogniDevAI/nexterm` linked to a NEW upstream issue (NOT #1):

- **PR A — Backend:** `Folder` struct, envelope format, dual-format load, migration, new commands, `AppState.folders`, `invoke_handler!` registration, full Rust TDD suite. ~300 LOC.
- **PR B — Frontend:** TS types, store additions, Sidebar folder render, move-to-folder UI, search flatten mode, i18n EN+ES. ~500 LOC. Depends on PR A merge.

Total ~700–900 LOC split in two reviewable chunks. A new issue will be filed upstream describing the feature and linking this SDD change folder for design transparency. Issue #1 (vault ACL hardening) remains untouched.

## 7. Open Questions for Maintainer

1. **Export format:** Is `ExportedProfile.folder_name: Option<String>` acceptable for envelope v3, or does the maintainer prefer the full `folders[]` array in the export envelope (higher fidelity, more coupling)?
2. **System folder position:** Should "Sin agrupar" render **first** (default home) or **last** (archive-like) in the sidebar? Exploration leaves this to design phase; a maintainer preference would lock it earlier.
3. **New capability spec name:** Prefer `profile-organization` or `profile-management` for the new `openspec/specs/<name>/spec.md`?

## 8. Capabilities (Contract with sdd-spec)

### New Capabilities

- `profile-organization`: folder CRUD, system "Sin agrupar" folder invariants, folder ordering, profile ↔ folder assignment, dual-format `profiles.json` load/save, expand-collapse persistence, search flatten behavior.

### Modified Capabilities

- None. The `vault-storage-security` spec is unaffected (credentials storage is orthogonal to profile grouping).

## 9. Success Criteria

- [ ] User creates a folder named "Proxmox" via the sidebar UI; it appears as a collapsible header with count `(0)`.
- [ ] User opens the context menu on 3 existing profiles, selects "Move to folder → Proxmox"; profiles appear under the Proxmox header with count `(3)`.
- [ ] User restarts the app; folders, profile assignments, and expand/collapse state all persist.
- [ ] On first launch after upgrading from a legacy NexTerm install (flat-array `profiles.json`), all existing profiles appear under "Sin agrupar" and `profiles.backup.json` is created with the old content.
- [ ] User deletes a non-empty folder; profiles move to "Sin agrupar", dialog confirmed the move count before deletion.
- [ ] System folder "Sin agrupar" shows no rename / delete option in its context menu, and backend rejects `rename_folder` / `delete_folder` on an `is_system: true` folder with a non-panic error.
- [ ] Within-folder drag-and-drop reorder persists `display_order` across restart.
- [ ] Drag-and-drop reorder of folder headers persists folder `display_order` across restart.
- [ ] Active search query renders a flat list with `[Folder Name]` badges per result; clearing search restores folder structure.
- [ ] All new i18n strings exist in both `en.ts` and `es.ts` (TypeScript compile passes).
- [ ] `cd src-tauri && cargo test` passes green, including new tests: migration, dual-format load, system folder invariants, folder lifecycle, move-profile command.

## 10. Risks (Top 3)

1. **`profiles.json` format hidden coupling** — any code path that reads `profiles.json` outside `load_profiles_from_disk` will break on the envelope format. Mitigation: grep the codebase for direct reads; route everything through the function.
2. **Upstream PR sizing / maintainer bandwidth** — ~900 LOC split across two PRs is still large for an external contributor; maintainer may request further splits or sit on the review. Mitigation: ship PR A in isolation (pure backend, fully tested, zero UI risk) and use the merge signal before opening PR B.
3. **`@dnd-kit` user confusion** — MVP allows DnD within a folder but NOT across folders; users will try to drag a card to another folder and watch it bounce back. Mitigation: tooltip on drag handle ("Drag to reorder within folder. Use ··· to move between folders.") + discoverable context-menu move action.

## 11. Rollback Plan

If either PR is rejected upstream or regressions surface after local merge:

1. **Code rollback:** revert the two PR commits (they're isolated to `src-tauri/src/profile.rs`, `src-tauri/src/commands/profile.rs`, `src-tauri/src/state.rs`, `src-tauri/src/lib.rs`, `src/lib/types.ts`, `src/stores/profileStore.ts`, `src/components/layout/Sidebar.tsx`, `src/lib/i18n/{en,es}.ts`).
2. **Data rollback:** The migration creates `profiles.backup.json` with the pre-migration flat-array content. Instruct affected users to rename `profiles.backup.json` → `profiles.json` (overwrite). The rolled-back binary will then read the legacy format unchanged.
3. **No data loss:** No profile data is ever dropped by the migration — only reshaped. Folder entities that existed only in the envelope format are lost on rollback, which is acceptable (the user had folders for days/hours, not months).
4. **Partial rollback option:** If PR A is merged but PR B is reverted, the backend envelope format stays and the frontend continues to read the `profiles` array ignoring `folders`. All profiles effectively behave as "Sin agrupar". Functional but less pretty.

---

*Generated by sdd-propose — anthropic/claude-opus-4-7 — 2026-04-19*
