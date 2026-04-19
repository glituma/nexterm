// fs_secure/fallback.rs — No-op permission hardening for non-Unix, non-Windows targets.
//
// On targets that have neither POSIX permissions nor Win32 ACLs (e.g., WASM,
// hypothetical embedded targets), hardening is a no-op. The write still
// completes successfully, and the caller can use `best_effort_harden` to
// distinguish the SkippedUnsupported outcome if needed.

use std::io;
use std::path::Path;

/// No-op permission hardening — always returns `Ok(())`.
///
/// # Platform behavior
///
/// Non-Unix, non-Windows targets only. Permission hardening is not possible
/// on these platforms; the function returns `Ok(())` to allow the write
/// to complete without error.
pub(crate) fn harden(_path: &Path) -> io::Result<()> {
    Ok(())
}
