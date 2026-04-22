// lib/folders.ts — Folder display helpers and constants
//
// Centralises the system-folder detection logic so it is never duplicated
// across components. The SYSTEM_FOLDER_MARKER string must match the Rust
// constant `SYSTEM_FOLDER_NAME = "__system__"` in src-tauri/src/profile.rs.

import type { Folder } from "./types";

// ─── Constants ──────────────────────────────────────────

/** Raw name written to disk for the system (ungrouped) folder. */
export const SYSTEM_FOLDER_MARKER = "__system__";

// ─── Helpers ────────────────────────────────────────────

/**
 * Returns true if the folder is the system "ungrouped" folder.
 * Prefer checking `folder.isSystem` (backend-authoritative) over comparing
 * the name string — this helper encapsulates both checks for safety.
 */
export function isSystemFolder(folder: Folder): boolean {
  return folder.isSystem;
}

/**
 * Human-readable display name for a folder.
 *
 * For the system folder the i18n key `sidebar.folders.ungroupedName` is used
 * (will be wired up in Phase 8). For user-created folders the raw name is
 * returned as-is.
 *
 * @param folder  The folder to get the display name for.
 * @param t       i18n translation function — pass `t` from `useI18n()`.
 */
export function displayFolderName(
  folder: Folder,
  t: (key: string) => string,
): string {
  if (folder.isSystem) return t("sidebar.folders.ungroupedName");
  return folder.name;
}
