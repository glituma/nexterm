# Design: Windows Vault ACL Hardening

## Technical Approach

Introduce `src-tauri/src/fs_secure/` (directory module) as the single home for all platform file-permission logic. Callers (`vault.rs`, `profile.rs`, `commands/*`) become `#[cfg]`-free and invoke one of three functions: `secure_write` (atomic strict write), `harden_file_permissions` (strict harden), or `best_effort_harden` (never fails). Windows uses the already-transitive `windows` crate with `SetNamedSecurityInfoW` + `PROTECTED_DACL_SECURITY_INFORMATION` to grant only the current user SID `GENERIC_ALL`, stripping inherited ACEs. Unix uses `PermissionsExt::from_mode(0o600)`. Non-unix/non-windows is a no-op. The Unix race window is closed by hardening the `.tmp` BEFORE rename.

## Architecture Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Module layout | Directory module `fs_secure/` with `unix.rs`, `windows.rs`, `fallback.rs` | Windows impl is ~60 LOC of `unsafe` + RAII guards + its own tests — needs isolation from Unix one-liner. Zero `#[cfg]` leaks outside the module. |
| API visibility | `pub(crate)` for all three fns | Internal tooling; no external Rust consumers. Matches proposal decision 1. |
| Error type for strict path | `std::io::Error` | Low-level; callers map to `AppError`. Lets `secure_write` use `?` across platform impls. |
| Best-effort outcome type | Enum `BestEffortOutcome { Hardened, SkippedUnsupported, Failed(io::Error) }` | Export flow needs to distinguish "unsupported FS (silent)" from "real failure (warn)" for the frontend toast. No `Result` — caller never forced to handle. |
| Harden placement in `secure_write` | Harden `.tmp` **before** rename | Closes race: final path never exists with default ACL. Fixes pre-existing Unix bug noted in explore §I. |
| `tmp_path` strategy | `<path>.tmp` in same directory | Guarantees same-volume rename → atomic on NTFS/ext4/APFS. Cross-volume is out of scope (proposal decision 4). |
| `profiles.backup.json` | `best_effort_harden` after `fs::copy` | Migration must not fail on ACL. In scope (proposal decision 6). |
| Windows dep | `windows = "0.61"` target-dep, features: `Win32_Foundation`, `Win32_Security`, `Win32_Security_Authorization`, `Win32_System_Threading` | Already transitive → zero new compile cost. |

## Public API

```rust
pub(crate) fn secure_write(path: &Path, bytes: &[u8]) -> std::io::Result<()>;
pub(crate) fn harden_file_permissions(path: &Path) -> std::io::Result<()>;
pub(crate) fn best_effort_harden(path: &Path) -> BestEffortOutcome;
pub(crate) enum BestEffortOutcome { Hardened, SkippedUnsupported, Failed(std::io::Error) }
```

`secure_write` contract: `create_dir_all(parent)` → `fs::write(tmp)` → `harden(tmp)` → `rename(tmp, path)`. On harden failure: attempt `remove_file(tmp)` (ignore error), propagate original error.

`best_effort_harden` classifies `io::ErrorKind::Unsupported` and raw OS errors `50` (`ERROR_NOT_SUPPORTED`) / `1` (`ERROR_INVALID_FUNCTION`) as `SkippedUnsupported` (FAT32/network); everything else is `Failed`.

## Windows Implementation Sketch (`fs_secure/windows.rs`)

1. `OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, ...)` wrapped in `HandleGuard` (RAII → `CloseHandle` on drop).
2. `GetTokenInformation(TokenUser)` — double-call pattern for buffer sizing; read `TOKEN_USER.User.Sid`.
3. Build `EXPLICIT_ACCESS_W`: `grfAccessPermissions=GENERIC_ALL`, `grfAccessMode=SET_ACCESS`, `grfInheritance=NO_INHERITANCE`, `Trustee{TrusteeForm=TRUSTEE_IS_SID, ptstrName=sid_ptr}`.
4. `SetEntriesInAclW(1, &ea, None, &mut new_acl)` → wrap `new_acl` in `LocalAllocGuard` (RAII → `LocalFree`).
5. Wide-string path: `OsStrExt::encode_wide().chain(once(0)).collect::<Vec<u16>>()`.
6. `SetNamedSecurityInfoW(wide.as_mut_ptr(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, None, None, Some(new_acl), None)`.
7. Map Win32 errors via `io::Error::from_raw_os_error`.

All `unsafe` uses `?` + `map_err`; no panics. RAII guards ensure no leaks on early return.

## Data Flow

    caller ──► secure_write(path, bytes)
                 │
                 ├─► fs::write(tmp)
                 ├─► harden_file_permissions(tmp) ─┬─ unix:    chmod 0o600
                 │                                 ├─ windows: Win32 DACL
                 │                                 └─ other:   no-op
                 └─► fs::rename(tmp, path)

## File Changes

| File | Action | Description |
|---|---|---|
| `src-tauri/src/fs_secure/mod.rs` | Create | Public API + `BestEffortOutcome` + `secure_write` + `tmp_path_for` + `is_unsupported` + tests |
| `src-tauri/src/fs_secure/unix.rs` | Create | `harden` via `PermissionsExt::from_mode(0o600)` |
| `src-tauri/src/fs_secure/windows.rs` | Create | Win32 DACL impl + RAII guards + `#[cfg(windows)]` tests |
| `src-tauri/src/fs_secure/fallback.rs` | Create | `harden` no-op returning `Ok(())` |
| `src-tauri/src/lib.rs` | Modify | `pub(crate) mod fs_secure;` |
| `src-tauri/src/vault.rs` | Modify | Replace L321–337 with `fs_secure::secure_write`; add test module (first ever) |
| `src-tauri/src/profile.rs` | Modify | Replace L237–251 with `secure_write`; `best_effort_harden(&backup_path)` after L206 `fs::copy` |
| `src-tauri/src/commands/vault.rs` | Modify | After `*vault_guard = Some(vault);` loop `best_effort_harden` for `vault.json`/`profiles.json` if present |
| `src-tauri/src/commands/profile.rs` | Modify | Export flow: `best_effort_harden` + propagate `"acl_not_applied"` warning in `ExportResult.warnings` |
| `src-tauri/Cargo.toml` | Modify | `[target.'cfg(windows)'.dependencies] windows = "0.61"` + `[dev-dependencies] tempfile = "3"` |
| `docs/security.md` | Create/Modify | ACL behavior, FAT32/network silent continue, GPO override limitation |

## Interfaces

Extend IPC `ExportResult` to carry `warnings: Vec<String>`. Frontend checks `warnings.includes("acl_not_applied")` → toast: "Export written, but the file system did not accept owner-only permissions." Internal save flows remain invisible.

## Error Handling Map

| Site | Error | Caller behavior |
|---|---|---|
| `fs::write(tmp)` | `io::Error` | Propagated by `secure_write` → `AppError::VaultError`/`ProfileError` |
| `harden` inside `secure_write` | `io::Error` | `secure_write` removes tmp (best-effort), propagates |
| `harden` in migration / export / post-unlock | mapped to `BestEffortOutcome` | `tracing::debug!` unsupported, `tracing::warn!` failed; never bubbled |
| `fs::rename` | `io::Error` | Propagated |

## Testing Strategy

| Layer | What | Approach |
|---|---|---|
| Unit — `fs_secure` | round-trip write, no `.tmp` leftover, idempotency | `tempfile::TempDir` |
| Unit — unix | `0o600` mode after `secure_write` | `#[cfg(unix)]` |
| Integration — windows | DACL has exactly 1 ACE = current-user SID + `GENERIC_ALL`; no `Everyone`/`Users`/`Authenticated Users`; inherited ACEs stripped | `#[cfg(windows)]` helper calls `GetNamedSecurityInfoW` and enumerates ACEs |
| Unit — `vault.rs` (new module) | `vault_create` → ACL applied; `store` → ACL preserved; `change_master_password` → ACL re-applied | `TempDir` helper establishes pattern |
| Unit — `profile.rs` (extend) | `save_profiles_to_disk` sets owner-only ACL; migration backup is hardened | `#[cfg(windows)]` |

## Observability

- `tracing::debug!` when `SkippedUnsupported` (FAT32/network).
- `tracing::warn!` when `Failed` (export path).
- No info logs on success — avoid pollution.

## Security Checklist (for apply)

- [ ] No `panic!`/`unwrap` in `unsafe` paths — `?` + `map_err` only.
- [ ] Every `LocalAlloc` freed via `LocalAllocGuard` on all paths.
- [ ] Every handle closed via `HandleGuard`.
- [ ] Wide-string null-terminated.
- [ ] `PROTECTED_DACL_SECURITY_INFORMATION` set (verified by test).
- [ ] Single ACE verified by integration test.
- [ ] No subprocess invocation (no path-injection surface).

## Migration / Rollout

No data migration. First unlock after update re-hardens existing `vault.json` + `profiles.json` (idempotent, <1ms Win32 call). Rollback per proposal §Rollback.

## Open Questions

None — all decisions are resolved by the proposal. Ready for `sdd-tasks`.
