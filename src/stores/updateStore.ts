// stores/updateStore.ts — Zustand store for auto-update state
//
// Manages update lifecycle: check, available, download progress, error, dismiss.
// Used by useUpdater hook and UpdateDialog/CriticalUpdateScreen components.

import { create } from "zustand";

export type UpdateStatus =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "installing"
  | "error"
  | "dismissed";

export interface UpdateInfo {
  version: string;
  body: string; // Release notes markdown
  date: string; // ISO date
}

export interface UpdateProgress {
  downloaded: number; // bytes downloaded so far
  total: number | null; // total bytes (null if unknown)
  percentage: number; // 0-100
}

interface UpdateStoreState {
  status: UpdateStatus;
  updateInfo: UpdateInfo | null;
  isCritical: boolean;
  progress: UpdateProgress | null;
  error: string | null;

  // Actions
  setStatus: (status: UpdateStatus) => void;
  setUpdateInfo: (info: UpdateInfo, critical: boolean) => void;
  setProgress: (downloaded: number, total: number | null) => void;
  setError: (error: string) => void;
  dismiss: () => void;
  reset: () => void;
}

export const useUpdateStore = create<UpdateStoreState>((set) => ({
  status: "idle",
  updateInfo: null,
  isCritical: false,
  progress: null,
  error: null,

  setStatus: (status) => set({ status }),

  setUpdateInfo: (info, critical) =>
    set({
      status: "available",
      updateInfo: info,
      isCritical: critical,
      error: null,
    }),

  setProgress: (downloaded, total) =>
    set({
      status: "downloading",
      progress: {
        downloaded,
        total,
        percentage: total != null && total > 0
          ? Math.round((downloaded / total) * 100)
          : 0,
      },
      error: null,
    }),

  setError: (error) => set({ status: "error", error }),

  dismiss: () => set({ status: "dismissed" }),

  reset: () =>
    set({
      status: "idle",
      updateInfo: null,
      isCritical: false,
      progress: null,
      error: null,
    }),
}));
