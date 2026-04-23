// lib/tauri.ts — Typed Tauri invoke wrapper and error handling
//
// All IPC calls to the Rust backend should go through tauriInvoke<T>()
// for consistent error handling and type safety.

import { invoke } from "@tauri-apps/api/core";
import type {
  Folder,
  ProfilesEnvelope,
  DeleteFolderResult,
} from "./types";

// ─── AppError ───────────────────────────────────────────

export class AppError extends Error {
  public readonly command: string;

  constructor(command: string, message: string) {
    super(message);
    this.name = "AppError";
    this.command = command;
  }

  get isAuthFailed(): boolean {
    return this.message.includes("Authentication failed");
  }

  get isNotConnected(): boolean {
    return this.message.includes("Not connected");
  }

  get isTimeout(): boolean {
    return this.message.includes("timeout");
  }

  get isSessionNotFound(): boolean {
    return this.message.includes("Session not found");
  }

  get isHostKeyRejected(): boolean {
    return this.message.includes("Host key verification failed");
  }

  get isKeyError(): boolean {
    return this.message.includes("Key error");
  }

  get isKeychainError(): boolean {
    return this.message.includes("Keychain error");
  }

  get isVaultLocked(): boolean {
    return this.message.includes("Vault is locked");
  }

  get isVaultError(): boolean {
    return this.message.includes("Vault error");
  }

  get isPermissionDenied(): boolean {
    return this.message.includes("Permission denied");
  }
}

// ─── Typed Invoke Wrapper ───────────────────────────────

export async function tauriInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (error) {
    throw new AppError(cmd, error as string);
  }
}

// ─── Folder + Profile Envelope Commands ─────────────────
// Typed wrappers for the Phase 4 backend commands.
// Rust param names use snake_case; Tauri serialises them to camelCase on the
// wire — the invoke arg keys here must match the camelCase of the Rust names.

/** Load the full folder + profile tree. Use this on startup instead of the
 *  legacy `load_profiles` command. */
export async function loadProfilesWithFolders(): Promise<ProfilesEnvelope> {
  return tauriInvoke<ProfilesEnvelope>("load_profiles_with_folders");
}

/** Create a new user folder with the given name. Returns the created Folder. */
export async function createFolder(name: string): Promise<Folder> {
  return tauriInvoke<Folder>("create_folder", { name });
}

/** Rename an existing folder. Returns the updated Folder. */
export async function renameFolder(
  folderId: string,
  newName: string,
): Promise<Folder> {
  return tauriInvoke<Folder>("rename_folder", { folderId, newName });
}

/** Delete a folder. Profiles inside are moved to the system folder.
 *  Returns the count of profiles that were moved. */
export async function deleteFolder(
  folderId: string,
): Promise<DeleteFolderResult> {
  return tauriInvoke<DeleteFolderResult>("delete_folder", { folderId });
}

/** Reorder all folders. `orderedIds` must contain every current folder UUID. */
export async function reorderFolders(orderedIds: string[]): Promise<void> {
  return tauriInvoke<void>("reorder_folders", { orderedIds });
}

/** Move a profile to a different folder (or reorder within the same folder). */
export async function moveProfileToFolder(
  profileId: string,
  targetFolderId: string,
  newOrder: number,
): Promise<void> {
  return tauriInvoke<void>("move_profile_to_folder", {
    profileId,
    targetFolderId,
    newOrder,
  });
}

/** Reorder all profiles within a specific folder. */
export async function reorderProfilesInFolder(
  folderId: string,
  orderedProfileIds: string[],
): Promise<void> {
  return tauriInvoke<void>("reorder_profiles_in_folder", {
    folderId,
    orderedProfileIds,
  });
}

/** Persist the expanded/collapsed state of a folder. */
export async function setFolderExpanded(
  folderId: string,
  expanded: boolean,
): Promise<void> {
  return tauriInvoke<void>("set_folder_expanded", { folderId, expanded });
}
