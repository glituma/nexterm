# profile-organization Specification

## Purpose

Defines how NexTerm organizes `ConnectionProfile` entries into named, user-managed folders in the sidebar. Covers folder lifecycle (create/rename/delete/reorder), the system "Sin agrupar" folder invariants, profile-to-folder assignment, persistence of the envelope format on disk, expand/collapse persistence, search behavior across folders, and backward-compatible migration from the legacy flat-array `profiles.json`.

## Requirements

### Requirement: R1 — Every profile belongs to exactly one folder

Every `ConnectionProfile` MUST be assigned to exactly one folder. The system MUST NOT persist any profile with a null, missing, or dangling folder reference in any serialized state (`profiles.json`, IPC payload, or exported envelope).

#### Scenario: Newly created profile has a folder assignment

- GIVEN the user creates a new profile
- WHEN the profile is persisted to disk
- THEN the persisted record has a non-null `folder_id` pointing to an existing folder

#### Scenario: Loading a profile with an unknown folder_id reassigns it

- GIVEN a `profiles.json` contains a profile whose `folder_id` does not match any folder in the envelope
- WHEN the app loads profiles
- THEN the profile is reassigned to the system "Sin agrupar" folder
- AND the corrected envelope is written back to disk

### Requirement: R2 — System folder "Sin agrupar" invariants

The system folder "Sin agrupar" (Ungrouped) MUST exist at all times, MUST be flagged `is_system: true`, MUST NOT be renameable, and MUST NOT be deletable by any means (UI, IPC command, or external JSON edit tolerated at load time by re-creating it).

#### Scenario: Rename command rejects system folder

- GIVEN the system folder "Sin agrupar" exists
- WHEN the `rename_folder` command is invoked with its id
- THEN the command returns an error
- AND the folder name on disk is unchanged

#### Scenario: Delete command rejects system folder

- GIVEN the system folder "Sin agrupar" exists
- WHEN the `delete_folder` command is invoked with its id
- THEN the command returns an error
- AND the folder still exists on disk after the call

#### Scenario: System folder auto-recreated when missing

- GIVEN `profiles.json` is deleted or contains no folder with `is_system: true`
- WHEN the app loads profiles
- THEN the system folder "Sin agrupar" is created in-memory
- AND the repaired envelope is written back to disk on next save

### Requirement: R3 — Create folder via UI

Users MUST be able to create a new folder through the sidebar UI by supplying a non-empty name (trimmed). Empty, whitespace-only, or `None`-equivalent names MUST be rejected.

#### Scenario: Create folder with valid name

- GIVEN the sidebar is visible
- WHEN the user creates a folder named "Proxmox"
- THEN a new folder with name "Proxmox", `is_system: false`, and a unique id appears in the sidebar and in persisted state

#### Scenario: Reject empty folder name

- GIVEN the user opens the create-folder input
- WHEN the user submits an empty or whitespace-only name
- THEN no folder is created
- AND the UI surfaces a validation error

### Requirement: R4 — Rename non-system folder

Users MUST be able to rename any non-system folder to a new non-empty name.

#### Scenario: Rename a user folder

- GIVEN a user folder named "Proxmox" exists
- WHEN the user renames it to "Proxmox Prod"
- THEN the folder name is "Proxmox Prod" in the sidebar and in persisted state
- AND all profiles inside retain their assignment

### Requirement: R5 — Delete non-system folder moves profiles atomically

Users MUST be able to delete any non-system folder. Deletion MUST atomically move all profiles contained in that folder to the system "Sin agrupar" folder. A failure during the move MUST NOT leave profiles pointing at a deleted folder.

#### Scenario: Delete non-empty folder moves profiles

- GIVEN folder "Proxmox" contains 3 profiles
- WHEN the user confirms deletion of "Proxmox"
- THEN folder "Proxmox" no longer exists
- AND all 3 profiles now have `folder_id` equal to the system folder id
- AND "Sin agrupar" count badge reflects the 3 added profiles

#### Scenario: Delete empty folder

- GIVEN folder "Lab" contains 0 profiles
- WHEN the user confirms deletion of "Lab"
- THEN folder "Lab" no longer exists
- AND no profile's `folder_id` changes

### Requirement: R6 — Move profile between folders

Users MUST be able to move a profile from one folder to any other folder (user or system) via the profile's context menu.

#### Scenario: Move profile via context menu

- GIVEN profile "prod-db-1" is in folder "Sin agrupar"
- WHEN the user selects "Move to folder → Core Financiero" from its context menu
- THEN "prod-db-1" appears under "Core Financiero"
- AND its `folder_id` in persisted state equals the id of "Core Financiero"

### Requirement: R7 — Reorder profiles within a folder via drag & drop

Users MUST be able to reorder profiles inside a single folder by drag & drop. The new `display_order` MUST persist across restarts. Drag operations MUST NOT move a profile across folder boundaries (MVP scope).

#### Scenario: Reorder two profiles in the same folder

- GIVEN folder "Proxmox" contains profiles [A, B, C] in that order
- WHEN the user drags C above A
- THEN the sidebar shows [C, A, B] under "Proxmox"
- AND after a restart the order is still [C, A, B]

### Requirement: R8 — Reorder folder headers via drag & drop

Users MUST be able to reorder folder headers themselves by drag & drop. New folder `display_order` MUST persist across restarts. The system folder MAY participate in reorder but cannot be removed from the list.

#### Scenario: Reorder folders

- GIVEN folders are displayed in the order [Sin agrupar, Proxmox, Core Financiero]
- WHEN the user drags "Core Financiero" above "Proxmox"
- THEN the sidebar shows [Sin agrupar, Core Financiero, Proxmox]
- AND the order survives an app restart

### Requirement: R9 — Migrate legacy flat-array profiles.json

On first load after upgrade, when `profiles.json` is a top-level JSON array, the system MUST migrate it to the envelope format `{ folders: [...], profiles: [...] }`, create the system folder "Sin agrupar", assign every existing profile to it, and write the migrated envelope back to disk. A pre-migration copy MUST be written to `profiles.backup.json`. Migration MUST be silent (no user interaction).

#### Scenario: Legacy flat-array migrates to envelope

- GIVEN `profiles.json` is a JSON array containing 15 profiles without `folder_id`
- WHEN the app loads profiles for the first time after upgrade
- THEN `profiles.json` is now an object with keys `folders` and `profiles`
- AND exactly one folder exists with `is_system: true` and name "Sin agrupar"
- AND all 15 profiles have `folder_id` equal to that folder's id
- AND `profiles.backup.json` exists containing the original flat array

#### Scenario: Already-envelope file is not re-migrated

- GIVEN `profiles.json` is already an object with `folders` and `profiles` keys
- WHEN the app loads profiles
- THEN no backup file is created
- AND the on-disk content is byte-identical to before (modulo unrelated writes)

### Requirement: R10 — Folder expand/collapse persistence

Each folder's expand/collapse state MUST persist across application restarts. Toggling expand/collapse in the UI MUST write the new state to disk.

#### Scenario: Collapsed folder reopens collapsed

- GIVEN folder "Proxmox" is expanded
- WHEN the user collapses it and then restarts the app
- THEN "Proxmox" is rendered collapsed on next launch

### Requirement: R11 — Sidebar search across folders

When the user types a query into the sidebar search, the system MUST filter profiles across all folders. Folders with zero matching profiles MUST be hidden from the result view; folders with at least one match MUST remain visible and show only their matching profiles. Clearing the query MUST restore the full folder structure.

#### Scenario: Single match inside one folder

- GIVEN folder "Proxmox" contains ["node-01", "node-02"] and folder "Core Financiero" contains ["core-db"]
- WHEN the user searches for "node-01"
- THEN only folder "Proxmox" is visible
- AND only profile "node-01" is rendered under it
- AND folder "Core Financiero" is not rendered

#### Scenario: Matches across multiple folders

- GIVEN folder "Proxmox" contains "prod-api" and folder "Core Financiero" contains "prod-db"
- WHEN the user searches for "prod"
- THEN both folders are visible
- AND each folder shows only its matching profile
- AND clearing the query restores all folders and all profiles

### Requirement: R12 — Export includes folder assignment

Profile export MUST include the folder assignment for each exported profile such that an import on another NexTerm instance can reconstruct the same logical grouping. The exact serialization shape is defined in design.md.

#### Scenario: Exported profile carries folder assignment

- GIVEN a profile "prod-db-1" in folder "Core Financiero"
- WHEN the user exports profiles
- THEN the exported record for "prod-db-1" contains its folder assignment (by name or id, per design)
- AND importing that file on a fresh instance places "prod-db-1" into a folder named "Core Financiero"

### Requirement: R13 — New profile lands in active folder

Creating a new profile via the "+ Nuevo" action MUST place the profile into the currently active (focused/highlighted) folder in the sidebar, or into "Sin agrupar" when no folder is active.

#### Scenario: New profile inherits active folder

- GIVEN the user has clicked the header of folder "Proxmox" making it active
- WHEN the user creates a new profile
- THEN the new profile's `folder_id` equals the id of "Proxmox"

#### Scenario: No active folder defaults to Sin agrupar

- GIVEN no folder is currently active in the sidebar
- WHEN the user creates a new profile
- THEN the new profile's `folder_id` equals the id of the system folder "Sin agrupar"

### Requirement: R14 — Empty folder empty-state

A folder containing zero profiles MUST render a visible empty-state hint inside its expanded body (e.g., localized "No profiles in this folder").

#### Scenario: Empty folder shows hint when expanded

- GIVEN folder "Lab" contains 0 profiles and is expanded
- WHEN the sidebar renders
- THEN an empty-state hint is displayed inside the folder body
- AND the folder count badge reads `(0)`

### Requirement: R15 — Folder operations are atomic

Folder operations (create, rename, delete, reorder, move-profile) MUST be atomic with respect to on-disk state. A failure during any operation MUST NOT leave the persisted envelope in an inconsistent state (e.g., dangling `folder_id`, duplicate ids, lost profiles).

#### Scenario: Failed delete leaves state unchanged

- GIVEN folder "Proxmox" contains 3 profiles
- WHEN the `delete_folder` command fails mid-way (e.g., disk write error)
- THEN folder "Proxmox" still exists
- AND all 3 profiles still have `folder_id` equal to "Proxmox"
- AND no profile is orphaned

#### Scenario: Failed rename does not corrupt name

- GIVEN folder "Proxmox" exists
- WHEN the `rename_folder` command fails during persistence
- THEN the folder name in memory and on disk is either the original "Proxmox" or the new name — never a partial or empty value
