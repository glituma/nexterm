// components/layout/Sidebar.tsx — Left sidebar with folder-grouped profiles
//
// Phase 7: Full folder grouping UI with DnD, context menus, CRUD modals.
// All folder display uses displayFolderName() — NEVER raw folder.name for system folder.

import { useEffect, useMemo, useState, useCallback, useRef } from "react";
import { save, open } from "@tauri-apps/plugin-dialog";
import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  verticalListSortingStrategy,
  useSortable,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useProfileStore, sortedFolders, profilesByFolder } from "../../stores/profileStore";
import { useShallow } from "zustand/react/shallow";
import {
  useSessionStore,
  type SessionEntry,
} from "../../stores/sessionStore";
import { useI18n } from "../../lib/i18n";
import { Dialog } from "../ui/Dialog";
import { displayFolderName } from "../../lib/folders";
import type { ConnectionProfile, Folder } from "../../lib/types";
import type { TranslationKey } from "../../lib/i18n";

// ─── Props ────────────────────────────────────────────────
interface SidebarProps {
  onConnect: (profileId: string, userId?: string) => void;
  onDisconnect: (sessionId: string) => void;
  onNewProfile: () => void;
  onEditProfile: (profileId: string) => void;
  connectingProfileId: string | null;
  connectError: string | null;
  onClearError: () => void;
}

// ─── Helpers ──────────────────────────────────────────────

function SessionStateIndicator({ state }: { state: SessionEntry["state"] }) {
  if (state === "connected") return <span className="indicator indicator-success" />;
  if (state === "connecting" || state === "authenticating")
    return <span className="indicator indicator-warning" />;
  if (state === "disconnected") return <span className="indicator indicator-muted" />;
  return <span className="indicator indicator-error" />;
}

function getSessionStateKey(state: SessionEntry["state"]): TranslationKey {
  if (state === "connected") return "session.connected";
  if (state === "connecting") return "session.connecting";
  if (state === "authenticating") return "session.authenticating";
  if (state === "disconnected") return "session.disconnected";
  return "session.error";
}

/** Returns plural form of profile count using two plain keys. */
function profileCountLabel(
  count: number,
  t: (key: TranslationKey, params?: Record<string, string | number>) => string,
): string {
  const key: TranslationKey = count === 1
    ? "sidebar.folders.profileCount_one"
    : "sidebar.folders.profileCount_other";
  return t(key, { count });
}

// ─── Context Menu (generic floating menu) ────────────────

interface ContextMenuPos { x: number; y: number }

interface ContextMenuProps {
  pos: ContextMenuPos;
  onClose: () => void;
  children: React.ReactNode;
}

function FloatingContextMenu({ pos, onClose, children }: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    const handleClick = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener("keydown", handleKey);
    document.addEventListener("mousedown", handleClick);
    return () => {
      document.removeEventListener("keydown", handleKey);
      document.removeEventListener("mousedown", handleClick);
    };
  }, [onClose]);

  // Adjust so menu doesn't overflow screen
  useEffect(() => {
    if (!menuRef.current) return;
    const rect = menuRef.current.getBoundingClientRect();
    if (rect.right > window.innerWidth) {
      menuRef.current.style.left = `${pos.x - rect.width}px`;
    }
    if (rect.bottom > window.innerHeight) {
      menuRef.current.style.top = `${pos.y - rect.height}px`;
    }
  }, [pos.x, pos.y]);

  return (
    <div
      ref={menuRef}
      className="sftp-context-menu"
      style={{ left: pos.x, top: pos.y, position: "fixed", zIndex: 1000 }}
      role="menu"
    >
      {children}
    </div>
  );
}

// ─── Sortable Profile Card ───────────────────────────────

interface SortableProfileCardProps {
  profile: ConnectionProfile;
  connected: boolean;
  connecting: boolean;
  hasActiveSessions: boolean;
  isExpanded: boolean;
  profileSessions?: SessionEntry[];
  activeSessionId: string | null;
  statusClass: string;
  folders: Folder[];
  onProfileClick: (id: string) => void;
  onConnect: (id: string, userId?: string) => void;
  onEditProfile: (id: string) => void;
  onDeleteClick: (id: string, name: string) => void;
  onSetActiveSession: (id: string) => void;
  onDisconnect: (id: string) => void;
  onMoveToFolder: (profileId: string, targetFolderId: string, targetCount: number) => void;
  t: (key: TranslationKey, params?: Record<string, string | number>) => string;
  connectingLabel: string;
  connectLabel: string;
  /** If true, DnD is disabled (search mode) */
  dndDisabled?: boolean;
}

function SortableProfileCard({
  profile: p,
  connected,
  connecting,
  hasActiveSessions,
  isExpanded,
  profileSessions,
  activeSessionId,
  statusClass,
  folders,
  onProfileClick,
  onConnect,
  onEditProfile,
  onDeleteClick,
  onSetActiveSession,
  onDisconnect,
  onMoveToFolder,
  t,
  connectingLabel,
  connectLabel,
  dndDisabled = false,
}: SortableProfileCardProps) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: `profile:${p.id}`, disabled: dndDisabled });

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : undefined,
    zIndex: isDragging ? 10 : undefined,
  };

  const isSingleUser = p.users.length <= 1;
  const defaultUser = p.users.find((u) => u.isDefault) ?? p.users[0];
  const subtitle = isSingleUser && defaultUser
    ? `${defaultUser.username || "?"}@${p.host}:${p.port}`
    : `${p.users.length} users · ${p.host}:${p.port}`;

  const [pickerOpen, setPickerOpen] = useState(false);
  const pickerRef = useRef<HTMLDivElement>(null);

  // Context menu state for "Move to folder"
  const [ctxMenu, setCtxMenu] = useState<ContextMenuPos | null>(null);

  useEffect(() => {
    if (!pickerOpen) return;
    function handleClickOutside(e: MouseEvent) {
      if (pickerRef.current && !pickerRef.current.contains(e.target as Node)) {
        setPickerOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [pickerOpen]);

  const connectedUserIds = new Set(
    (profileSessions ?? [])
      .filter((s) => s.state === "connected" || s.state === "connecting" || s.state === "authenticating")
      .map((s) => s.userId)
      .filter(Boolean)
  );

  function handleConnectClick() {
    if (isSingleUser) {
      onConnect(p.id);
    } else {
      setPickerOpen((prev) => !prev);
    }
  }

  function handlePickUser(userId: string) {
    setPickerOpen(false);
    onConnect(p.id, userId);
  }

  function handleContextMenu(e: React.MouseEvent) {
    e.preventDefault();
    setCtxMenu({ x: e.clientX, y: e.clientY });
  }

  // Folders other than the current profile's folder
  const otherFolders = folders.filter((f) => f.id !== p.folderId);

  return (
    <div ref={setNodeRef} style={style} onContextMenu={handleContextMenu}>
      {/* Profile card */}
      <div
        className={`sidebar-profile-card ${connected ? "sidebar-profile-card-connected" : ""}`}
        onClick={() => onProfileClick(p.id)}
        title={subtitle}
      >
        {/* Drag handle */}
        {!dndDisabled && (
          <div
            className="sidebar-drag-handle"
            aria-label={t("sidebar.folders.dragHandle")}
            title={t("sidebar.folders.dragHandleTooltip")}
            {...attributes}
            {...listeners}
          >
            <span className="sidebar-drag-dots" />
          </div>
        )}

        <div className="sidebar-profile-card-left">
          <span className={`sidebar-chevron sidebar-profile-chevron ${isExpanded ? "" : "sidebar-chevron-collapsed"}`}>{"\u25BC"}</span>
          <span className={`sidebar-status-dot ${statusClass}`} />
          <div className="sidebar-profile-text">
            <div className="sidebar-profile-name">{p.name}</div>
            <div className="sidebar-profile-host">{subtitle}</div>
          </div>
        </div>
        <div
          className="sidebar-profile-card-actions"
          onClick={(e) => e.stopPropagation()}
        >
          <div className="sidebar-connect-wrapper" ref={pickerRef}>
            <button
              className={`sidebar-profile-btn sidebar-profile-btn-connect ${connected ? "sidebar-profile-btn-add" : ""}`}
              onClick={handleConnectClick}
              disabled={connecting}
              title={connecting ? connectingLabel : connected ? t("sidebar.newSession") : connectLabel}
            >
              {connecting ? (
                <span className="sidebar-btn-spinner" />
              ) : connected ? (
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                  <line x1="12" y1="5" x2="12" y2="19" />
                  <line x1="5" y1="12" x2="19" y2="12" />
                </svg>
              ) : (
                <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
                  <polygon points="5,3 19,12 5,21" />
                </svg>
              )}
            </button>
            {/* User picker popover (multi-user profiles) */}
            {pickerOpen && !isSingleUser && (
              <div className="sidebar-user-picker">
                {p.users.map((u) => {
                  const isUserConnected = connectedUserIds.has(u.id);
                  return (
                    <button
                      key={u.id}
                      className={`sidebar-user-picker-item ${isUserConnected ? "sidebar-user-picker-item-connected" : ""}`}
                      onClick={() => handlePickUser(u.id)}
                    >
                      <span className="sidebar-user-picker-name">{u.username || "?"}</span>
                      <span className="sidebar-user-picker-auth">
                        {u.authMethod.type === "publicKey" ? "\uD83D\uDD11" : "\uD83D\uDD12"}
                      </span>
                      {isUserConnected && (
                        <span className="sidebar-user-picker-connected-dot" />
                      )}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
          <button
            className="sidebar-profile-btn"
            onClick={() => onEditProfile(p.id)}
            title={t("sidebar.edit")}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" />
            </svg>
          </button>
          <button
            className="sidebar-profile-btn sidebar-profile-btn-delete"
            onClick={() => onDeleteClick(p.id, p.name)}
            title={t("sidebar.delete")}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </div>
      </div>

      {/* Nested sessions under profile (visible when expanded) */}
      {isExpanded && hasActiveSessions && profileSessions && (
        <div className="sidebar-nested-sessions">
          {profileSessions
            .filter(
              (s) =>
                s.state === "connected" ||
                s.state === "connecting" ||
                s.state === "authenticating"
            )
            .map((s) => (
              <div
                key={s.id}
                className={`sidebar-nested-session ${s.id === activeSessionId ? "sidebar-nested-session-active" : ""}`}
                onClick={() => onSetActiveSession(s.id)}
              >
                <div className="sidebar-nested-session-left">
                  <SessionStateIndicator state={s.state} />
                  <div className="sidebar-nested-session-info">
                    <div className="sidebar-nested-session-host">
                      {s.username}@{s.host}
                    </div>
                    <div className="sidebar-nested-session-state">
                      {t(getSessionStateKey(s.state))}
                    </div>
                  </div>
                </div>
                <button
                  className="sidebar-nested-disconnect-btn"
                  onClick={(e) => {
                    e.stopPropagation();
                    onDisconnect(s.id);
                  }}
                  title={t("sidebar.disconnect")}
                >
                  {"\u23FB"}
                </button>
              </div>
            ))}
        </div>
      )}

      {/* Context menu — "Move to folder" */}
      {ctxMenu && (
        <FloatingContextMenu pos={ctxMenu} onClose={() => setCtxMenu(null)}>
          {otherFolders.length > 0 && (
            <>
              <div className="sftp-context-separator" style={{ margin: "0", padding: "4px 12px", fontSize: "11px", color: "var(--text-muted, #888)", fontWeight: 600, textTransform: "uppercase", letterSpacing: "0.05em" }}>
                {t("sidebar.folders.moveToSubmenu")}
              </div>
              {otherFolders.map((folder) => (
                <button
                  key={folder.id}
                  className="sftp-context-item"
                  role="menuitem"
                  onClick={(e) => {
                    e.stopPropagation();
                    setCtxMenu(null);
                    // We pass the target folder's current profile count so backend appends at end
                    onMoveToFolder(p.id, folder.id, 9999);
                  }}
                >
                  {displayFolderName(folder, t as (key: string) => string)}
                </button>
              ))}
              <div className="sftp-context-separator" />
            </>
          )}
          <button
            className="sftp-context-item"
            role="menuitem"
            onClick={(e) => {
              e.stopPropagation();
              setCtxMenu(null);
              onEditProfile(p.id);
            }}
          >
            {t("sidebar.edit")}
          </button>
          <button
            className="sftp-context-item sftp-context-danger"
            role="menuitem"
            onClick={(e) => {
              e.stopPropagation();
              setCtxMenu(null);
              onDeleteClick(p.id, p.name);
            }}
          >
            {t("sidebar.delete")}
          </button>
        </FloatingContextMenu>
      )}
    </div>
  );
}

// ─── Sortable Folder Row ──────────────────────────────────

interface FolderRowProps {
  folder: Folder;
  profiles: ConnectionProfile[];
  isExpanded: boolean;
  isSearching: boolean;
  connectingProfileId: string | null;
  sessionMap: Map<string, SessionEntry[]>;
  activeSessionId: string | null;
  expandedProfileIds: Set<string>;
  allFolders: Folder[];
  onToggleExpand: (folderId: string) => void;
  onToggleProfileExpand: (profileId: string) => void;
  onConnect: (profileId: string, userId?: string) => void;
  onEditProfile: (profileId: string) => void;
  onDeleteProfileClick: (profileId: string, profileName: string) => void;
  onSetActiveSession: (id: string) => void;
  onDisconnect: (sessionId: string) => void;
  onMoveToFolder: (profileId: string, targetFolderId: string, targetCount: number) => void;
  onRenameFolder: (folder: Folder) => void;
  onDeleteFolder: (folder: Folder) => void;
  t: (key: TranslationKey, params?: Record<string, string | number>) => string;
  dndDisabled: boolean;
}

function FolderRow({
  folder,
  profiles,
  isExpanded,
  isSearching,
  connectingProfileId,
  sessionMap,
  activeSessionId,
  expandedProfileIds,
  allFolders,
  onToggleExpand,
  onToggleProfileExpand,
  onConnect,
  onEditProfile,
  onDeleteProfileClick,
  onSetActiveSession,
  onDisconnect,
  onMoveToFolder,
  onRenameFolder,
  onDeleteFolder,
  t,
  dndDisabled,
}: FolderRowProps) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: `folder:${folder.id}`, disabled: dndDisabled });

  const folderStyle = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : undefined,
  };

  // Folder ⋯ menu state
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const menuBtnRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    function handleClickOutside(e: MouseEvent) {
      if (
        menuRef.current && !menuRef.current.contains(e.target as Node) &&
        menuBtnRef.current && !menuBtnRef.current.contains(e.target as Node)
      ) {
        setMenuOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [menuOpen]);

  const displayName = displayFolderName(folder, t as (key: string) => string);
  const count = profiles.length;
  const effectiveExpanded = isSearching || isExpanded;

  function isProfileConnected(profileId: string): boolean {
    const sess = sessionMap.get(profileId);
    return sess?.some((s) => s.state === "connected") ?? false;
  }

  function isProfileConnecting(profileId: string): boolean {
    return connectingProfileId === profileId;
  }

  function getProfileStatusClass(profileId: string): string {
    if (isProfileConnected(profileId)) return "sidebar-status-dot-connected";
    if (isProfileConnecting(profileId)) return "sidebar-status-dot-connecting";
    return "sidebar-status-dot-idle";
  }

  return (
    <div ref={setNodeRef} style={folderStyle} className="sidebar-folder-group">
      {/* Folder header */}
      <div
        className="sidebar-folder-header"
        role="treeitem"
        aria-expanded={effectiveExpanded}
        aria-label={displayName}
        onClick={() => onToggleExpand(folder.id)}
        style={{ cursor: "pointer" }}
      >
        {/* Drag handle */}
        {!dndDisabled && (
          <div
            className="sidebar-drag-handle sidebar-folder-drag-handle"
            aria-label={t("sidebar.folders.dragHandle")}
            onClick={(e) => e.stopPropagation()}
            {...attributes}
            {...listeners}
          >
            <span className="sidebar-drag-dots" />
          </div>
        )}

        {/* Chevron */}
        <span
          className="sidebar-folder-chevron"
          style={{
            display: "inline-block",
            transition: "transform 150ms ease",
            transform: effectiveExpanded ? "rotate(0deg)" : "rotate(-90deg)",
            marginRight: "4px",
            fontSize: "10px",
            color: "var(--text-muted, #888)",
            cursor: "pointer",
          }}
          onClick={(e) => {
            e.stopPropagation();
            onToggleExpand(folder.id);
          }}
        >
          {"\u25BC"}
        </span>

        {/* Folder name */}
        <span className="sidebar-folder-name" style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {displayName}
        </span>

        {/* Profile count badge */}
        <span className="sidebar-section-badge" style={{ marginRight: "4px", fontSize: "11px" }}>
          {profileCountLabel(count, t)}
        </span>

        {/* ⋯ menu button — non-system folders show rename+delete, all show it */}
        {!folder.isSystem && (
          <div style={{ position: "relative" }}>
            <button
              ref={menuBtnRef}
              className="sidebar-profile-btn"
              title={t("sidebar.folders.rename")}
              onClick={(e) => {
                e.stopPropagation();
                setMenuOpen((prev) => !prev);
              }}
              style={{ fontSize: "14px", fontWeight: 700, letterSpacing: "1px" }}
            >
              {"⋯"}
            </button>
            {menuOpen && (
              <div
                ref={menuRef}
                className="sftp-context-menu"
                style={{ position: "absolute", top: "100%", right: 0, zIndex: 1000, minWidth: "140px" }}
                role="menu"
              >
                <button
                  className="sftp-context-item"
                  role="menuitem"
                  onClick={(e) => {
                    e.stopPropagation();
                    setMenuOpen(false);
                    onRenameFolder(folder);
                  }}
                >
                  {t("sidebar.folders.rename")}
                </button>
                <button
                  className="sftp-context-item sftp-context-danger"
                  role="menuitem"
                  onClick={(e) => {
                    e.stopPropagation();
                    setMenuOpen(false);
                    onDeleteFolder(folder);
                  }}
                >
                  {t("sidebar.folders.delete")}
                </button>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Folder contents */}
      {effectiveExpanded && (
        <div role="group" className="sidebar-folder-content">
          {profiles.length === 0 && !isSearching && (
            <div className="sidebar-empty" style={{ padding: "6px 12px 6px 24px", fontSize: "12px", color: "var(--text-muted, #888)", fontStyle: "italic" }}>
              {t("sidebar.folders.emptyHint")}
            </div>
          )}
          {profiles.length > 0 && (
            <SortableContext
              items={profiles.map((p) => `profile:${p.id}`)}
              strategy={verticalListSortingStrategy}
              disabled={dndDisabled}
            >
              {profiles.map((p) => {
                const connected = isProfileConnected(p.id);
                const connecting = isProfileConnecting(p.id);
                const profileSessions = sessionMap.get(p.id);
                const hasActiveSessions =
                  profileSessions?.some(
                    (s) => s.state === "connected" || s.state === "connecting" || s.state === "authenticating"
                  ) ?? false;

                return (
                  <SortableProfileCard
                    key={p.id}
                    profile={p}
                    connected={connected}
                    connecting={connecting}
                    hasActiveSessions={hasActiveSessions}
                    isExpanded={expandedProfileIds.has(p.id)}
                    profileSessions={profileSessions}
                    activeSessionId={activeSessionId}
                    statusClass={getProfileStatusClass(p.id)}
                    folders={allFolders}
                    onProfileClick={onToggleProfileExpand}
                    onConnect={onConnect}
                    onEditProfile={onEditProfile}
                    onDeleteClick={onDeleteProfileClick}
                    onSetActiveSession={onSetActiveSession}
                    onDisconnect={onDisconnect}
                    onMoveToFolder={onMoveToFolder}
                    t={t}
                    connectingLabel={t("sidebar.connecting")}
                    connectLabel={t("sidebar.connect")}
                    dndDisabled={dndDisabled}
                  />
                );
              })}
            </SortableContext>
          )}
        </div>
      )}
    </div>
  );
}

// ─── Create Folder Dialog ─────────────────────────────────

interface CreateFolderDialogProps {
  open: boolean;
  onClose: () => void;
  onConfirm: (name: string) => Promise<void>;
  existingNames: string[];
  t: (key: TranslationKey, params?: Record<string, string | number>) => string;
}

function CreateFolderDialog({ open, onClose, onConfirm, existingNames, t }: CreateFolderDialogProps) {
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  // Reset on open
  useEffect(() => {
    if (open) {
      setName("");
      setError(null);
      setLoading(false);
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [open]);

  function validate(val: string): string | null {
    const trimmed = val.trim();
    if (trimmed.length === 0 || trimmed.length > 64) return t("sidebar.folders.invalidName");
    if (existingNames.some((n) => n.toLowerCase() === trimmed.toLowerCase())) {
      return t("sidebar.folders.duplicateName");
    }
    return null;
  }

  async function handleSubmit() {
    const trimmed = name.trim();
    const err = validate(name);
    if (err) { setError(err); return; }
    setLoading(true);
    setError(null);
    try {
      await onConfirm(trimmed);
      onClose();
    } catch (e) {
      const msg = String(e);
      if (msg.toLowerCase().includes("duplicate") || msg.toLowerCase().includes("already")) {
        setError(t("sidebar.folders.duplicateName"));
      } else if (msg.toLowerCase().includes("invalid") || msg.toLowerCase().includes("name")) {
        setError(t("sidebar.folders.invalidName"));
      } else {
        setError(t("sidebar.folders.errorGeneric"));
      }
    } finally {
      setLoading(false);
    }
  }

  return (
    <Dialog open={open} onClose={onClose} title="" width="400px">
      <div className="cd-header">
        <div className="cd-header-text">
          <h3 className="cd-title">{t("sidebar.folders.createFolder")}</h3>
        </div>
      </div>
      <div className="cd-section-content">
        <div className="input-group">
          <input
            ref={inputRef}
            className={`input ${error ? "input-error" : ""}`}
            type="text"
            placeholder={t("sidebar.folders.createFolderPlaceholder")}
            value={name}
            onChange={(e) => { setName(e.target.value); setError(null); }}
            onKeyDown={(e) => { if (e.key === "Enter") void handleSubmit(); if (e.key === "Escape") onClose(); }}
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            data-form-type="other"
            data-lpignore="true"
            maxLength={64}
            disabled={loading}
          />
          {error && <span className="input-error-text">{error}</span>}
        </div>
      </div>
      <div className="cd-actions">
        <button className="btn btn-ghost btn-md" onClick={onClose} disabled={loading}>
          {t("sidebar.folders.cancel")}
        </button>
        <button className="btn btn-primary btn-md" onClick={() => void handleSubmit()} disabled={loading || !name.trim()}>
          {loading ? t("general.loading") : t("sidebar.folders.create")}
        </button>
      </div>
    </Dialog>
  );
}

// ─── Rename Folder Dialog ─────────────────────────────────

interface RenameFolderDialogProps {
  open: boolean;
  folder: Folder | null;
  onClose: () => void;
  onConfirm: (folderId: string, newName: string) => Promise<void>;
  existingNames: string[];
  t: (key: TranslationKey, params?: Record<string, string | number>) => string;
}

function RenameFolderDialog({ open, folder, onClose, onConfirm, existingNames, t }: RenameFolderDialogProps) {
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open && folder) {
      setName(folder.name);
      setError(null);
      setLoading(false);
      setTimeout(() => { inputRef.current?.focus(); inputRef.current?.select(); }, 50);
    }
  }, [open, folder]);

  function validate(val: string): string | null {
    const trimmed = val.trim();
    if (trimmed.length === 0 || trimmed.length > 64) return t("sidebar.folders.invalidName");
    // Exclude self from duplicate check
    if (existingNames.filter((n) => n !== folder?.name).some((n) => n.toLowerCase() === trimmed.toLowerCase())) {
      return t("sidebar.folders.duplicateName");
    }
    return null;
  }

  async function handleSubmit() {
    if (!folder) return;
    const trimmed = name.trim();
    const err = validate(name);
    if (err) { setError(err); return; }
    setLoading(true);
    setError(null);
    try {
      await onConfirm(folder.id, trimmed);
      onClose();
    } catch (e) {
      const msg = String(e);
      if (msg.toLowerCase().includes("duplicate") || msg.toLowerCase().includes("already")) {
        setError(t("sidebar.folders.duplicateName"));
      } else if (msg.toLowerCase().includes("system") || msg.toLowerCase().includes("protected")) {
        setError(t("sidebar.folders.systemProtected"));
      } else {
        setError(t("sidebar.folders.errorGeneric"));
      }
    } finally {
      setLoading(false);
    }
  }

  return (
    <Dialog open={open} onClose={onClose} title="" width="400px">
      <div className="cd-header">
        <div className="cd-header-text">
          <h3 className="cd-title">{t("sidebar.folders.renameFolder")}</h3>
        </div>
      </div>
      <div className="cd-section-content">
        <div className="input-group">
          <input
            ref={inputRef}
            className={`input ${error ? "input-error" : ""}`}
            type="text"
            value={name}
            onChange={(e) => { setName(e.target.value); setError(null); }}
            onKeyDown={(e) => { if (e.key === "Enter") void handleSubmit(); if (e.key === "Escape") onClose(); }}
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            data-form-type="other"
            data-lpignore="true"
            maxLength={64}
            disabled={loading}
          />
          {error && <span className="input-error-text">{error}</span>}
        </div>
      </div>
      <div className="cd-actions">
        <button className="btn btn-ghost btn-md" onClick={onClose} disabled={loading}>
          {t("sidebar.folders.cancel")}
        </button>
        <button className="btn btn-primary btn-md" onClick={() => void handleSubmit()} disabled={loading || !name.trim()}>
          {loading ? t("general.loading") : t("sidebar.folders.save")}
        </button>
      </div>
    </Dialog>
  );
}

// ─── Delete Folder Dialog ─────────────────────────────────

interface DeleteFolderDialogProps {
  open: boolean;
  folder: Folder | null;
  profileCount: number;
  onClose: () => void;
  onConfirm: (folderId: string) => Promise<void>;
  t: (key: TranslationKey, params?: Record<string, string | number>) => string;
}

function DeleteFolderDialog({ open, folder, profileCount, onClose, onConfirm, t }: DeleteFolderDialogProps) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (open) { setLoading(false); setError(null); }
  }, [open]);

  async function handleConfirm() {
    if (!folder) return;
    setLoading(true);
    setError(null);
    try {
      await onConfirm(folder.id);
      onClose();
    } catch (e) {
      setError(t("sidebar.folders.errorGeneric"));
    } finally {
      setLoading(false);
    }
  }

  return (
    <Dialog open={open} onClose={onClose} title="" width="400px">
      {folder && (
        <>
          <div className="delete-confirm-header">
            <div className="delete-confirm-icon">{"\u26A0"}</div>
            <div className="delete-confirm-text">
              <h3 className="delete-confirm-title">{t("sidebar.folders.deleteConfirmTitle")}</h3>
              <p className="delete-confirm-message">
                {profileCount > 0
                  ? t("sidebar.folders.deleteConfirmBody")
                  : t("sidebar.folders.deleteConfirmEmpty")}
              </p>
              {error && <p className="cd-error-message">{error}</p>}
            </div>
          </div>
          <div className="delete-confirm-actions">
            <button className="btn btn-ghost btn-md" onClick={onClose} disabled={loading}>
              {t("sidebar.folders.cancel")}
            </button>
            <button className="btn btn-danger btn-md" onClick={() => void handleConfirm()} disabled={loading}>
              {loading ? t("general.loading") : t("sidebar.folders.delete")}
            </button>
          </div>
        </>
      )}
    </Dialog>
  );
}

// ─── Component ────────────────────────────────────────────

export function Sidebar({
  onConnect,
  onDisconnect,
  onNewProfile,
  onEditProfile,
  connectingProfileId,
  connectError,
  onClearError,
}: SidebarProps) {
  const { t } = useI18n();

  // Subscribe to store with useShallow for stable object references on flat fields
  const {
    loading,
    loadAll,
    deleteProfile,
    exportProfiles,
    importProfiles,
    createFolder,
    renameFolder,
    deleteFolder,
    reorderFolders,
    moveProfileToFolder,
    reorderProfilesInFolder,
    toggleFolderExpanded,
  } = useProfileStore(
    useShallow((s) => ({
      loading: s.loading,
      loadAll: s.loadAll,
      deleteProfile: s.deleteProfile,
      exportProfiles: s.exportProfiles,
      importProfiles: s.importProfiles,
      createFolder: s.createFolder,
      renameFolder: s.renameFolder,
      deleteFolder: s.deleteFolder,
      reorderFolders: s.reorderFolders,
      moveProfileToFolder: s.moveProfileToFolder,
      reorderProfilesInFolder: s.reorderProfilesInFolder,
      toggleFolderExpanded: s.toggleFolderExpanded,
    })),
  );

  // Subscribe to raw store slices (stable references) and derive with useMemo.
  // Selectors like sortedFolders / profilesByFolder allocate new arrays/maps on
  // every call, so passing them directly to useProfileStore causes the
  // "getSnapshot should be cached" infinite-loop in React 19. Computing them
  // via useMemo downstream sidesteps that while keeping the same data shape.
  const rawFolders = useProfileStore((s) => s.folders);
  const allProfiles = useProfileStore((s) => s.profiles);
  const expandedFolderIds = useProfileStore((s) => s.expandedFolderIds);

  const folders = useMemo(() => sortedFolders({ folders: rawFolders }), [rawFolders]);
  const profileMap = useMemo(
    () => profilesByFolder({ profiles: allProfiles }),
    [allProfiles],
  );

  const { sessions, activeSessionId, setActiveSession } = useSessionStore();

  const [searchQuery, setSearchQuery] = useState("");
  const isSearching = searchQuery.trim().length > 0;

  // Per-profile expanded sessions state (chevron toggle per profile card)
  const [expandedProfileIds, setExpandedProfileIds] = useState<Set<string>>(new Set());

  // Delete profile confirmation dialog state
  const [deleteProfileConfirm, setDeleteProfileConfirm] = useState<{
    profileId: string;
    profileName: string;
    hasActiveSession: boolean;
  } | null>(null);
  const [deleteProfileLoading, setDeleteProfileLoading] = useState(false);

  // Export/import feedback banner
  const [banner, setBanner] = useState<{ type: "success" | "error"; message: string } | null>(null);

  // Export dialog state
  const [exportDialog, setExportDialog] = useState(false);
  const [exportIncludePasswords, setExportIncludePasswords] = useState(false);
  const [exportPassword, setExportPassword] = useState("");
  const [exportPasswordConfirm, setExportPasswordConfirm] = useState("");
  const [exportError, setExportError] = useState<string | null>(null);
  const [exportLoading, setExportLoading] = useState(false);

  // Import password dialog state
  const [importPasswordDialog, setImportPasswordDialog] = useState<string | null>(null);
  const [importPassword, setImportPassword] = useState("");
  const [importError, setImportError] = useState<string | null>(null);
  const [importLoading, setImportLoading] = useState(false);

  // Folder CRUD dialog state
  const [createFolderOpen, setCreateFolderOpen] = useState(false);
  const [renameFolderTarget, setRenameFolderTarget] = useState<Folder | null>(null);
  const [deleteFolderTarget, setDeleteFolderTarget] = useState<Folder | null>(null);

  // DnD sensors
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  // Load data on mount
  useEffect(() => {
    void loadAll();
  }, [loadAll]);

  // Auto-clear banner
  useEffect(() => {
    if (banner) {
      const timer = setTimeout(() => setBanner(null), 4000);
      return () => clearTimeout(timer);
    }
  }, [banner]);

  // Auto-clear error
  useEffect(() => {
    if (connectError) {
      const timer = setTimeout(onClearError, 5000);
      return () => clearTimeout(timer);
    }
  }, [connectError, onClearError]);

  const sessionEntries = Array.from(sessions.values());

  // Build a map: profileId -> sessions for that profile
  const profileSessionMap = useMemo(() => {
    const map = new Map<string, SessionEntry[]>();
    for (const s of sessionEntries) {
      const existing = map.get(s.profileId) ?? [];
      existing.push(s);
      map.set(s.profileId, existing);
    }
    return map;
  }, [sessionEntries]);

  // Auto-expand profile when it FIRST gets an active session
  const autoExpandedRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    const profilesWithSessions = new Set<string>();
    for (const [profileId, sess] of profileSessionMap) {
      if (sess.some((s) => s.state === "connected" || s.state === "connecting" || s.state === "authenticating")) {
        profilesWithSessions.add(profileId);
      }
    }
    const toExpand: string[] = [];
    for (const id of profilesWithSessions) {
      if (!autoExpandedRef.current.has(id)) {
        toExpand.push(id);
        autoExpandedRef.current.add(id);
      }
    }
    for (const id of autoExpandedRef.current) {
      if (!profilesWithSessions.has(id)) autoExpandedRef.current.delete(id);
    }
    if (toExpand.length > 0) {
      setExpandedProfileIds((prev) => {
        const next = new Set(prev);
        for (const id of toExpand) next.add(id);
        return next;
      });
    }
  }, [profileSessionMap]);

  // Filtered profiles for search
  const filteredProfileIds = useMemo(() => {
    if (!isSearching) return null; // null = no filter
    const q = searchQuery.toLowerCase();
    return new Set(
      allProfiles
        .filter(
          (p) =>
            p.name.toLowerCase().includes(q) ||
            p.host.toLowerCase().includes(q) ||
            p.users.some((u) => u.username.toLowerCase().includes(q))
        )
        .map((p) => p.id)
    );
  }, [allProfiles, searchQuery, isSearching]);

  // Compute per-folder filtered profiles (for rendering)
  const folderProfilesForRender = useMemo((): Map<string, ConnectionProfile[]> => {
    const result = new Map<string, ConnectionProfile[]>();
    for (const folder of folders) {
      const inFolder = profileMap.get(folder.id) ?? [];
      if (!filteredProfileIds) {
        result.set(folder.id, inFolder);
      } else {
        result.set(folder.id, inFolder.filter((p) => filteredProfileIds.has(p.id)));
      }
    }
    return result;
  }, [folders, profileMap, filteredProfileIds]);

  // During search, hide folders with 0 visible profiles (except if not searching)
  const visibleFolders = useMemo(() => {
    if (!isSearching) return folders;
    return folders.filter((f) => (folderProfilesForRender.get(f.id) ?? []).length > 0);
  }, [folders, isSearching, folderProfilesForRender]);

  // ── DnD handlers ─────────────────────────────────────

  const handleDragEnd = useCallback(
    (event: DragEndEvent) => {
      const { active, over } = event;
      if (!over || active.id === over.id) return;

      const activeStr = active.id.toString();
      const overStr = over.id.toString();

      const [kindA, idA] = activeStr.split(":");
      const [kindB, idB] = overStr.split(":");

      if (kindA === "folder" && kindB === "folder") {
        // Reorder folders
        const currentIds = folders.map((f) => `folder:${f.id}`);
        const oldIndex = currentIds.indexOf(activeStr);
        const newIndex = currentIds.indexOf(overStr);
        if (oldIndex === -1 || newIndex === -1) return;

        const newOrder = [...currentIds];
        newOrder.splice(oldIndex, 1);
        newOrder.splice(newIndex, 0, activeStr);
        void reorderFolders(newOrder.map((s) => s.replace("folder:", "")));
        return;
      }

      if (kindA === "profile" && kindB === "profile") {
        // Find which folder each profile is in
        const activeProfile = allProfiles.find((p) => p.id === idA);
        const overProfile = allProfiles.find((p) => p.id === idB);
        if (!activeProfile || !overProfile) return;

        if (activeProfile.folderId === overProfile.folderId && activeProfile.folderId) {
          // Same folder — reorder within folder
          const inFolder = folderProfilesForRender.get(activeProfile.folderId) ?? [];
          const oldIdx = inFolder.findIndex((p) => p.id === idA);
          const newIdx = inFolder.findIndex((p) => p.id === idB);
          if (oldIdx === -1 || newIdx === -1) return;

          const newOrder = [...inFolder];
          newOrder.splice(oldIdx, 1);
          newOrder.splice(newIdx, 0, activeProfile);
          void reorderProfilesInFolder(
            activeProfile.folderId,
            newOrder.map((p) => p.id),
          );
        }
        // Cross-folder drag: silent no-op (user can use context menu "Move to folder")
        return;
      }
    },
    [folders, allProfiles, folderProfilesForRender, reorderFolders, reorderProfilesInFolder],
  );

  // ── Profile action handlers ───────────────────────────

  function handleToggleProfileExpand(profileId: string) {
    setExpandedProfileIds((prev) => {
      const next = new Set(prev);
      if (next.has(profileId)) { next.delete(profileId); } else { next.add(profileId); }
      return next;
    });
  }

  function handleDeleteProfileClick(profileId: string, profileName: string) {
    const profileSessions = profileSessionMap.get(profileId);
    const hasActive =
      profileSessions?.some(
        (s) => s.state === "connected" || s.state === "connecting" || s.state === "authenticating"
      ) ?? false;
    setDeleteProfileConfirm({ profileId, profileName, hasActiveSession: hasActive });
  }

  async function handleDeleteProfileConfirm() {
    if (!deleteProfileConfirm) return;
    setDeleteProfileLoading(true);
    try {
      if (deleteProfileConfirm.hasActiveSession) {
        const profileSessions = profileSessionMap.get(deleteProfileConfirm.profileId);
        const activeSess = profileSessions?.filter(
          (s) => s.state === "connected" || s.state === "connecting" || s.state === "authenticating"
        ) ?? [];
        for (const s of activeSess) { onDisconnect(s.id); }
        await new Promise((r) => setTimeout(r, 300));
      }
      await deleteProfile(deleteProfileConfirm.profileId);
      setDeleteProfileConfirm(null);
    } catch {
      try {
        const profileSessions = profileSessionMap.get(deleteProfileConfirm.profileId);
        const activeSess = profileSessions?.filter(
          (s) => s.state === "connected" || s.state === "connecting" || s.state === "authenticating"
        ) ?? [];
        for (const s of activeSess) { onDisconnect(s.id); }
        await new Promise((r) => setTimeout(r, 500));
        await deleteProfile(deleteProfileConfirm.profileId);
        setDeleteProfileConfirm(null);
      } catch {
        setDeleteProfileConfirm((prev) => prev ? { ...prev, hasActiveSession: true } : null);
      }
    } finally {
      setDeleteProfileLoading(false);
    }
  }

  function handleDeleteProfileCancel() {
    if (deleteProfileLoading) return;
    setDeleteProfileConfirm(null);
  }

  const handleMoveToFolder = useCallback(
    (profileId: string, targetFolderId: string, _targetCount: number) => {
      // Append at end of target folder
      const targetProfiles = profileMap.get(targetFolderId) ?? [];
      void moveProfileToFolder(profileId, targetFolderId, targetProfiles.length);
    },
    [profileMap, moveProfileToFolder],
  );

  // ── Folder action handlers ────────────────────────────

  function handleToggleFolderExpand(folderId: string) {
    void toggleFolderExpanded(folderId);
  }

  function handleRenameFolder(folder: Folder) {
    if (folder.isSystem) return;
    setRenameFolderTarget(folder);
  }

  function handleDeleteFolderRequest(folder: Folder) {
    if (folder.isSystem) return;
    setDeleteFolderTarget(folder);
  }

  async function handleCreateFolderConfirm(name: string) {
    await createFolder(name);
  }

  async function handleRenameFolderConfirm(folderId: string, newName: string) {
    await renameFolder(folderId, newName);
  }

  async function handleDeleteFolderConfirm(folderId: string) {
    await deleteFolder(folderId);
  }

  // ── Export/Import handlers ────────────────────────────

  const handleExportClick = useCallback(() => {
    if (allProfiles.length === 0) {
      setBanner({ type: "error", message: t("sidebar.noProfilesToExport") });
      return;
    }
    setExportIncludePasswords(false);
    setExportPassword("");
    setExportPasswordConfirm("");
    setExportError(null);
    setExportLoading(false);
    setExportDialog(true);
  }, [allProfiles.length, t]);

  const handleExportConfirm = useCallback(async () => {
    if (exportIncludePasswords) {
      if (!exportPassword) { setExportError(t("sidebar.exportDialog.passwordRequired")); return; }
      if (exportPassword !== exportPasswordConfirm) { setExportError(t("sidebar.exportDialog.passwordMismatch")); return; }
    }
    setExportError(null);
    setExportLoading(true);
    try {
      const ext = exportIncludePasswords ? "nexterm" : "json";
      const defaultName = exportIncludePasswords ? "nexterm-profiles.nexterm" : "nexterm-profiles.json";
      const filterName = exportIncludePasswords ? "NexTerm Encrypted" : "JSON";
      const path = await save({ defaultPath: defaultName, filters: [{ name: filterName, extensions: [ext] }] });
      if (!path) { setExportLoading(false); return; }
      const result = await exportProfiles(path, exportIncludePasswords, exportIncludePasswords ? exportPassword : undefined);
      setExportDialog(false);
      if (result.warnings.includes("acl_not_applied")) {
        setBanner({ type: "success", message: t("sidebar.exportSuccessWithAclWarning", { count: result.count }) });
      } else {
        setBanner({ type: "success", message: t("sidebar.exportSuccess", { count: result.count }) });
      }
    } catch (err) {
      setExportError(String(err));
    } finally {
      setExportLoading(false);
    }
  }, [exportIncludePasswords, exportPassword, exportPasswordConfirm, exportProfiles, t]);

  const handleImportClick = useCallback(async () => {
    try {
      const path = await open({ filters: [{ name: "NexTerm", extensions: ["json", "nexterm"] }], multiple: false });
      if (!path) return;
      const filePath = path as string;
      if (filePath.endsWith(".nexterm")) {
        setImportPassword("");
        setImportError(null);
        setImportLoading(false);
        setImportPasswordDialog(filePath);
        return;
      }
      const result = await importProfiles(filePath);
      setBanner({ type: "success", message: t("sidebar.importSuccess", { imported: result.imported, skipped: result.skipped }) });
    } catch {
      setBanner({ type: "error", message: t("sidebar.importError") });
    }
  }, [importProfiles, t]);

  const handleImportWithPassword = useCallback(async () => {
    if (!importPasswordDialog || !importPassword) return;
    setImportError(null);
    setImportLoading(true);
    try {
      const result = await importProfiles(importPasswordDialog, importPassword);
      setImportPasswordDialog(null);
      setBanner({ type: "success", message: t("sidebar.importSuccess", { imported: result.imported, skipped: result.skipped }) });
    } catch (err) {
      const msg = String(err);
      if (msg.includes("Wrong export password") || msg.includes("corrupted")) {
        setImportError(t("sidebar.importPassword.wrongPassword"));
      } else {
        setImportError(msg);
      }
    } finally {
      setImportLoading(false);
    }
  }, [importPasswordDialog, importPassword, importProfiles, t]);

  // Existing folder names for validation (exclude system folder)
  const existingFolderNames = useMemo(
    () => folders.filter((f) => !f.isSystem).map((f) => f.name),
    [folders],
  );

  // ── Render ────────────────────────────────────────────

  // Total profile count for badge
  const totalProfiles = allProfiles.length;

  // DnD items at folder level (outer SortableContext)
  const folderDndIds = useMemo(() => folders.map((f) => `folder:${f.id}`), [folders]);

  return (
    <aside className="sidebar">
      {/* ── Search ── */}
      <div className="sidebar-search">
        <div className="sidebar-search-wrapper">
          <svg className="sidebar-search-icon-svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="11" cy="11" r="8" />
            <line x1="21" y1="21" x2="16.65" y2="16.65" />
          </svg>
          <input
            className="sidebar-search-input"
            type="text"
            placeholder={t("sidebar.search")}
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            data-form-type="other"
            data-lpignore="true"
          />
        </div>
      </div>

      {/* ── Profiles Section ── */}
      <div className="sidebar-section">
        {/* Section header + toolbar */}
        <div className="sidebar-section-header-collapsible" style={{ cursor: "default" }}>
          <div className="sidebar-section-header-left">
            <span className="sidebar-section-title">{t("sidebar.profiles")}</span>
            <span className="sidebar-section-badge">{totalProfiles}</span>
          </div>
        </div>

        {/* Actions toolbar */}
        <div className="sidebar-actions-row">
          <button
            className="sidebar-action-btn sidebar-action-btn-labeled"
            onClick={() => void handleImportClick()}
            title={t("sidebar.import")}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="7 10 12 15 17 10" />
              <line x1="12" y1="15" x2="12" y2="3" />
            </svg>
            <span>{t("sidebar.importShort")}</span>
          </button>
          <button
            className="sidebar-action-btn sidebar-action-btn-labeled"
            onClick={() => handleExportClick()}
            title={t("sidebar.export")}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="17 8 12 3 7 8" />
              <line x1="12" y1="3" x2="12" y2="15" />
            </svg>
            <span>{t("sidebar.exportShort")}</span>
          </button>
          <button
            className="sidebar-action-btn sidebar-action-btn-labeled sidebar-action-btn-primary"
            onClick={() => onNewProfile()}
            title={t("sidebar.newProfile")}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
              <line x1="12" y1="5" x2="12" y2="19" />
              <line x1="5" y1="12" x2="19" y2="12" />
            </svg>
            <span>{t("sidebar.newProfileShort")}</span>
          </button>
          <button
            className="sidebar-action-btn sidebar-action-btn-labeled"
            onClick={() => setCreateFolderOpen(true)}
            title={t("sidebar.folders.createFolder")}
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
              <line x1="12" y1="11" x2="12" y2="17" />
              <line x1="9" y1="14" x2="15" y2="14" />
            </svg>
            <span>{t("sidebar.folders.newFolder")}</span>
          </button>
        </div>

        {/* Content */}
        <div className="sidebar-section-content">
          <div className="sidebar-list" role="tree">
            {/* Loading state */}
            {loading && <div className="sidebar-empty">{t("sidebar.loading")}</div>}

            {/* Empty: no profiles at all */}
            {!loading && totalProfiles === 0 && (
              <div className="sidebar-empty-state">
                {t("sidebar.noProfiles")}{" "}
                <button className="sidebar-empty-state-cta" onClick={onNewProfile}>
                  {t("sidebar.noProfilesCta")}
                </button>
              </div>
            )}

            {/* Search: no results */}
            {!loading && totalProfiles > 0 && isSearching && visibleFolders.length === 0 && (
              <div className="sidebar-empty-state">{t("sidebar.noResults")}</div>
            )}

            {/* Folder-grouped profiles with DnD */}
            {!loading && (
              <DndContext
                sensors={sensors}
                collisionDetection={closestCenter}
                onDragEnd={handleDragEnd}
              >
                <SortableContext
                  items={folderDndIds}
                  strategy={verticalListSortingStrategy}
                  disabled={isSearching}
                >
                  {visibleFolders.map((folder) => {
                    const folderProfiles = folderProfilesForRender.get(folder.id) ?? [];
                    const isExpanded = expandedFolderIds.has(folder.id);

                    return (
                      <FolderRow
                        key={folder.id}
                        folder={folder}
                        profiles={folderProfiles}
                        isExpanded={isExpanded}
                        isSearching={isSearching}
                        connectingProfileId={connectingProfileId}
                        sessionMap={profileSessionMap}
                        activeSessionId={activeSessionId}
                        expandedProfileIds={expandedProfileIds}
                        allFolders={folders}
                        onToggleExpand={handleToggleFolderExpand}
                        onToggleProfileExpand={handleToggleProfileExpand}
                        onConnect={onConnect}
                        onEditProfile={onEditProfile}
                        onDeleteProfileClick={handleDeleteProfileClick}
                        onSetActiveSession={setActiveSession}
                        onDisconnect={onDisconnect}
                        onMoveToFolder={handleMoveToFolder}
                        onRenameFolder={handleRenameFolder}
                        onDeleteFolder={handleDeleteFolderRequest}
                        t={t}
                        dndDisabled={isSearching}
                      />
                    );
                  })}
                </SortableContext>
              </DndContext>
            )}
          </div>
        </div>

        {/* Export/import feedback banner */}
        {banner && (
          <div
            className={`sidebar-banner ${banner.type === "success" ? "sidebar-banner-success" : "sidebar-banner-error"}`}
            onClick={() => setBanner(null)}
            title={t("general.close")}
          >
            {banner.type === "success" ? "\u2713 " : ""}{banner.message}
          </div>
        )}

        {/* Connect error inline */}
        {connectError && (
          <div className="sidebar-error" onClick={onClearError} title={t("general.close")}>
            {connectError}
          </div>
        )}
      </div>

      {/* ── Delete Profile Confirmation Dialog ── */}
      <Dialog
        open={deleteProfileConfirm !== null}
        onClose={handleDeleteProfileCancel}
        title=""
        width="400px"
      >
        {deleteProfileConfirm && (
          <>
            <div className="delete-confirm-header">
              <div className="delete-confirm-icon">{"\u26A0"}</div>
              <div className="delete-confirm-text">
                <h3 className="delete-confirm-title">
                  {t("sidebar.deleteConfirmTitle")}
                </h3>
                <p className="delete-confirm-message">
                  {deleteProfileConfirm.hasActiveSession
                    ? t("sidebar.deleteConfirmActiveSession", { name: deleteProfileConfirm.profileName })
                    : t("sidebar.deleteConfirmMessage", { name: deleteProfileConfirm.profileName })}
                </p>
              </div>
            </div>
            <div className="delete-confirm-actions">
              <button
                className="btn btn-ghost btn-md"
                onClick={handleDeleteProfileCancel}
                disabled={deleteProfileLoading}
              >
                {t("general.cancel")}
              </button>
              <button
                className="btn btn-danger btn-md"
                onClick={() => void handleDeleteProfileConfirm()}
                disabled={deleteProfileLoading}
              >
                {deleteProfileLoading
                  ? t("sidebar.deleteConfirmDeleting")
                  : deleteProfileConfirm.hasActiveSession
                    ? t("sidebar.deleteConfirmDisconnectDelete")
                    : t("sidebar.deleteConfirmDelete")}
              </button>
            </div>
          </>
        )}
      </Dialog>

      {/* ── Export Dialog ── */}
      <Dialog open={exportDialog} onClose={() => !exportLoading && setExportDialog(false)} title="" width="420px">
        <div className="cd-header">
          <div className="cd-header-icon">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
              <polyline points="17 8 12 3 7 8" />
              <line x1="12" y1="3" x2="12" y2="15" />
            </svg>
          </div>
          <div className="cd-header-text">
            <h3 className="cd-title">{t("sidebar.exportDialog.title")}</h3>
            <p className="cd-subtitle">{t("sidebar.exportDialog.subtitle")}</p>
          </div>
        </div>
        <div className="cd-section-content">
          <label className="export-checkbox-label">
            <input
              type="checkbox"
              checked={exportIncludePasswords}
              onChange={(e) => {
                setExportIncludePasswords(e.target.checked);
                if (!e.target.checked) { setExportPassword(""); setExportPasswordConfirm(""); setExportError(null); }
              }}
            />
            <span>{t("sidebar.exportDialog.includePasswords")}</span>
          </label>
          {exportIncludePasswords && (
            <>
              <p className="export-password-hint">{t("sidebar.exportDialog.exportPasswordHint")}</p>
              <div className="input-group">
                <label className="input-label">{t("sidebar.exportDialog.exportPassword")}</label>
                <input className="input" type="password" value={exportPassword} onChange={(e) => setExportPassword(e.target.value)} autoComplete="off" autoCorrect="off" autoCapitalize="off" spellCheck={false} data-form-type="other" data-lpignore="true" autoFocus />
              </div>
              <div className="input-group">
                <label className="input-label">{t("sidebar.exportDialog.confirmPassword")}</label>
                <input className="input" type="password" value={exportPasswordConfirm} onChange={(e) => setExportPasswordConfirm(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") void handleExportConfirm(); }} autoComplete="off" autoCorrect="off" autoCapitalize="off" spellCheck={false} data-form-type="other" data-lpignore="true" />
              </div>
            </>
          )}
          {exportError && <div className="cd-error-message">{exportError}</div>}
        </div>
        <div className="cd-actions">
          <button className="btn btn-ghost btn-md" onClick={() => setExportDialog(false)} disabled={exportLoading}>
            {t("general.cancel")}
          </button>
          <button className="btn btn-primary btn-md" onClick={() => void handleExportConfirm()} disabled={exportLoading}>
            {exportLoading ? t("general.loading") : t("sidebar.export")}
          </button>
        </div>
      </Dialog>

      {/* ── Import Password Dialog ── */}
      <Dialog open={importPasswordDialog !== null} onClose={() => !importLoading && setImportPasswordDialog(null)} title="" width="420px">
        <div className="cd-header">
          <div className="cd-header-icon">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
              <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
              <path d="M7 11V7a5 5 0 0 1 10 0v4" />
            </svg>
          </div>
          <div className="cd-header-text">
            <h3 className="cd-title">{t("sidebar.importPassword.title")}</h3>
            <p className="cd-subtitle">{t("sidebar.importPassword.message")}</p>
          </div>
        </div>
        <div className="cd-section-content">
          <div className="input-group">
            <label className="input-label">{t("sidebar.exportDialog.exportPassword")}</label>
            <input className="input" type="password" value={importPassword} onChange={(e) => setImportPassword(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") void handleImportWithPassword(); }} autoComplete="off" autoCorrect="off" autoCapitalize="off" spellCheck={false} data-form-type="other" data-lpignore="true" autoFocus />
          </div>
          {importError && <div className="cd-error-message">{importError}</div>}
        </div>
        <div className="cd-actions">
          <button className="btn btn-ghost btn-md" onClick={() => setImportPasswordDialog(null)} disabled={importLoading}>
            {t("general.cancel")}
          </button>
          <button className="btn btn-primary btn-md" onClick={() => void handleImportWithPassword()} disabled={importLoading || !importPassword}>
            {importLoading ? t("general.loading") : t("sidebar.import")}
          </button>
        </div>
      </Dialog>

      {/* ── Create Folder Dialog ── */}
      <CreateFolderDialog
        open={createFolderOpen}
        onClose={() => setCreateFolderOpen(false)}
        onConfirm={handleCreateFolderConfirm}
        existingNames={existingFolderNames}
        t={t}
      />

      {/* ── Rename Folder Dialog ── */}
      <RenameFolderDialog
        open={renameFolderTarget !== null}
        folder={renameFolderTarget}
        onClose={() => setRenameFolderTarget(null)}
        onConfirm={handleRenameFolderConfirm}
        existingNames={existingFolderNames}
        t={t}
      />

      {/* ── Delete Folder Dialog ── */}
      <DeleteFolderDialog
        open={deleteFolderTarget !== null}
        folder={deleteFolderTarget}
        profileCount={deleteFolderTarget ? (profileMap.get(deleteFolderTarget.id) ?? []).length : 0}
        onClose={() => setDeleteFolderTarget(null)}
        onConfirm={handleDeleteFolderConfirm}
        t={t}
      />
    </aside>
  );
}
