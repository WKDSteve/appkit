//! Windows: a file for the current user alone, through the security API of the system (no `icacls.exe`: a program
//! that starts the tools of the system to change rights is a pattern an endpoint protection reads as an attack, see
//! appkit/edr-ausloeser-aus-allen-nativen-apps-entfernen).
//!
//! The rights the file inherits from its folder are taken off (a protected list) and the current user gets full
//! control, as the only entry. The SID is the one of the token of this process, not a name looked up in the
//! environment (`USERNAME`, `USERDOMAIN` can say anything).
#![allow(unsafe_code)]

use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::Security::Authorization::{
    SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SET_ACCESS,
    SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE,
    PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// The user of the token of this process, as the bytes of its `TOKEN_USER` (the SID lives inside them).
struct TokenUserBuf(Vec<u64>);

impl TokenUserBuf {
    fn of_this_process() -> io::Result<Self> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: the pseudo handle of this process needs no closing; `token` receives a handle we close below.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = Self::of_token(token);
        // SAFETY: `token` is the handle OpenProcessToken gave us, closed once.
        unsafe { CloseHandle(token) };
        result
    }

    fn of_token(token: HANDLE) -> io::Result<Self> {
        let mut needed = 0u32;
        // SAFETY: a call with no buffer only asks for the size.
        unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        // u64 elements: the buffer is aligned for TOKEN_USER and the SID in it.
        let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: `buf` holds at least `needed` bytes.
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buf.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(buf))
    }

    fn sid(&self) -> *mut core::ffi::c_void {
        // SAFETY: the buffer was filled by GetTokenInformation(TokenUser) and starts with a TOKEN_USER.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Take every inherited right off `path` and give full control to the current user alone.
///
/// # Errors
///
/// What the system said; the file is then as it was.
pub fn restrict_to_user(path: &Path) -> io::Result<()> {
    let user = TokenUserBuf::of_this_process()?;
    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS,
        grfAccessMode: SET_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_USER,
            ptstrName: user.sid().cast(),
        },
    };
    let mut acl: *mut ACL = std::ptr::null_mut();
    // SAFETY: one entry, no old list; `acl` receives a list we free with LocalFree.
    let made = unsafe { SetEntriesInAclW(1, &access, std::ptr::null(), &mut acl) };
    if made != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(made as i32));
    }
    let name = wide(path);
    // SAFETY: `name` ends with a NUL; only the DACL is set, protected so that nothing is inherited.
    let set = unsafe {
        SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl,
            std::ptr::null(),
        )
    };
    // SAFETY: `acl` came from SetEntriesInAclW and is freed once.
    unsafe { LocalFree(acl.cast()) };
    if set == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(set as i32))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;
    use windows_sys::Win32::Security::{
        EqualSid, GetAce, GetSecurityDescriptorControl, ACCESS_ALLOWED_ACE, PSECURITY_DESCRIPTOR,
        SE_DACL_PROTECTED,
    };

    /// What the list of rights of `path` says: whether it is protected (inherits nothing), how many entries it has,
    /// and whether the first allows the current user full control.
    pub(crate) fn rights_of(path: &Path) -> (bool, u32, bool) {
        let name = wide(path);
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: the out pointers are ours; `sd` is freed below.
        let got = unsafe {
            GetNamedSecurityInfoW(
                name.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut sd,
            )
        };
        assert_eq!(got, ERROR_SUCCESS);
        let mut control = 0u16;
        let mut revision = 0u32;
        // SAFETY: `sd` is the descriptor GetNamedSecurityInfoW gave.
        unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) };
        // SAFETY: `dacl` points into `sd`, which lives until LocalFree.
        let count = u32::from(unsafe { (*dacl).AceCount });
        let user = TokenUserBuf::of_this_process().unwrap();
        let mut ace: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: index 0 of a list with `count` entries, read only when there is one.
        let first_is_user = count > 0
            && unsafe { GetAce(dacl, 0, &mut ace) } != 0
            && unsafe {
                let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
                allowed.Mask == FILE_ALL_ACCESS
                    && EqualSid(
                        std::ptr::addr_of!(allowed.SidStart).cast_mut().cast(),
                        user.sid(),
                    ) != 0
            };
        // SAFETY: `sd` came from GetNamedSecurityInfoW and is freed once.
        unsafe { LocalFree(sd) };
        (control & SE_DACL_PROTECTED != 0, count, first_is_user)
    }
}
