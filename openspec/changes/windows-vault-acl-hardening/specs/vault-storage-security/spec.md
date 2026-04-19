# vault-storage-security Specification

## Purpose

The local credential vault files (`vault.json`, `profiles.json`, `profiles.backup.json`) and their atomic-write temporaries SHALL be created and maintained with filesystem permissions that restrict access to the current OS user account only, on both Unix and Windows platforms. Hardening SHALL be applied before the destination filename becomes visible to other processes, and SHALL degrade gracefully on filesystems that do not support owner-only permissions.

## ADDED Requirements

### Requirement: R1 — Atomic write with pre-rename hardening

The system SHALL write sensitive files via a `.tmp` path in the SAME directory as the destination, SHALL apply owner-only permissions to the `.tmp` file BEFORE renaming, and SHALL NOT leave the destination with default inherited permissions at any point.

#### Scenario: Race window prevention

- GIVEN a vault save is in progress
- WHEN the on-disk state is observed between write and rename
- THEN the `.tmp` file already has owner-only permissions and no destination file with default permissions ever exists

#### Scenario: Rename preserves ACL on NTFS

- GIVEN a `.tmp` file with an owner-only DACL on an NTFS volume
- WHEN `std::fs::rename` moves it to the final path on the same volume
- THEN the final file retains the owner-only DACL

### Requirement: R2 — Unix permissions

On Unix systems, sensitive files SHALL have mode `0o600` (owner read+write only; no group, no world).

#### Scenario: New vault creation on Unix

- GIVEN a fresh install on Linux
- WHEN the user creates a vault
- THEN `vault.json` exists with mode `0o600` and no `.tmp` file remains

### Requirement: R3 — Windows permissions

On Windows, sensitive files SHALL have a DACL granting `GENERIC_ALL` exclusively to the current user's SID, SHALL strip inherited ACEs via `PROTECTED_DACL_SECURITY_INFORMATION`, and SHALL NOT grant any access to Everyone (S-1-1-0), Users (S-1-5-32-545), or Authenticated Users (S-1-5-11).

#### Scenario: New vault creation on Windows

- GIVEN a fresh install on Windows
- WHEN the user creates a vault
- THEN `vault.json` has a DACL with exactly one ACE (current user SID, GENERIC_ALL) and no inherited ACEs

#### Scenario: Verify DACL via GetNamedSecurityInfoW

- GIVEN `secure_write` has written `vault.json` on Windows
- WHEN a test calls `GetNamedSecurityInfoW` on the file
- THEN the returned DACL contains exactly one ACE for the current user SID with `GENERIC_ALL` and no ACEs for Everyone, Users, or Authenticated Users

### Requirement: R4 — Files covered

The following files SHALL receive owner-only hardening: `vault.json`, `profiles.json`, `profiles.backup.json`, `vault.json.tmp`, `profiles.json.tmp`, and export files written to the application data directory. Export files written to user-chosen external paths SHALL receive best-effort hardening only.

#### Scenario: profiles.backup.json hardening during legacy migration

- GIVEN a user upgrading from a pre-v0.2.0 profile format
- WHEN legacy migration creates `profiles.backup.json` via `fs::copy`
- THEN the backup file receives owner-only permissions on a best-effort basis

### Requirement: R5 — Failure behavior on unsupported filesystems

If permission hardening fails (e.g., FAT32, network share, ACL-less filesystem), the write SHALL complete successfully, the failure SHALL be logged at `debug!` for internal files and `warn!` for export files, the frontend SHALL receive a non-fatal warning when an export cannot be hardened, and the application SHALL NOT abort the write.

#### Scenario: FAT32 / network share fallback for internal files

- GIVEN the app data directory resides on FAT32 or a network share that rejects ACL operations
- WHEN the vault is saved
- THEN the write succeeds, the failure is logged at `debug!` level, and no user-visible error is shown

#### Scenario: Export to FAT32 USB

- GIVEN the user exports credentials to a FAT32 USB drive
- WHEN `best_effort_harden` fails
- THEN the export file is written successfully, a `warn!` log entry is recorded, and the frontend receives a non-fatal notification indicating the export is not ACL-protected

### Requirement: R6 — Migration of existing files

On successful `vault_unlock`, existing `vault.json` and `profiles.json` SHALL have permission hardening re-applied. Re-application SHALL be idempotent and SHALL NOT require any flag or persisted state.

#### Scenario: Existing vault migration on Windows

- GIVEN a user upgrading from a pre-hardening version on Windows
- WHEN they unlock the vault
- THEN `vault.json` and `profiles.json` receive owner-only DACLs

#### Scenario: Repeated saves remain idempotent

- GIVEN `vault.json` already has owner-only permissions
- WHEN another save occurs
- THEN the permissions remain owner-only and no error occurs

### Requirement: R7 — Platform abstraction

`vault.rs` and `profile.rs` SHALL contain zero `#[cfg(unix)]` or `#[cfg(windows)]` blocks related to filesystem permission handling. All platform-specific permission code SHALL live in a dedicated `fs_secure` module.

#### Scenario: No platform cfg in vault/profile modules

- GIVEN the source files `src-tauri/src/vault.rs` and `src-tauri/src/profile.rs`
- WHEN scanned for `#[cfg(unix)]` or `#[cfg(windows)]` attributes related to permissions
- THEN zero matches are found

### Requirement: R8 — Error propagation

The low-level `harden_file_permissions` function SHALL return `std::io::Error` and SHALL be `pub(crate)`. Callers (`vault.rs`, `profile.rs`, export flow) SHALL map the io error to `AppError` with context appropriate to their module.

#### Scenario: Vault save maps io error to AppError

- GIVEN `harden_file_permissions` returns an `std::io::Error`
- WHEN called from `vault.rs::save_to_disk`
- THEN the error is wrapped in an `AppError` variant with vault-context

### Requirement: R9 — Known limitations documented

Domain Group Policy (GPO) reassertion of inherited ACLs on Windows SHALL be documented as a known limitation in `docs/security.md` or equivalent. Cross-volume rename behavior SHALL be validated by an integration test; no copy+delete fallback SHALL be implemented since `.tmp` is co-located with its destination by design.

#### Scenario: Security docs cover GPO and FS limits

- GIVEN the security documentation file exists
- WHEN read
- THEN it contains explicit notes on GPO override, FAT32/network silent continue, and the no-cross-volume-rename design choice

## Edge Cases & Explicit Non-Requirements

- Cross-volume rename is NOT supported; the `.tmp` path lives in the same directory as the destination by construction (`path.with_extension("json.tmp")`).
- Directory-level ACL on the application data directory is NOT hardened by this capability.
- GPO reassertion of inherited ACLs is NOT counteracted; it is documented only.
- An `acl_hardened` flag is explicitly NOT introduced — re-hardening on every save and unlock is idempotent and cheap.
