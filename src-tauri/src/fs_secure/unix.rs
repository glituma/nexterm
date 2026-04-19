// fs_secure/unix.rs — Unix permission hardening via PermissionsExt.
//
// Sets file mode to 0o600 (owner read+write only; no group, no world).

use std::io;
use std::path::Path;

/// Apply owner-only permissions (`0o600`) to the file at `path`.
///
/// # Errors
///
/// Returns `io::Error` if `set_permissions` fails (e.g., permission denied,
/// path does not exist).
///
/// # Platform behavior
///
/// Unix only. Implemented via `std::os::unix::fs::PermissionsExt::from_mode`.
pub(crate) fn harden(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
}
