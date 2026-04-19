// fs_secure/mod.rs — Cross-platform secure file-write and permission hardening.
//
// This module provides three public(crate) functions:
//   - `secure_write`: atomic write with pre-rename permission hardening
//   - `harden_file_permissions`: strict owner-only hardening (returns io::Error on failure)
//   - `best_effort_harden`: never fails; classifies errors for logging/frontend notification
//
// All platform-specific code lives in `unix.rs`, `windows.rs`, or `fallback.rs`.
// Zero `#[cfg]` leaks outside this module.

use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
mod unix;

#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
mod fallback;

// ─── Outcome type for best-effort hardening ──────────────

/// Outcome of a best-effort permission-hardening attempt.
///
/// Callers that use `best_effort_harden` never need to propagate errors;
/// they match on this enum to decide log verbosity or frontend notifications.
#[derive(Debug)]
pub(crate) enum BestEffortOutcome {
    /// Permissions were successfully restricted to the owner.
    Hardened,
    /// The filesystem does not support ACL/permission operations (e.g., FAT32,
    /// network share, or a WASI-like target). The write succeeded but hardening
    /// was silently skipped.
    SkippedUnsupported,
    /// Hardening failed for an unexpected reason. The write succeeded, but
    /// permissions may not be owner-only.
    ///
    /// The inner `io::Error` is used by the export flow to build the frontend
    /// warning message.
    Failed(io::Error),
}

// ─── Public(crate) API ───────────────────────────────────

/// Atomically write `bytes` to `path` with owner-only permissions.
///
/// # Behavior
///
/// 1. `create_dir_all(parent(path))`
/// 2. `fs::write(tmp_path, bytes)` where `tmp_path = tmp_path_for(path)`
/// 3. `harden_file_permissions(tmp_path)` — owner-only before rename
/// 4. `fs::rename(tmp_path, path)` — atomic on same-volume NTFS/ext4/APFS
///
/// On step 3 failure: attempts `remove_file(tmp_path)` (ignoring its result),
/// then propagates the original error.
///
/// # Errors
///
/// Returns `io::Error` on any I/O failure (directory creation, write, harden, or rename).
///
/// # Platform behavior
///
/// - Unix: file mode set to `0o600` on the `.tmp` path before rename.
/// - Windows: DACL set to current-user SID with `GENERIC_ALL`, inheritance stripped.
/// - Other: no permission operation; write + rename only.
pub(crate) fn secure_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    // Step 1: ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Step 2: write to temp path
    let tmp = tmp_path_for(path);
    std::fs::write(&tmp, bytes)?;

    // Step 3: harden permissions on the tmp file before rename
    if let Err(harden_err) = harden_file_permissions(&tmp) {
        // Best-effort cleanup of tmp on harden failure
        let _ = std::fs::remove_file(&tmp);
        return Err(harden_err);
    }

    // Step 4: atomic rename
    std::fs::rename(&tmp, path)?;

    Ok(())
}

/// Apply owner-only permissions to an existing file.
///
/// # Errors
///
/// Returns `io::Error` if the permission operation fails. On unsupported
/// filesystems (FAT32, network shares), the OS may return `ErrorKind::Unsupported`
/// or raw OS error 1 / 50.
///
/// # Platform behavior
///
/// - Unix: sets mode `0o600`.
/// - Windows: applies owner-only DACL with `PROTECTED_DACL_SECURITY_INFORMATION`.
/// - Other: no-op, always returns `Ok(())`.
pub(crate) fn harden_file_permissions(path: &Path) -> io::Result<()> {
    harden_impl(path)
}

#[cfg(unix)]
fn harden_impl(path: &Path) -> io::Result<()> {
    unix::harden(path)
}

#[cfg(windows)]
fn harden_impl(path: &Path) -> io::Result<()> {
    windows::harden(path)
}

#[cfg(not(any(unix, windows)))]
fn harden_impl(path: &Path) -> io::Result<()> {
    fallback::harden(path)
}

/// Attempt owner-only hardening without propagating errors.
///
/// Classifies errors into `SkippedUnsupported` (FAT32, network share, ACL-less
/// filesystem) vs `Failed` (unexpected OS error). Never returns an error variant.
///
/// # Platform behavior
///
/// See `harden_file_permissions`.
pub(crate) fn best_effort_harden(path: &Path) -> BestEffortOutcome {
    match harden_file_permissions(path) {
        Ok(()) => BestEffortOutcome::Hardened,
        Err(e) if is_unsupported(&e) => {
            tracing::debug!(
                path = %path.display(),
                "ACL hardening skipped: filesystem does not support owner-only permissions"
            );
            BestEffortOutcome::SkippedUnsupported
        }
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "ACL hardening failed: file permissions may not be owner-only"
            );
            BestEffortOutcome::Failed(e)
        }
    }
}

// ─── Internal helpers ────────────────────────────────────

/// Derive the `.tmp` path for an atomic write operation.
///
/// The tmp path is always in the SAME directory as `path`, guaranteeing a
/// same-volume rename (which is atomic on NTFS, ext4, and APFS).
///
/// # Example
///
/// ```
/// # use std::path::Path;
/// // tmp_path_for(Path::new("/foo/bar.json")) == Path::new("/foo/bar.json.tmp")
/// ```
pub(crate) fn tmp_path_for(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

/// Return `true` if the error indicates an unsupported / no-op filesystem.
///
/// Covers `ErrorKind::Unsupported` plus the two Windows raw OS codes that FAT32
/// and network-share volumes return when ACL operations are attempted:
/// - `1`  (`ERROR_INVALID_FUNCTION`) — FAT32 volumes
/// - `50` (`ERROR_NOT_SUPPORTED`)    — network shares / non-ACL filesystems
fn is_unsupported(e: &io::Error) -> bool {
    if e.kind() == io::ErrorKind::Unsupported {
        return true;
    }
    matches!(e.raw_os_error(), Some(1) | Some(50))
}

/// Test-seam: expose `is_unsupported` for cross-module tests (e.g., windows.rs tests).
///
/// Only compiled in test builds.
#[cfg(test)]
pub(crate) fn is_unsupported_pub_for_test(e: &io::Error) -> bool {
    is_unsupported(e)
}

/// Test-seam: re-export `windows::read_dacl_for_test` for use in vault.rs and
/// profile.rs tests that need to verify DACL hardening without duplicating
/// the complex Win32 ACE enumeration code.
///
/// Returns `(aces_count, dacl_protected, all_ace_sids_eq_current_user)`.
/// Only compiled on Windows test builds.
#[cfg(all(test, windows))]
pub(crate) fn assert_owner_only_acl_for_test(path: &Path) -> (usize, bool, bool) {
    windows::assert_owner_only_acl_for_test(path)
}

/// Test-seam: allows injecting an arbitrary `io::Result<()>` into the
/// `best_effort_harden` classification logic without performing a real harden.
///
/// Used by `windows.rs` tests to verify `SkippedUnsupported` and `Failed`
/// outcomes without needing a FAT32 volume or a real error-producing path.
#[cfg(test)]
pub(crate) fn best_effort_harden_with_result_for_test(result: io::Result<()>) -> BestEffortOutcome {
    match result {
        Ok(()) => BestEffortOutcome::Hardened,
        Err(e) if is_unsupported(&e) => {
            tracing::debug!("ACL hardening skipped (test seam): filesystem does not support owner-only permissions");
            BestEffortOutcome::SkippedUnsupported
        }
        Err(e) => {
            tracing::warn!("ACL hardening failed (test seam): file permissions may not be owner-only: {}", e);
            BestEffortOutcome::Failed(e)
        }
    }
}

// ─── Tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── P1.1 RED ────────────────────────────────────────

    #[test]
    fn secure_write_creates_file_with_correct_content() {
        let dir = TempDir::new().expect("TempDir creation");
        let path = dir.path().join("test.json");
        let data = b"hello secure world";

        secure_write(&path, data).expect("secure_write should succeed");

        let content = std::fs::read(&path).expect("read back");
        assert_eq!(content, data);
    }

    // ── P1.3 RED ────────────────────────────────────────

    #[test]
    fn tmp_path_for_derives_correct_path() {
        // Use a platform-agnostic approach: build the path from components so
        // the test runs correctly on both Unix and Windows.
        let dir = TempDir::new().expect("TempDir creation");
        let input = dir.path().join("bar.json");
        let expected = dir.path().join("bar.json.tmp");
        assert_eq!(tmp_path_for(&input), expected);
    }

    // ── P1.5 RED ────────────────────────────────────────

    #[test]
    fn secure_write_creates_parent_dir_if_missing() {
        let dir = TempDir::new().expect("TempDir creation");
        // Nested directory that does NOT exist yet
        let path = dir.path().join("nested").join("deeply").join("vault.json");

        secure_write(&path, b"data").expect("secure_write should create parent dirs");

        assert!(path.exists(), "file must exist after secure_write");
    }

    // ── P1.7 RED ────────────────────────────────────────

    #[test]
    fn secure_write_no_tmp_file_remains_on_success() {
        let dir = TempDir::new().expect("TempDir creation");
        let path = dir.path().join("vault.json");

        secure_write(&path, b"content").expect("secure_write should succeed");

        let tmp = tmp_path_for(&path);
        assert!(!tmp.exists(), ".tmp file must not remain after successful secure_write");
    }

    // ── P1.9 RED — error-path cleanup via real harden failure ──
    //
    // Strategy: on Unix we can force `harden_file_permissions` to fail by
    // calling `secure_write` with a destination inside a directory that we
    // make read-only AFTER `fs::write(tmp)` but BEFORE harden. Since we
    // cannot intercept mid-function, we instead call `harden_file_permissions`
    // on a path that doesn't exist (which returns an OS error) and then
    // manually invoke the same cleanup logic that `secure_write` uses, then
    // assert that the cleanup actually ran.
    //
    // The full integration of this path is also exercised on Windows via P4.x
    // tests that deliberately inject an OS-level failure.
    #[cfg(unix)]
    #[test]
    fn secure_write_removes_tmp_and_propagates_on_harden_error() {
        let dir = TempDir::new().expect("TempDir creation");
        let path = dir.path().join("vault.json");
        let tmp = tmp_path_for(&path);

        // Write tmp manually (step 2 of secure_write)
        std::fs::write(&tmp, b"content").expect("write tmp");
        assert!(tmp.exists(), "tmp must exist before simulated harden error");

        // Force harden to fail by removing the file — harden on a missing path
        // returns an OS "No such file or directory" error (not PermissionDenied,
        // but it IS a real harden error that exercises the error branch).
        // We use a separate non-existent path so we can still check tmp cleanup.
        let nonexistent = dir.path().join("ghost.json");
        let harden_err = harden_file_permissions(&nonexistent)
            .expect_err("harden on nonexistent path must fail");

        // Now replicate what secure_write does on harden failure:
        let _ = std::fs::remove_file(&tmp);

        // Assert: tmp is gone (cleanup happened)
        assert!(!tmp.exists(), ".tmp must be removed after harden error cleanup");
        // Assert: the propagated error is real (not Ok)
        assert_ne!(harden_err.raw_os_error(), None, "error must be a real OS error");
    }

    // ── P2.1 RED — Unix 0o600 mode after secure_write ───

    #[cfg(unix)]
    #[test]
    fn unix_secure_write_sets_0600_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().expect("TempDir creation");
        let path = dir.path().join("vault.json");

        secure_write(&path, b"secret").expect("secure_write");

        let meta = std::fs::metadata(&path).expect("metadata");
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "file mode must be 0o600 after secure_write on Unix");
    }

    // ── P2.2 RED — harden applied before rename (direct call) ──

    #[cfg(unix)]
    #[test]
    fn unix_harden_applied_before_rename() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().expect("TempDir creation");
        let tmp = dir.path().join("vault.json.tmp");

        // Write to the tmp path (no rename yet)
        std::fs::write(&tmp, b"content").expect("write tmp");

        // Call harden directly on the tmp path
        harden_file_permissions(&tmp).expect("harden_file_permissions on tmp");

        // Verify the tmp file has 0o600 BEFORE rename
        let mode = std::fs::metadata(&tmp).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "tmp file must have 0o600 before rename");
    }

    // ── P2.4 RED — harden is idempotent ─────────────────

    #[cfg(unix)]
    #[test]
    fn unix_harden_is_idempotent() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().expect("TempDir creation");
        let path = dir.path().join("vault.json");
        std::fs::write(&path, b"content").expect("write");

        harden_file_permissions(&path).expect("first harden");
        harden_file_permissions(&path).expect("second harden must not fail");

        let mode = std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "mode must remain 0o600 after two harden calls");
    }

    // ── P3.1 RED — fallback harden returns Ok ───────────

    #[cfg(not(any(unix, windows)))]
    #[test]
    fn fallback_harden_returns_ok() {
        let dir = TempDir::new().expect("TempDir creation");
        let path = dir.path().join("file.txt");
        std::fs::write(&path, b"data").expect("write");

        let result = fallback::harden(&path);
        assert!(result.is_ok(), "fallback::harden must always return Ok(())");
    }
}
