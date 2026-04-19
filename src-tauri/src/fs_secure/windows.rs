// fs_secure/windows.rs — Windows ACL hardening via SetNamedSecurityInfoW.
//
// Grants only the current user SID GENERIC_ALL, stripping inherited ACEs
// via PROTECTED_DACL_SECURITY_INFORMATION.
//
// All Win32 unsafe code is isolated here. Zero #[cfg(windows)] leaks outside
// this module.
//
// The `harden` fn is called via `mod.rs::harden_impl` on Windows builds.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, GENERIC_ALL, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::{
    GetLengthSid, GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Security::Authorization::{
    SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, SE_FILE_OBJECT,
    SET_ACCESS, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows::Win32::Security::NO_INHERITANCE;
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PWSTR;

// ─── RAII Guards ─────────────────────────────────────────

/// RAII guard for a Win32 `HANDLE`.
///
/// Calls `CloseHandle` on drop if the handle is valid.
#[allow(dead_code)]
struct HandleGuard(Option<HANDLE>);

#[allow(dead_code)]
impl HandleGuard {
    /// Wrap a handle returned by `OpenProcessToken`.
    /// Stores `None` if the handle is null/invalid so `Drop` is a no-op.
    fn new(h: HANDLE) -> Self {
        if h.is_invalid() || h.0.is_null() {
            Self(None)
        } else {
            Self(Some(h))
        }
    }

    fn get(&self) -> Option<HANDLE> {
        self.0
    }
}

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            // SAFETY: `h` is a valid, open handle previously returned by
            // `OpenProcessToken`. We have exclusive ownership — no other code
            // holds a copy. `CloseHandle` frees the OS handle on return.
            unsafe {
                let _ = CloseHandle(h);
            }
        }
    }
}

/// RAII guard for a pointer allocated via Win32 `LocalAlloc`.
///
/// `SetEntriesInAclW` allocates the ACL buffer via `LocalAlloc`.
/// `GetNamedSecurityInfoW` allocates the security descriptor similarly.
/// Both must be freed with `LocalFree`.
#[allow(dead_code)]
struct LocalAllocGuard(*mut std::ffi::c_void);

impl LocalAllocGuard {
    #[allow(dead_code)]
    fn as_acl_ptr(&self) -> *mut ACL {
        self.0 as *mut ACL
    }
}

impl Drop for LocalAllocGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` was returned by `LocalAlloc` (via
            // `SetEntriesInAclW` or `GetNamedSecurityInfoW`). We have
            // exclusive ownership. `LocalFree` accepts a pointer previously
            // returned by `LocalAlloc`/`LocalReAlloc` and frees it. The
            // function handles null safely but we guard above anyway.
            unsafe {
                LocalFree(Some(HLOCAL(self.0)));
            }
            self.0 = std::ptr::null_mut();
        }
    }
}

// SAFETY: these guards wrap OS resources tied to this process. They are not
// Sync but are Send — safe to move between threads.
unsafe impl Send for LocalAllocGuard {}
unsafe impl Send for HandleGuard {}

// ─── Core helpers ─────────────────────────────────────────

/// Retrieve the current process user SID as an owned byte vector.
///
/// Uses the double-call pattern for `GetTokenInformation` to determine the
/// required buffer size before allocating.
///
/// # Errors
///
/// Returns `io::Error` (mapped from Win32 error) on failure.
fn get_current_user_sid() -> io::Result<Vec<u8>> {
    // Step 1: open the current process token for query.
    let mut token_handle = HANDLE::default();
    // SAFETY: `GetCurrentProcess()` always returns a pseudo-handle for the
    // current process (it never needs to be closed). `OpenProcessToken` fills
    // `token_handle` with a real handle if it succeeds; we wrap it in
    // `HandleGuard` immediately so it is closed on all code paths.
    unsafe {
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_handle)
            .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
    }
    let _token_guard = HandleGuard::new(token_handle);
    let token = _token_guard.get().ok_or_else(|| {
        io::Error::other("OpenProcessToken returned invalid handle")
    })?;

    // Step 2: query the buffer size needed for TOKEN_USER (sizing call).
    let mut needed: u32 = 0;
    // SAFETY: This first call with `None` buffer and zero length is the
    // documented pattern to retrieve the required buffer size. The call
    // always returns an error (ERROR_INSUFFICIENT_BUFFER); we ignore it —
    // only `needed` is meaningful here.
    let _ = unsafe {
        GetTokenInformation(token, TokenUser, None, 0, &mut needed)
    };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }

    // Step 3: allocate buffer and call again with the real size.
    let mut buf: Vec<u8> = vec![0u8; needed as usize];
    // SAFETY: `buf` has exactly `needed` bytes, matching the size returned by
    // the sizing call above. `GetTokenInformation` writes a `TOKEN_USER`
    // struct (a `SID_AND_ATTRIBUTES`) followed by the variable-length SID
    // bytes into `buf`. The alignment of `Vec<u8>` meets the requirement for
    // `TOKEN_USER` on all Windows x64/x86 ABI targets (pointer-size aligned).
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut _),
            needed,
            &mut needed,
        )
        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
    }

    // Step 4: read the SID pointer out of the TOKEN_USER struct and copy the
    // SID bytes into an owned Vec so they survive beyond the function.
    // SAFETY: `buf` was fully written by `GetTokenInformation`. The first
    // bytes of `TOKEN_USER` are a `SID_AND_ATTRIBUTES` whose first field is
    // a `PSID` (a fat pointer `*mut c_void`). We reinterpret the start of
    // `buf` as `TOKEN_USER` to read that pointer.
    let token_user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let sid_ptr: PSID = token_user.User.Sid;
    if sid_ptr.0.is_null() {
        return Err(io::Error::other("invalid SID pointer from token"));
    }

    // SAFETY: `sid_ptr` is a valid PSID within the `buf` we own. `GetLengthSid`
    // reads only the fixed-size SID revision and sub-authority count fields to
    // compute the length — it does not follow any pointers.
    let sid_len = unsafe { GetLengthSid(sid_ptr) } as usize;
    if sid_len == 0 {
        return Err(io::Error::other("SID length is zero"));
    }

    // Copy SID bytes to a standalone Vec (buf may be dropped after this fn).
    // SAFETY: `sid_ptr.0` points to `sid_len` valid bytes within `buf`.
    let sid_bytes = unsafe {
        std::slice::from_raw_parts(sid_ptr.0 as *const u8, sid_len).to_vec()
    };

    Ok(sid_bytes)
}

/// Build an `EXPLICIT_ACCESS_W` granting `GENERIC_ALL` to the SID in `sid_bytes`.
///
/// `sid_bytes` must remain valid for the lifetime of the returned struct because
/// `Trustee.ptstrName` is a raw pointer into it.
///
/// # Safety (caller obligation)
///
/// Caller must NOT drop `sid_bytes` before passing the returned struct to
/// `SetEntriesInAclW`.
fn build_explicit_access(sid_bytes: &[u8]) -> EXPLICIT_ACCESS_W {
    // SAFETY: We cast the SID pointer to `PWSTR` as required by the
    // `EXPLICIT_ACCESS_W` API. When `TrusteeForm = TRUSTEE_IS_SID`, the Win32
    // documentation says `ptstrName` must point to the SID — not a string —
    // even though the field type is `PWSTR`. This is the canonical Win32 SDK
    // approach (see MSDN "TRUSTEE structure").
    let sid_ptr = sid_bytes.as_ptr() as *mut u16;
    EXPLICIT_ACCESS_W {
        grfAccessPermissions: GENERIC_ALL.0,
        grfAccessMode: SET_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation:
                windows::Win32::Security::Authorization::NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: PWSTR(sid_ptr),
        },
    }
}

// ─── Public implementation ────────────────────────────────

/// Apply owner-only DACL to the file at `path` on Windows.
///
/// The resulting DACL contains exactly one ACE (current user SID,
/// `GENERIC_ALL`). Inherited ACEs are stripped via
/// `PROTECTED_DACL_SECURITY_INFORMATION`.
///
/// # Errors
///
/// Returns `io::Error` (raw OS error code preserved) on any Win32 failure.
pub(crate) fn harden(path: &Path) -> io::Result<()> {
    // Step 1: get the current user's SID as an owned byte vec.
    let sid_bytes = get_current_user_sid()?;

    // Step 2: build EXPLICIT_ACCESS_W for a single owner ACE.
    // `sid_bytes` must outlive `ea` — both live in this function frame.
    let ea = build_explicit_access(&sid_bytes);

    // Step 3: build a new ACL from the explicit access entry.
    let mut new_acl_ptr: *mut ACL = std::ptr::null_mut();
    // SAFETY: We pass exactly 1 entry (`ea`) to `SetEntriesInAclW`. The
    // function allocates a new ACL via `LocalAlloc` and writes its address
    // into `new_acl_ptr`. We wrap this immediately in `LocalAllocGuard` so
    // `LocalFree` is called on all paths (success and failure).
    let err = unsafe {
        SetEntriesInAclW(
            Some(std::slice::from_ref(&ea)),
            None,
            &mut new_acl_ptr,
        )
    };
    if err.0 != 0 {
        return Err(io::Error::from_raw_os_error(err.0 as i32));
    }
    let _acl_guard = LocalAllocGuard(new_acl_ptr as *mut std::ffi::c_void);

    // Step 4: convert the path to a null-terminated wide string.
    // `encode_wide` produces UTF-16 code units; we append a `0u16` null
    // terminator. The resulting `Vec<u16>` owns its memory for the
    // duration of the `SetNamedSecurityInfoW` call below.
    let mut wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0u16))
        .collect();

    // Step 5: apply the DACL to the file, stripping inherited ACEs.
    // SAFETY: `wide.as_mut_ptr()` is a valid pointer to a null-terminated
    // wide string naming the file. `new_acl_ptr` is a valid ACL returned by
    // `SetEntriesInAclW` and still live (owned by `_acl_guard`). The
    // `PROTECTED_DACL_SECURITY_INFORMATION` flag strips all inherited ACEs
    // from the final DACL, making the permission set owner-only.
    let err = unsafe {
        SetNamedSecurityInfoW(
            PWSTR(wide.as_mut_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(new_acl_ptr),
            None,
        )
    };
    if err.0 != 0 {
        return Err(io::Error::from_raw_os_error(err.0 as i32));
    }

    Ok(())
}

// ─── Windows test helpers (module-level, #[cfg(test)]) ───────────────────────
//
// Placed outside the `tests` submodule so they can be re-exported by mod.rs
// for use in vault.rs / profile.rs tests that need DACL verification without
// duplicating the complex Win32 ACE-enumeration code.

/// One Access Control Entry from the DACL.
///
/// Only compiled in test builds.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct AceInfo {
    /// Raw access mask (e.g. `GENERIC_ALL.0 = 0x10000000`)
    pub(crate) access_mask: u32,
    /// SID bytes copied from the ACE.
    pub(crate) sid_bytes: Vec<u8>,
}

/// Read the DACL of `path` and return all ACEs plus the DACL-protected flag.
///
/// Panics on any Win32 error — test-only helper, never called in production.
#[cfg(test)]
pub(crate) fn read_dacl(path: &Path) -> (Vec<AceInfo>, bool) {
    use windows::Win32::Security::{
        GetAce, GetSecurityDescriptorControl, PSECURITY_DESCRIPTOR,
    };
    use windows::Win32::Security::Authorization::GetNamedSecurityInfoW;

    let mut wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0u16))
        .collect();

    let mut sd: PSECURITY_DESCRIPTOR = PSECURITY_DESCRIPTOR::default();
    let mut dacl_ptr: *mut ACL = std::ptr::null_mut();

    // SAFETY: Wide path is null-terminated and valid for the duration of
    // the call. `GetNamedSecurityInfoW` allocates the security descriptor
    // via `LocalAlloc`; we wrap it in `LocalAllocGuard` immediately.
    let err = unsafe {
        GetNamedSecurityInfoW(
            PWSTR(wide.as_mut_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut dacl_ptr),
            None,
            &mut sd,
        )
    };
    assert_eq!(err.0, 0, "GetNamedSecurityInfoW failed with error {}", err.0);
    // Wrap so the security descriptor is freed when this function returns.
    let _sd_guard = LocalAllocGuard(sd.0 as *mut std::ffi::c_void);

    // Check SE_DACL_PROTECTED control bit.
    let mut control: u16 = 0;
    let mut revision: u32 = 0;
    // SAFETY: `sd` is a valid security descriptor returned above.
    unsafe {
        GetSecurityDescriptorControl(sd, &mut control, &mut revision)
            .expect("GetSecurityDescriptorControl should succeed");
    }
    let dacl_protected = (control & windows::Win32::Security::SE_DACL_PROTECTED.0) != 0;

    // Enumerate ACEs.
    let mut aces: Vec<AceInfo> = Vec::new();
    if dacl_ptr.is_null() {
        return (aces, dacl_protected);
    }

    // SAFETY: `dacl_ptr` points into the security descriptor buffer owned
    // by `_sd_guard`. Both are live for the rest of this function.
    let ace_count = unsafe { (*dacl_ptr).AceCount };

    for i in 0..ace_count {
        let mut ace_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        // SAFETY: `dacl_ptr` is valid. `GetAce` writes a pointer to the
        // i-th ACE within the ACL memory — lifetime tied to `_sd_guard`.
        unsafe {
            GetAce(dacl_ptr, i as u32, &mut ace_ptr)
                .expect("GetAce should succeed");
        }

        // An ACCESS_ALLOWED_ACE layout (bytes):
        //   [0]  AceType  : u8
        //   [1]  AceFlags : u8
        //   [2..3] AceSize : u16
        //   [4..7] AccessMask : u32
        //   [8..]  SID : variable
        // SAFETY: `ace_ptr` is a valid ACE inside the ACL buffer.
        let ace_bytes = ace_ptr as *const u8;
        let access_mask = unsafe { *(ace_bytes.add(4) as *const u32) };

        // Compute SID length to copy the bytes.
        // SAFETY: The SID starts at byte 8 of the ACE and is entirely
        // within the ACL buffer owned by `_sd_guard`.
        let psid = PSID(unsafe { ace_bytes.add(8) as *mut _ });
        let sid_len = unsafe { GetLengthSid(psid) } as usize;
        let sid_bytes = unsafe {
            std::slice::from_raw_parts(ace_bytes.add(8), sid_len).to_vec()
        };

        aces.push(AceInfo { access_mask, sid_bytes });
    }

    (aces, dacl_protected)
}

/// Returns the current user's SID bytes. Test-only helper.
#[cfg(test)]
pub(crate) fn current_user_sid_for_test() -> Vec<u8> {
    get_current_user_sid().expect("get_current_user_sid must succeed in tests")
}

/// Returns `true` if two SID byte slices represent the same SID. Test-only.
#[cfg(test)]
pub(crate) fn sids_equal(a: &[u8], b: &[u8]) -> bool {
    use windows::Win32::Security::EqualSid;
    // SAFETY: Both slices are valid SID buffers of the correct length as
    // returned by `GetLengthSid`. `EqualSid` performs a byte-wise
    // comparison of the SID structures.
    unsafe {
        let psid_a = PSID(a.as_ptr() as *mut _);
        let psid_b = PSID(b.as_ptr() as *mut _);
        EqualSid(psid_a, psid_b).is_ok()
    }
}

/// Assert that the file at `path` has an owner-only DACL.
///
/// Returns `(ace_count, dacl_protected, all_aces_belong_to_current_user)`.
///
/// Re-exported from `mod.rs` as `fs_secure::assert_owner_only_acl_for_test`
/// so vault.rs / profile.rs tests can call it without duplicating Win32 code.
#[cfg(test)]
pub(crate) fn assert_owner_only_acl_for_test(path: &Path) -> (usize, bool, bool) {
    let (aces, protected) = read_dacl(path);
    let owner_sid = current_user_sid_for_test();
    let all_owner = aces
        .iter()
        .all(|ace| sids_equal(&ace.sid_bytes, &owner_sid));
    (aces.len(), protected, all_owner)
}

// ─── Windows Tests ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use windows::Win32::Security::{
        CreateWellKnownSid, WELL_KNOWN_SID_TYPE, WinAuthenticatedUserSid, WinBuiltinUsersSid,
        WinWorldSid,
    };

    /// Create and return the bytes of a well-known SID.
    fn well_known_sid(kind: WELL_KNOWN_SID_TYPE) -> Vec<u8> {
        // Sizing call to determine required buffer size.
        let mut size: u32 = 0;
        // SAFETY: We pass a null PSID with size 0 — the documented pattern
        // to retrieve the required size. The call always errors; we ignore it.
        let _ = unsafe {
            CreateWellKnownSid(
                kind,
                None,
                Some(PSID(std::ptr::null_mut())),
                &mut size,
            )
        };
        assert!(size > 0, "CreateWellKnownSid sizing call returned 0");

        let mut buf = vec![0u8; size as usize];
        // SAFETY: `buf` has `size` bytes as required by the sizing call.
        unsafe {
            CreateWellKnownSid(
                kind,
                None,
                Some(PSID(buf.as_mut_ptr() as *mut _)),
                &mut size,
            )
            .expect("CreateWellKnownSid should succeed");
        }
        buf
    }

    // ── P4.1 RED — smoke test: helper can read DACL ─────

    /// P4.1/P4.2 — Verify that `read_dacl` does not panic on a freshly
    /// created file (before any hardening). The ACE count may be > 1 (default
    /// inherited ACL from the temp directory), but the helper must not crash.
    #[test]
    fn test_helper_assert_owner_only_acl_can_read_dacl() {
        let dir = TempDir::new().expect("TempDir creation");
        let path = dir.path().join("smoke.txt");
        std::fs::write(&path, b"smoke").expect("write");

        // Just call the helper; no assertion on count — only verifying it
        // doesn't panic. ACE count sanity: < 1000 ensures the return is real.
        let (aces, _protected) = read_dacl(&path);
        assert!(aces.len() < 1000, "ACE count sanity check (got {})", aces.len());
    }

    // ── P4.3 RED — single owner ACE with GENERIC_ALL ────

    /// P4.3 — After `secure_write`, the DACL has exactly 1 ACE belonging to
    /// the current user SID with access mask `GENERIC_ALL`.
    #[test]
    fn windows_secure_write_produces_single_ace() {
        let dir = TempDir::new().expect("TempDir creation");
        let path = dir.path().join("vault.json");

        super::super::secure_write(&path, b"secret").expect("secure_write");

        let (aces, _protected) = read_dacl(&path);
        assert_eq!(
            aces.len(),
            1,
            "Expected exactly 1 ACE after harden; got {}",
            aces.len()
        );

        let owner_sid = current_user_sid_for_test();
        assert!(
            sids_equal(&aces[0].sid_bytes, &owner_sid),
            "The single ACE must belong to the current user SID"
        );
        // When `GENERIC_ALL` is stored in a file-object DACL, Windows maps it
        // to `FILE_ALL_ACCESS` (0x001F01FF) via the file generic mapping.
        // We accept either value: GENERIC_ALL (set via EXPLICIT_ACCESS_W) or
        // FILE_ALL_ACCESS (the mapped/stored form).
        const FILE_ALL_ACCESS: u32 = 0x001F01FF; // windows::Win32::Storage::FileSystem::FILE_ALL_ACCESS
        assert!(
            aces[0].access_mask == GENERIC_ALL.0 || aces[0].access_mask == FILE_ALL_ACCESS,
            "ACE access mask must be GENERIC_ALL (0x{:08X}) or FILE_ALL_ACCESS (0x001F01FF), got 0x{:08X}",
            GENERIC_ALL.0, aces[0].access_mask
        );
    }

    // ── P4.4 RED — no well-known group SIDs in DACL ─────

    /// P4.4 — After `secure_write`, the DACL contains no ACE for
    /// Everyone (S-1-1-0), Users (S-1-5-32-545), or Authenticated Users
    /// (S-1-5-11).
    #[test]
    fn windows_secure_write_no_well_known_sids() {
        let dir = TempDir::new().expect("TempDir creation");
        let path = dir.path().join("profiles.json");

        super::super::secure_write(&path, b"profiles").expect("secure_write");

        let (aces, _protected) = read_dacl(&path);

        let everyone = well_known_sid(WinWorldSid);
        let users = well_known_sid(WinBuiltinUsersSid);
        let auth_users = well_known_sid(WinAuthenticatedUserSid);

        for ace in &aces {
            assert!(
                !sids_equal(&ace.sid_bytes, &everyone),
                "ACE must NOT grant access to Everyone (S-1-1-0)"
            );
            assert!(
                !sids_equal(&ace.sid_bytes, &users),
                "ACE must NOT grant access to Users (S-1-5-32-545)"
            );
            assert!(
                !sids_equal(&ace.sid_bytes, &auth_users),
                "ACE must NOT grant access to Authenticated Users (S-1-5-11)"
            );
        }
    }

    // ── P4.5 RED — SE_DACL_PROTECTED bit is set ──────────

    /// P4.5 — After `secure_write`, the `SE_DACL_PROTECTED` control bit is
    /// set on the security descriptor (no inherited ACEs).
    #[test]
    fn windows_secure_write_protected_dacl_set() {
        let dir = TempDir::new().expect("TempDir creation");
        let path = dir.path().join("protected_check.json");

        super::super::secure_write(&path, b"data").expect("secure_write");

        let (_aces, dacl_protected) = read_dacl(&path);
        assert!(
            dacl_protected,
            "SE_DACL_PROTECTED must be set after harden (inherited ACEs stripped)"
        );
    }

    // ── P4.10 RED — rename preserves DACL on NTFS ────────

    /// P4.10/P4.11 — After `secure_write` to a `.tmp` path and then
    /// `std::fs::rename` to the final path on the same NTFS volume, the
    /// DACL is preserved (owner-only, protected).
    #[test]
    fn windows_rename_preserves_dacl() {
        let dir = TempDir::new().expect("TempDir creation");
        let tmp_path = dir.path().join("foo.txt.tmp");
        let final_path = dir.path().join("foo.txt");

        // Harden the .tmp file first.
        super::super::secure_write(&tmp_path, b"content").expect("secure_write to tmp");

        let (aces_before, protected_before) = read_dacl(&tmp_path);
        assert_eq!(aces_before.len(), 1, "tmp should have 1 ACE after harden");
        assert!(protected_before, "tmp should have SE_DACL_PROTECTED");

        // Same-volume rename — NTFS must preserve the DACL.
        std::fs::rename(&tmp_path, &final_path).expect("rename");

        let (aces_after, protected_after) = read_dacl(&final_path);
        let owner_sid = current_user_sid_for_test();

        assert_eq!(
            aces_after.len(),
            1,
            "Renamed file must retain exactly 1 ACE; got {}",
            aces_after.len()
        );
        assert!(
            sids_equal(&aces_after[0].sid_bytes, &owner_sid),
            "ACE after rename must still belong to the current user SID"
        );
        assert!(
            protected_after,
            "SE_DACL_PROTECTED must be preserved after rename on NTFS"
        );
    }

    // ── P4.12 RED — is_unsupported classification ─────────

    /// P4.12/P4.13 — Verify `is_unsupported` correctly classifies:
    /// - `ErrorKind::Unsupported` → true
    /// - raw OS error 1 (ERROR_INVALID_FUNCTION) → true
    /// - raw OS error 50 (ERROR_NOT_SUPPORTED) → true
    /// - raw OS error 5 (ERROR_ACCESS_DENIED) → false
    #[test]
    fn is_unsupported_returns_true_for_unsupported_errors() {
        let e1 = io::Error::from(io::ErrorKind::Unsupported);
        assert!(
            super::super::is_unsupported_pub_for_test(&e1),
            "ErrorKind::Unsupported must be classified as unsupported"
        );

        let e2 = io::Error::from_raw_os_error(1);
        assert!(
            super::super::is_unsupported_pub_for_test(&e2),
            "OS error 1 (ERROR_INVALID_FUNCTION) must be classified as unsupported"
        );

        let e3 = io::Error::from_raw_os_error(50);
        assert!(
            super::super::is_unsupported_pub_for_test(&e3),
            "OS error 50 (ERROR_NOT_SUPPORTED) must be classified as unsupported"
        );

        let e4 = io::Error::from_raw_os_error(5); // ERROR_ACCESS_DENIED
        assert!(
            !super::super::is_unsupported_pub_for_test(&e4),
            "OS error 5 (ERROR_ACCESS_DENIED) must NOT be classified as unsupported"
        );
    }

    // ── P4.16 RED — best_effort_harden returns Hardened ───

    /// P4.16 — On a real writable file in a temp dir, `best_effort_harden`
    /// returns `BestEffortOutcome::Hardened`.
    #[test]
    fn best_effort_harden_returns_hardened_on_success() {
        use super::super::{best_effort_harden, BestEffortOutcome};

        let dir = TempDir::new().expect("TempDir creation");
        let path = dir.path().join("harden_success.json");
        std::fs::write(&path, b"data").expect("write");

        let outcome = best_effort_harden(&path);
        assert!(
            matches!(outcome, BestEffortOutcome::Hardened),
            "Expected Hardened but got {:?}",
            outcome
        );
    }

    // ── P4.15 RED — best_effort_harden returns Failed ─────

    /// P4.15 — On a non-existent path, `best_effort_harden` returns
    /// `BestEffortOutcome::Failed` (ERROR_FILE_NOT_FOUND = 2, not unsupported).
    #[test]
    fn best_effort_harden_returns_failed_on_nonexistent_path() {
        use super::super::{best_effort_harden, BestEffortOutcome};

        let dir = TempDir::new().expect("TempDir creation");
        let path = dir.path().join("does_not_exist.json");

        let outcome = best_effort_harden(&path);
        assert!(
            matches!(outcome, BestEffortOutcome::Failed(_)),
            "Expected Failed but got {:?}",
            outcome
        );
    }

    // ── P4.14 RED — best_effort_harden returns SkippedUnsupported ─────

    /// P4.14 — When the underlying harden call would return an "unsupported"
    /// error (e.g. OS error 50 for a network share / FAT32), `best_effort_harden`
    /// returns `BestEffortOutcome::SkippedUnsupported`.
    ///
    /// We use the `best_effort_harden_with_result_for_test` test-seam to inject
    /// the error, because forcing `SetNamedSecurityInfoW` to return ERROR_NOT_SUPPORTED
    /// on an NTFS volume is not feasible in a unit test without a real FAT32 device.
    #[test]
    fn best_effort_harden_returns_skipped_unsupported_for_os_error_50() {
        use super::super::{best_effort_harden_with_result_for_test, BestEffortOutcome};

        let outcome = best_effort_harden_with_result_for_test(
            Err(io::Error::from_raw_os_error(50))
        );
        assert!(
            matches!(outcome, BestEffortOutcome::SkippedUnsupported),
            "Expected SkippedUnsupported for OS error 50, got {:?}",
            outcome
        );
    }

    // ── P4.14 triangulation — ErrorKind::Unsupported also routes to SkippedUnsupported

    #[test]
    fn best_effort_harden_returns_skipped_unsupported_for_error_kind_unsupported() {
        use super::super::{best_effort_harden_with_result_for_test, BestEffortOutcome};

        let outcome = best_effort_harden_with_result_for_test(
            Err(io::Error::from(io::ErrorKind::Unsupported))
        );
        assert!(
            matches!(outcome, BestEffortOutcome::SkippedUnsupported),
            "Expected SkippedUnsupported for ErrorKind::Unsupported, got {:?}",
            outcome
        );
    }
}
