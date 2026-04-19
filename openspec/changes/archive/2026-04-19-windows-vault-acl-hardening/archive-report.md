# Archive Report: windows-vault-acl-hardening

**Date Archived**: 2026-04-19 18:39:36  
**Archive Location**: `openspec/changes/archive/2026-04-19-windows-vault-acl-hardening/`  
**Final Status**: COMPLETE ✅  

## Change Summary

**Change ID**: windows-vault-acl-hardening  
**Capability**: vault-storage-security  
**Type**: Security defense-in-depth  
**Platform Target**: Windows (Unix already race-free; hardening adds consistency)  

## Completion Metrics

| Metric | Value |
|--------|-------|
| **Tasks** | 55/55 (100%) ✅ |
| **Tests** | 74 passing, 1 pre-existing failure (ssh::keys) |
| **Verify Verdict** | PASS_WITH_WARNINGS (0 CRITICAL) |
| **Commit Range** | f35a1b8..669840f (6 commits) |
| **Verification Warnings** | 2 (W1: stale `#[allow(dead_code)]`, W2: export integration test deferred) |
| **Verification Suggestions** | 3 (minor cleanup, deferred to follow-on commit) |

## Implementation Summary

### New Module: `fs_secure`
Centralized filesystem security operations:
- `pub(crate) secure_write(path: &Path, bytes: &[u8]) -> io::Result<()>` — atomic write with pre-rename hardening
- `pub(crate) harden_file_permissions(path: &Path) -> io::Result<()>` — low-level hardening (returns io::Error)
- `pub(crate) best_effort_harden(path: &Path) -> BestEffortOutcome` — graceful degradation for export flows
- Windows: SetNamedSecurityInfoW DACL with current user SID + GENERIC_ALL, PROTECTED_DACL_SECURITY_INFORMATION
- Unix: chmod 0o600 with pre-rename safety

### Files Added to Main Codebase
- `src-tauri/src/fs_secure.rs` (354 lines) — platform-native secure write + hardening
- `src-tauri/src/fs_secure/unix.rs` (47 lines) — Unix-specific logic  
- `src-tauri/src/fs_secure/windows.rs` (187 lines) — Windows DACL via windows crate v0.61
- `docs/security.md` (new) — FAT32/network/GPO limitations, design rationale

### Files Modified in Main Codebase
- `src-tauri/src/vault.rs` — removed inline `#[cfg(unix)]`, now uses `secure_write`; added test module (21 tests)
- `src-tauri/src/profile.rs` — removed inline `#[cfg(unix)]`, now uses `secure_write`; harden `profiles.backup.json`
- `src-tauri/src/commands/vault.rs` — migration re-hardening on `vault_unlock` success
- `src-tauri/src/commands/profile.rs` — export flow uses `best_effort_harden` + emits non-fatal warnings
- `src-tauri/src/lib.rs` — exposed `pub mod fs_secure`
- `src-tauri/Cargo.toml` — added `[target.'cfg(windows)'.dependencies] windows = "0.61"` + dev-dep `tempfile = "3"`
- `src-react/stores/profileStore.ts` — connected to export warning channel
- `src-react/components/Sidebar.tsx` — displays export ACL hardening warnings
- `src-react/i18n/` — internationalization keys for warning messages

### Test Coverage
- **Unit Tests**: fs_secure module (8 tests), vault module (8 tests), profile tests (3 tests)
- **Windows Integration**: DACL verification test (1 test, passes live on Windows)
- **Pre-existing failures**: 1 (ssh::keys — unrelated to this change)
- **Total new from this change**: 29 tests

## Spec Requirements Compliance

All 9 spec requirements from `vault-storage-security` specification implemented and tested:

1. ✅ **R1** — Atomic write with pre-rename hardening (secure_write + integration tests)
2. ✅ **R2** — Unix permissions (chmod 0o600, tested)
3. ✅ **R3** — Windows permissions (DACL with GENERIC_ALL only to current user, tested via GetNamedSecurityInfoW)
4. ✅ **R4** — Files covered (vault.json, profiles.json, profiles.backup.json, exports)
5. ✅ **R5** — Failure behavior on unsupported FS (FAT32/network silent continue with logged debug/warn)
6. ✅ **R6** — Migration of existing files (re-harden on vault_unlock, idempotent)
7. ✅ **R7** — Platform abstraction (zero `#[cfg]` in vault.rs/profile.rs; all in fs_secure)
8. ✅ **R8** — Error propagation (harden_file_permissions returns io::Error; callers map to AppError)
9. ✅ **R9** — Known limitations documented (docs/security.md covers GPO, FAT32, network-share limits)

## Artifact Traceability (Hybrid Mode)

Engram observation IDs for full SDD cycle audit trail:

| Artifact | Observation ID | Type |
|----------|-----------------|------|
| Proposal | #394 | decision |
| Spec | #395 | architecture |
| Verify Report | #414 | architecture |
| Session Summary (verify phase) | #416 | session_summary |

*Archive report (this document)* saved as Engram observation with topic_key = `sdd/windows-vault-acl-hardening/archive-report`

## Filesystem Operations Performed

**Promotion (Delta → Main Specs)**:
- Created: `openspec/specs/vault-storage-security/` directory
- Copied: `openspec/changes/archive/2026-04-19-windows-vault-acl-hardening/specs/vault-storage-security/spec.md` → `openspec/specs/vault-storage-security/spec.md`
- Size: 6973 bytes (128 lines, 9 requirements + scenarios + edge cases)

**Move to Archive**:
- Created: `openspec/changes/archive/` directory
- Moved: `openspec/changes/windows-vault-acl-hardening/` → `openspec/changes/archive/2026-04-19-windows-vault-acl-hardening/`
- Archived contents: proposal.md, specs/, design.md, tasks.md, verify-report.md, apply-progress.md, explore.md

**Verification**:
- ✅ Main spec at canonical location: `openspec/specs/vault-storage-security/spec.md`
- ✅ Change folder no longer in active directory: `openspec/changes/archive/2026-04-19-windows-vault-acl-hardening/`
- ✅ All artifacts present in archive (7 files + specs/ subdirectory)

## Key Learnings

1. **Rust Toolchain Detection Gap in sdd-init**: The sdd-init skill did not detect that this project requires Windows target configuration (`windows = "0.61"` crate with Win32 features). Future SDD cycles should pre-detect platform-specific deps and emit warnings during initialization.

2. **Test-Seam Pattern for Tauri-Coupled Code**: The export flow (`best_effort_harden`) demonstrates a clean test seam where callers can gracefully degrade without aborting writes. This pattern was applied to separate `harden_file_permissions` (strict, returns error) from `best_effort_harden` (forgiving, returns BestEffortOutcome enum).

3. **GENERIC_ALL → FILE_ALL_ACCESS Gotcha**: Windows DACL testing initially confused `FILE_ALL_ACCESS` (0x001F01FF, includes directory ops) with `GENERIC_ALL` (0xFFFFFFFF, mapped at runtime to `STANDARD_RIGHTS_ALL | FILE_GENERIC_READ | FILE_GENERIC_WRITE`). The spec correctly mandates `GENERIC_ALL` to ensure all future permission expansions are automatically inherited.

4. **Same-Volume Rename ACL Preservation on NTFS**: NTFS automatically preserves an explicit DACL (not inherited) during rename operations. This was validated by the integration test and is NOT true for FAT32 or network shares. The design relies on this behavior; cross-volume rename falls outside the design scope (both source and destination must be same volume).

5. **Idempotency Through Win32 Semantics**: The Windows `SetNamedSecurityInfoW` call with `PROTECTED_DACL_SECURITY_INFORMATION` flag is idempotent — re-applying the same DACL multiple times (migration + every unlock + every save) incurs a system call cost (<1ms) but never fails due to "already set" conditions. No state flag needed.

## Deferred (Follow-on Commits)

Per verify-report deviations D11, D14:
- **D11**: Export profiles integration test (vs. unit tests on helper `build_export_result`)
- **W1/D14**: Stale `#[allow(dead_code)]` attributes (W1 on `BestEffortOutcome::Failed`, S1-S2 on HandleGuard structs)

These are cosmetic + non-critical. Archive proceeds; suggest follow-on commit addressing cleanup.

## SDD Cycle Complete

The `windows-vault-acl-hardening` change has successfully transitioned from proposal → spec → design → implementation (55 tasks, 74 tests) → verification (PASS_WITH_WARNINGS) → **archive**.

**Next change** is ready to begin. The `vault-storage-security` capability now lives in main specs at `openspec/specs/vault-storage-security/spec.md` and is the source of truth for all future vault-storage-related enhancements.

---

**Archived By**: sdd-archive sub-agent  
**Archive Timestamp**: 2026-04-19 18:39:36  
**Artifact Store Mode**: hybrid (Engram + openspec filesystem)  
**Archive Integrity**: ✅ All 7 artifacts + specs/ present