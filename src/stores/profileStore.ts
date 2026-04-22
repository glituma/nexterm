// stores/profileStore.ts — Zustand store for connection profiles + folder CRUD
//
// Actions call Tauri backend commands via tauriInvoke wrapper.
// Folder actions follow a pessimistic pattern (create/rename/delete) or
// optimistic pattern (reorder*, toggleFolderExpanded) per design §5.

import { create } from "zustand";
import type { ConnectionProfile, Folder, ProfilesEnvelope, DeleteFolderResult } from "../lib/types";
import { tauriInvoke } from "../lib/tauri";
import {
  loadProfilesWithFolders,
  createFolder as tauriCreateFolder,
  renameFolder as tauriRenameFolder,
  deleteFolder as tauriDeleteFolder,
  reorderFolders as tauriReorderFolders,
  moveProfileToFolder as tauriMoveProfileToFolder,
  reorderProfilesInFolder as tauriReorderProfilesInFolder,
  setFolderExpanded as tauriSetFolderExpanded,
} from "../lib/tauri";

export interface ImportResult {
  imported: number;
  skipped: number;
  errors: string[];
}

/// Result returned by the `export_profiles` Tauri command.
/// `count` is the number of profiles written.
/// `warnings` carries stable string identifiers (NOT translation keys) that the
/// frontend maps to localised messages. Current values: `"acl_not_applied"`.
export interface ExportResult {
  count: number;
  warnings: string[];
}

// ─── Debounce helper ────────────────────────────────────

/** Returns a debounced version of fn with the given delay (ms). */
function debounce<T extends unknown[]>(
  fn: (...args: T) => void,
  delay: number,
): (...args: T) => void {
  let timer: ReturnType<typeof setTimeout> | null = null;
  return (...args: T) => {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      fn(...args);
      timer = null;
    }, delay);
  };
}

// ─── Store State Interface ───────────────────────────────

interface ProfileStoreState {
  // ── Profiles ──────────────────────────────────────────
  profiles: ConnectionProfile[];
  loading: boolean;
  error: string | null;

  // ── Folders ───────────────────────────────────────────
  folders: Folder[];
  /** Local UI state — which folders are currently expanded.
   *  Initialised from folder.isExpanded on loadAll(). Mirrored to backend
   *  via setFolderExpanded (debounced). */
  expandedFolderIds: Set<string>;

  // ── Profile actions (existing) ────────────────────────
  /** @deprecated Use loadAll() — kept for backward compat with Sidebar.tsx */
  loadProfiles: () => Promise<void>;
  loadAll: () => Promise<void>;
  saveProfile: (profile: ConnectionProfile) => Promise<string>;
  deleteProfile: (id: string) => Promise<void>;
  storeCredential: (profileId: string, userId: string, password: string) => Promise<void>;
  reorderProfiles: (ids: string[]) => Promise<void>;
  exportProfiles: (exportPath: string, includeCredentials: boolean, exportPassword?: string) => Promise<ExportResult>;
  importProfiles: (importPath: string, importPassword?: string) => Promise<ImportResult>;
  clearError: () => void;

  // ── Folder actions (new Phase 6) ──────────────────────
  createFolder: (name: string) => Promise<Folder>;
  renameFolder: (folderId: string, newName: string) => Promise<void>;
  deleteFolder: (folderId: string) => Promise<DeleteFolderResult>;
  reorderFolders: (orderedIds: string[]) => Promise<void>;
  moveProfileToFolder: (profileId: string, targetFolderId: string, newOrder: number) => Promise<void>;
  reorderProfilesInFolder: (folderId: string, orderedProfileIds: string[]) => Promise<void>;
  toggleFolderExpanded: (folderId: string) => Promise<void>;
}

// ─── Debounced setFolderExpanded (module-level — survives re-renders) ───────
// 300 ms: if user toggles same folder rapidly, only the last call hits backend.
const debouncedSetFolderExpanded = debounce(
  (folderId: string, expanded: boolean) => {
    void tauriSetFolderExpanded(folderId, expanded);
  },
  300,
);

// ─── Store ──────────────────────────────────────────────

export const useProfileStore = create<ProfileStoreState>((set, get) => ({
  profiles: [],
  folders: [],
  expandedFolderIds: new Set<string>(),
  loading: false,
  error: null,

  // ── loadAll ─────────────────────────────────────────────────────────────────
  // Primary entry point — replaces legacy loadProfiles().
  // Calls load_profiles_with_folders, sets both profiles and folders, and
  // initialises expandedFolderIds from the persisted isExpanded field.
  loadAll: async () => {
    set({ loading: true, error: null });
    try {
      const envelope: ProfilesEnvelope = await loadProfilesWithFolders();
      const expandedFolderIds = new Set<string>(
        envelope.folders
          .filter((f) => f.isExpanded)
          .map((f) => f.id),
      );
      set({
        profiles: envelope.profiles,
        folders: envelope.folders,
        expandedFolderIds,
        loading: false,
      });
    } catch (err) {
      set({ loading: false, error: String(err) });
    }
  },

  // ── loadProfiles (compat shim — delegates to loadAll) ───────────────────────
  loadProfiles: async () => {
    return get().loadAll();
  },

  // ── saveProfile ──────────────────────────────────────────────────────────────
  saveProfile: async (profile: ConnectionProfile) => {
    set({ error: null });
    try {
      const id = await tauriInvoke<string>("save_profile", {
        profileData: profile,
      });
      // Reload full envelope to sync state (folder assignments may change)
      await get().loadAll();
      return id;
    } catch (err) {
      set({ error: String(err) });
      throw err;
    }
  },

  // ── deleteProfile ─────────────────────────────────────────────────────────────
  deleteProfile: async (id: string) => {
    set({ error: null });
    try {
      await tauriInvoke<void>("delete_profile", { profileId: id });
      // Reload full envelope to sync state
      await get().loadAll();
    } catch (err) {
      set({ error: String(err) });
      throw err;
    }
  },

  // ── storeCredential ──────────────────────────────────────────────────────────
  storeCredential: async (profileId: string, userId: string, password: string) => {
    try {
      await tauriInvoke<void>("store_credential", {
        profileId,
        userId,
        credentialType: "password",
        value: password,
      });
    } catch (err) {
      // Non-blocking — log but don't prevent connection
      console.error("Failed to store credential:", err);
      set({ error: `Vault error: ${String(err)}` });
    }
  },

  // ── reorderProfiles ───────────────────────────────────────────────────────────
  // Optimistic: reorder locally first, rollback on backend failure.
  reorderProfiles: async (ids: string[]) => {
    set({ error: null });
    try {
      // Optimistic update — reorder locally first
      set((state) => {
        const profileMap = new Map(state.profiles.map((p) => [p.id, p]));
        const reordered = ids
          .map((id) => profileMap.get(id))
          .filter((p): p is ConnectionProfile => p !== undefined);
        return { profiles: reordered };
      });
      await tauriInvoke<void>("reorder_profiles", { profileIds: ids });
    } catch (err) {
      set({ error: String(err) });
      // Reload from backend on error to restore correct order
      try {
        await get().loadAll();
      } catch { /* ignore reload error */ }
    }
  },

  // ── exportProfiles ────────────────────────────────────────────────────────────
  exportProfiles: async (exportPath: string, includeCredentials: boolean, exportPassword?: string) => {
    set({ error: null });
    try {
      return await tauriInvoke<ExportResult>("export_profiles", {
        exportPath,
        includeCredentials,
        exportPassword: exportPassword ?? null,
      });
    } catch (err) {
      set({ error: String(err) });
      throw err;
    }
  },

  // ── importProfiles ────────────────────────────────────────────────────────────
  importProfiles: async (importPath: string, importPassword?: string) => {
    set({ error: null });
    try {
      const result = await tauriInvoke<ImportResult>("import_profiles", {
        importPath,
        importPassword: importPassword ?? null,
      });
      // Reload full envelope to sync state after import
      await get().loadAll();
      return result;
    } catch (err) {
      set({ error: String(err) });
      throw err;
    }
  },

  // ── clearError ────────────────────────────────────────────────────────────────
  clearError: () => set({ error: null }),

  // ─────────────────────────────────────────────────────────────────────────────
  // Folder actions
  // ─────────────────────────────────────────────────────────────────────────────

  // ── createFolder — PESSIMISTIC ──────────────────────────────────────────────
  createFolder: async (name: string) => {
    set({ error: null });
    try {
      const folder = await tauriCreateFolder(name);
      // Backend is source of truth — reload full envelope
      await get().loadAll();
      return folder;
    } catch (err) {
      set({ error: String(err) });
      throw err;
    }
  },

  // ── renameFolder — PESSIMISTIC ──────────────────────────────────────────────
  renameFolder: async (folderId: string, newName: string) => {
    set({ error: null });
    try {
      await tauriRenameFolder(folderId, newName);
      await get().loadAll();
    } catch (err) {
      set({ error: String(err) });
      throw err;
    }
  },

  // ── deleteFolder — PESSIMISTIC ──────────────────────────────────────────────
  deleteFolder: async (folderId: string) => {
    set({ error: null });
    try {
      const result = await tauriDeleteFolder(folderId);
      // Reload to get updated profile folder assignments
      await get().loadAll();
      return result;
    } catch (err) {
      set({ error: String(err) });
      throw err;
    }
  },

  // ── reorderFolders — OPTIMISTIC ──────────────────────────────────────────────
  reorderFolders: async (orderedIds: string[]) => {
    set({ error: null });

    // Snapshot for rollback
    const snapshot = get().folders;

    // Optimistic update: reorder local folders array by the given id order
    set((state) => {
      const folderMap = new Map(state.folders.map((f) => [f.id, f]));
      const reordered = orderedIds
        .map((id) => folderMap.get(id))
        .filter((f): f is Folder => f !== undefined);
      // Assign display_order to match new position
      const updated = reordered.map((f, i) => ({ ...f, displayOrder: i }));
      return { folders: updated };
    });

    try {
      await tauriReorderFolders(orderedIds);
    } catch (err) {
      // Rollback
      set({ folders: snapshot, error: String(err) });
      throw err;
    }
  },

  // ── moveProfileToFolder — PESSIMISTIC (refetch after) ─────────────────────
  moveProfileToFolder: async (
    profileId: string,
    targetFolderId: string,
    newOrder: number,
  ) => {
    set({ error: null });
    try {
      await tauriMoveProfileToFolder(profileId, targetFolderId, newOrder);
      // Refetch canonical state — simpler than mirroring Rust logic locally
      await get().loadAll();
    } catch (err) {
      set({ error: String(err) });
      throw err;
    }
  },

  // ── reorderProfilesInFolder — OPTIMISTIC ──────────────────────────────────
  reorderProfilesInFolder: async (
    folderId: string,
    orderedProfileIds: string[],
  ) => {
    set({ error: null });

    // Snapshot for rollback
    const snapshot = get().profiles;

    // Optimistic update: reorder profiles that belong to this folder
    set((state) => {
      const profileMap = new Map(state.profiles.map((p) => [p.id, p]));
      const reorderedInFolder = orderedProfileIds
        .map((id) => profileMap.get(id))
        .filter((p): p is ConnectionProfile => p !== undefined)
        .map((p, i) => ({ ...p, displayOrder: i }));

      // Merge back: keep profiles from other folders, replace this folder's profiles
      const otherProfiles = state.profiles.filter(
        (p) => p.folderId !== folderId,
      );
      return { profiles: [...otherProfiles, ...reorderedInFolder] };
    });

    try {
      await tauriReorderProfilesInFolder(folderId, orderedProfileIds);
    } catch (err) {
      // Rollback
      set({ profiles: snapshot, error: String(err) });
      throw err;
    }
  },

  // ── toggleFolderExpanded — OPTIMISTIC (debounced backend persist) ──────────
  toggleFolderExpanded: async (folderId: string) => {
    // Compute new expanded state from current Set
    const current = get().expandedFolderIds;
    const isCurrentlyExpanded = current.has(folderId);
    const expanded = !isCurrentlyExpanded;

    // Optimistic update — no rollback (expand state is UI-only, minor inconsistency ok)
    set((state) => {
      const next = new Set(state.expandedFolderIds);
      if (expanded) {
        next.add(folderId);
      } else {
        next.delete(folderId);
      }
      return { expandedFolderIds: next };
    });

    // Persist to backend (debounced — only last call in 300ms window hits Rust)
    debouncedSetFolderExpanded(folderId, expanded);
  },
}));

// ─── Selectors ──────────────────────────────────────────
// Exported as plain functions (not inside the store) so components can use
// them with shallow-equal selectors or useMemo for memoisation.

/** Map from folderId → profiles in that folder, sorted by displayOrder. */
export function profilesByFolder(
  state: Pick<ProfileStoreState, "profiles">,
): Map<string, ConnectionProfile[]> {
  const map = new Map<string, ConnectionProfile[]>();
  for (const profile of state.profiles) {
    const key = profile.folderId ?? "__system__";
    const bucket = map.get(key) ?? [];
    bucket.push(profile);
    map.set(key, bucket);
  }
  // Sort each bucket by displayOrder
  for (const [key, bucket] of map) {
    map.set(
      key,
      bucket.slice().sort((a, b) => (a.displayOrder ?? 0) - (b.displayOrder ?? 0)),
    );
  }
  return map;
}

/** Folders sorted by displayOrder. */
export function sortedFolders(
  state: Pick<ProfileStoreState, "folders">,
): Folder[] {
  return state.folders
    .slice()
    .sort((a, b) => a.displayOrder - b.displayOrder);
}

/** The system (ungrouped) folder, or undefined if not yet loaded. */
export function systemFolder(
  state: Pick<ProfileStoreState, "folders">,
): Folder | undefined {
  return state.folders.find((f) => f.isSystem);
}
