# NexTerm Security Model

> Generated as part of the `windows-vault-acl-hardening` change.  
> Last updated: 2026-04-19

---

## Table of Contents

1. [Defense-in-Depth Model](#1-defense-in-depth-model)
2. [Encrypted Vault (Layer 1)](#2-encrypted-vault-layer-1)
3. [Filesystem ACL Hardening (Layer 2)](#3-filesystem-acl-hardening-layer-2)
4. [Windows ACL Behavior](#4-windows-acl-behavior)
5. [Unix Permission Behavior](#5-unix-permission-behavior)
6. [FAT32 and Network-Share Silent Fallback](#6-fat32-and-network-share-silent-fallback)
7. [Known Limitations](#7-known-limitations)
8. [Export File Security](#8-export-file-security)
9. [Auto-Updater Endpoint](#9-auto-updater-endpoint)
10. [How to Verify ACL Manually](#10-how-to-verify-acl-manually)
11. [File Locations](#11-file-locations)

---

## 1. Defense-in-Depth Model

NexTerm protects credentials stored on disk with two independent layers:

```
┌───────────────────────────────────────────────────────┐
│  Layer 1 — Cryptographic encryption (AES-256-GCM)     │
│  vault.json is AES-256-GCM encrypted with a key       │
│  derived from the user's master password via Argon2.   │
│  Even if an attacker reads the file bytes, they cannot │
│  recover credentials without the master password.      │
├───────────────────────────────────────────────────────┤
│  Layer 2 — Filesystem permissions (ACL / chmod)       │
│  vault.json and profiles.json receive owner-only       │
│  permissions after every write. This prevents other   │
│  local OS accounts from opening the files at all,     │
│  providing protection even if the encryption key were │
│  ever compromised.                                     │
└───────────────────────────────────────────────────────┘
```

Neither layer alone is sufficient:

- Encryption without ACL hardening means a sibling OS account can copy the
  encrypted blob and attempt offline brute-force.
- ACL without encryption means a privileged administrator or a process running
  as the user can read plaintext credentials.

Both layers together raise the attack cost considerably.

---

## 2. Encrypted Vault (Layer 1)

**Algorithm**: AES-256-GCM (authenticated encryption)  
**Key derivation**: Argon2id — memory-hard, resistant to GPU/ASIC attacks  
**Storage format**: `vault.json` — JSON-serialised, encrypted at rest

Every read of `vault.json` requires the master password to be entered by the
user. The key is never persisted; it lives in memory only for the duration of
the session.

**Source**: `src-tauri/src/vault.rs`

---

## 3. Filesystem ACL Hardening (Layer 2)

The `fs_secure` module (`src-tauri/src/fs_secure/`) provides a single
cross-platform API. Callers in `vault.rs`, `profile.rs`, and
`commands/profile.rs` are `#[cfg]`-free — they call `secure_write` or
`best_effort_harden` and the platform-specific code is encapsulated.

### Atomic write protocol

Every sensitive file write uses the following sequence:

```
1. Write bytes to  <path>.tmp  (in the SAME directory as <path>)
2. Apply owner-only permissions to  <path>.tmp  (BEFORE rename)
3. Rename  <path>.tmp  →  <path>  (atomic on NTFS, ext4, APFS)
```

Hardening the `.tmp` file _before_ rename closes a race window: the final
path never exists on disk with default inherited permissions, even
momentarily.

**Source**: `src-tauri/src/fs_secure/mod.rs`

---

## 4. Windows ACL Behavior

On Windows, `fs_secure/windows.rs` applies a DACL (Discretionary Access
Control List) that:

- Grants **`GENERIC_ALL`** to the current user's SID only.
- Contains **exactly one ACE** (Access Control Entry).
- Strips all inherited ACEs via **`PROTECTED_DACL_SECURITY_INFORMATION`**.
- Grants **no access** to:
  - `S-1-1-0` (Everyone)
  - `S-1-5-32-545` (Users)
  - `S-1-5-11` (Authenticated Users)

### Win32 API calls used

| Call | Purpose |
|------|---------|
| `OpenProcessToken` + `GetTokenInformation(TokenUser)` | Obtain current user SID |
| `SetEntriesInAclW` | Build new DACL from `EXPLICIT_ACCESS_W` |
| `SetNamedSecurityInfoW` | Apply DACL to file |

RAII guards (`HandleGuard`, `LocalAllocGuard`) ensure no handle or heap leaks
on any code path, including early returns.

**Source**: `src-tauri/src/fs_secure/windows.rs`

### Verification

```
icacls %APPDATA%\com.cognidevai.nexterm\vault.json
```

Expected output (simplified):
```
DESKTOP-XXX\username:(F)
Successfully processed 1 files; Failed processing 0 files
```

The absence of `(I)` (Inherited) and the absence of `NT AUTHORITY\...` or
`BUILTIN\...` entries confirms that ACL hardening is in effect.

---

## 5. Unix Permission Behavior

On Linux, macOS, and other Unix-like systems, `fs_secure/unix.rs` sets file
mode `0o600` (owner read+write; no group, no world):

```rust
fs::set_permissions(path, PermissionsExt::from_mode(0o600))
```

This is idempotent — calling it twice on the same file leaves permissions
unchanged and returns `Ok(())`.

**Source**: `src-tauri/src/fs_secure/unix.rs`

---

## 6. FAT32 and Network-Share Silent Fallback

Some filesystems do not support ACL or permission operations:

- **FAT32** — USB drives, older SD cards, some partition schemes
- **Network shares** — SMB/CIFS mounts without ACL support
- **Certain NAS devices** — may expose ACL-less filesystems
- **WASI / exotic targets** — platform has no permission concept

When `harden_file_permissions` returns an error classified as "unsupported"
(i.e., `io::ErrorKind::Unsupported`, or raw OS error `1`
[`ERROR_INVALID_FUNCTION`] or `50` [`ERROR_NOT_SUPPORTED`]), NexTerm:

1. **Does NOT fail the write** — the file is written successfully.
2. Logs at `debug!` level for internal files (`vault.json`, `profiles.json`).
3. Logs at `warn!` level for export files (user-chosen path).
4. For exports, sends a non-fatal **frontend warning** to the user.

In this scenario, Layer 1 (encryption) still protects vault credentials.
`profiles.json` does not contain credentials (only metadata), so its
exposure on a shared filesystem is lower-risk — though still undesirable.

The classification logic lives in `fs_secure::is_unsupported`.

---

## 7. Known Limitations

### 7.1 Group Policy (GPO) ACL Reassertion

Windows Domain Group Policy (GPO) can reassert inherited ACLs on files in
user profile directories. If a GPO rule mandates that `%APPDATA%` files
inherit domain-level ACEs, the protected DACL that NexTerm sets may be
overwritten by the policy engine.

**Mitigation**: NexTerm re-hardens `vault.json` and `profiles.json` every
time the vault is unlocked (`commands/vault.rs::vault_unlock`). If the GPO
runs between unlock operations, the window of exposure exists but is bounded
by the next unlock cycle.

**This cannot be mitigated at the application level.** It requires a domain
administrator to either exempt the NexTerm data directory from the policy or
to configure the policy to honour user-set DACLs.

### 7.2 Cross-Volume Rename Not Supported

NexTerm's atomic write uses `std::fs::rename`, which requires the source
(`.tmp`) and destination to reside on the same volume. By design, the `.tmp`
path is always `<destination>.tmp` in the same directory — so cross-volume
renames never occur.

If the user moves the Tauri application data directory to a different volume
via a symlink or junction point, the rename may fail with an OS error, causing
`secure_write` to return an error. NexTerm will surface this as a save failure.

No copy+delete fallback is implemented — this is intentional. A copy
operation is not atomic and would re-introduce the race window we are
closing.

### 7.3 Application Data Directory Not Hardened

The _directory_ containing `vault.json` (e.g., `%APPDATA%\com.cognidevai.nexterm`)
does not receive ACL hardening. Hardening the directory would prevent other
tools (including the OS profile manager) from operating on it correctly.

This means a sibling OS account can enumerate the directory contents (learn
that vault.json exists) but cannot read the file itself.

### 7.4 Memory Security

Credentials are held in memory as `String` values during a session. No
`mlock`/`VirtualLock` is applied, and Rust's allocator may page memory to
disk under memory pressure. This is a known limitation shared by most
desktop credential managers.

### 7.5 Backup File (`profiles.backup.json`)

During legacy profile format migration, `profiles.backup.json` is created via
`fs::copy` followed by `best_effort_harden`. Because `fs::copy` does not
preserve ACL from source, the backup file starts with default inherited
permissions and is hardened in a subsequent call. There is a brief window
between `fs::copy` and the harden call during which the file has default
permissions. This window is bounded to a sub-millisecond CPU operation.

---

## 8. Export File Security

When the user exports profiles to a file (`.json` or `.nexterm`), NexTerm
calls `best_effort_harden` on the export path after writing. This is
best-effort because the export path is user-chosen and may be on a FAT32
drive, a network share, or any other filesystem.

- **Encrypted exports** (`.nexterm`): AES-256-GCM encrypted. Even without ACL
  protection, credentials are safe as long as the export password is strong.
- **Plain JSON exports**: Profiles only — no credentials are included in the
  exported JSON unless the "Include saved passwords" option is enabled.

If ACL hardening fails on the export path, the frontend displays a non-fatal
warning: _"the file system did not accept owner-only permissions."_

The warning identifier `"acl_not_applied"` is the stable contract between
the Rust backend and the TypeScript frontend. The frontend maps it to a
localised message via the i18n system (`sidebar.exportSuccessWithAclWarning`).

---

## 9. Auto-Updater Endpoint

NexTerm uses `@tauri-apps/plugin-updater` for automatic updates. The update
manifest URL is configured at build time in `src-tauri/tauri.conf.json` and
points to **GitHub Releases** for the `cognidevai/nexterm` repository.

The update check:
- Is performed over HTTPS.
- Downloads the binary from GitHub's CDN.
- The release binary is signed; the signature is verified before installation.

**Disclosure**: The auto-updater makes an outbound HTTPS request to
`https://api.github.com/repos/cognidevai/nexterm/releases/latest` (or
equivalent Tauri update endpoint). This request includes the current app
version and platform. No credentials or user data are transmitted.

---

## 10. How to Verify ACL Manually

### Windows

Open a Command Prompt or PowerShell as the user who runs NexTerm:

```batch
icacls "%APPDATA%\com.cognidevai.nexterm\vault.json"
```

**Expected — ACL hardening active:**
```
DESKTOP-XXX\YourUsername:(F)
Successfully processed 1 files; Failed processing 0 files
```

**Problematic — ACL not hardened (inherited entries present):**
```
DESKTOP-XXX\YourUsername:(F)
NT AUTHORITY\SYSTEM:(I)(F)
BUILTIN\Administrators:(I)(F)
BUILTIN\Users:(I)(RX)
Successfully processed 1 files; Failed processing 0 files
```

If you see `(I)` entries (Inherited), the DACL protection is not in effect.
Possible causes: FAT32 filesystem, GPO reassertion (see §7.1), or a pre-v0.3
install that has not been unlocked yet.

To trigger re-hardening, simply **unlock the vault** — `vault_unlock` applies
`best_effort_harden` to both `vault.json` and `profiles.json` on every call.

### Linux / macOS

```bash
ls -l ~/.local/share/com.cognidevai.nexterm/vault.json
# Expected: -rw------- (0600)

stat -c '%a' ~/.local/share/com.cognidevai.nexterm/vault.json
# Expected: 600
```

---

## 11. File Locations

| File | Platform | Path |
|------|----------|------|
| `vault.json` | Windows | `%APPDATA%\com.cognidevai.nexterm\vault.json` |
| `vault.json` | Linux | `~/.local/share/com.cognidevai.nexterm/vault.json` |
| `vault.json` | macOS | `~/Library/Application Support/com.cognidevai.nexterm/vault.json` |
| `profiles.json` | Windows | `%APPDATA%\com.cognidevai.nexterm\profiles.json` |
| `profiles.json` | Linux | `~/.local/share/com.cognidevai.nexterm/profiles.json` |
| `profiles.json` | macOS | `~/Library/Application Support/com.cognidevai.nexterm/profiles.json` |
| `profiles.backup.json` | All | Same directory as `profiles.json` |

---

## Related Source Files

| File | Role |
|------|------|
| `src-tauri/src/fs_secure/mod.rs` | Public API: `secure_write`, `harden_file_permissions`, `best_effort_harden` |
| `src-tauri/src/fs_secure/windows.rs` | Win32 DACL implementation |
| `src-tauri/src/fs_secure/unix.rs` | `chmod 0o600` implementation |
| `src-tauri/src/fs_secure/fallback.rs` | No-op for unsupported platforms |
| `src-tauri/src/vault.rs` | Vault persistence + `harden_existing_credential_files` |
| `src-tauri/src/profile.rs` | Profile persistence + legacy migration |
| `src-tauri/src/commands/vault.rs` | `vault_unlock` — triggers re-hardening on unlock |
| `src-tauri/src/commands/profile.rs` | `export_profiles` — best-effort hardening on export |
