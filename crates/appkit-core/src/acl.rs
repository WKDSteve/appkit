//! Windows: a file for the current user alone, through the security API of the system (no `icacls.exe`: a program
//! that starts the tools of the system to change rights is a pattern an endpoint protection reads as an attack, see
//! appkit/edr-ausloeser-aus-allen-nativen-apps-entfernen).
//!
//! The file is created new with its rights from the first moment: a protected list (nothing inherited from the
//! folder) whose only entry gives the current user full control. Changing the rights of an existing file afterwards
//! is not enough: a program that opened it before keeps its handle. The SID is the one of the token of this process,
//! not a name looked up in the environment (`USERNAME`, `USERDOMAIN` can say anything).
#![allow(unsafe_code)]

use std::io;
use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;

use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, ERROR_SUCCESS, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    SetEntriesInAclW, EXPLICIT_ACCESS_W, NO_MULTIPLE_TRUSTEE, SET_ACCESS, TRUSTEE_IS_SID,
    TRUSTEE_IS_USER, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, InitializeSecurityDescriptor, SetSecurityDescriptorControl,
    SetSecurityDescriptorDacl, TokenUser, ACL, NO_INHERITANCE, PSECURITY_DESCRIPTOR,
    SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, CREATE_NEW, FILE_ALL_ACCESS, FILE_ATTRIBUTE_NORMAL,
};
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

/// The list of rights "the current user, full control, nothing else", made by `SetEntriesInAclW`; freed on drop.
struct UserOnlyAcl(*mut ACL);

impl UserOnlyAcl {
    fn new() -> io::Result<Self> {
        let user = TokenUserBuf::of_this_process()?;
        let access = user_only_access(&user);
        let mut acl: *mut ACL = std::ptr::null_mut();
        // SAFETY: one entry, no old list; `acl` receives a list freed in Drop. The SID in `access` points into `user`,
        // which lives until the call returns (the list copies it).
        let made = unsafe { SetEntriesInAclW(1, &access, std::ptr::null(), &mut acl) };
        if made != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(made as i32));
        }
        Ok(Self(acl))
    }
}

impl Drop for UserOnlyAcl {
    fn drop(&mut self) {
        // SAFETY: the list came from SetEntriesInAclW and is freed once.
        unsafe { LocalFree(self.0.cast()) };
    }
}

fn user_only_access(user: &TokenUserBuf) -> EXPLICIT_ACCESS_W {
    EXPLICIT_ACCESS_W {
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
    }
}

/// `SECURITY_DESCRIPTOR_REVISION` (winnt.h); its feature module of `windows-sys` is not one this crate pulls in.
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;

/// Create `path` as a NEW file that is the current user's alone from its first moment, and write `content` into it.
///
/// Never an old file: a program that opened it before (the folder's inherited rights may let it) keeps its handle when
/// the rights change afterwards and would read what is written through it. `CREATE_NEW` fails when a file of that
/// name is there; nobody may share the file while it is written. A failed write removes the new file.
///
/// # Errors
///
/// What the system said (a file of that name exists: `AlreadyExists`).
pub fn create_user_only(path: &Path, content: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    use std::os::windows::io::FromRawHandle as _;

    let acl = UserOnlyAcl::new()?;
    // SAFETY: an all-zero SECURITY_DESCRIPTOR is plain data that InitializeSecurityDescriptor fills.
    let mut descriptor: SECURITY_DESCRIPTOR = unsafe { std::mem::zeroed() };
    let sd: PSECURITY_DESCRIPTOR = std::ptr::addr_of_mut!(descriptor).cast();
    // SAFETY: `sd` points at `descriptor`, which outlives CreateFileW; the DACL is `acl`, which does too.
    let described = unsafe {
        InitializeSecurityDescriptor(sd, SECURITY_DESCRIPTOR_REVISION) != 0
            && SetSecurityDescriptorDacl(sd, 1, acl.0, 0) != 0
            && SetSecurityDescriptorControl(sd, SE_DACL_PROTECTED, SE_DACL_PROTECTED) != 0
    };
    if !described {
        return Err(io::Error::last_os_error());
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    };
    let name = wide(path);
    // SAFETY: `name` ends with a NUL; `attributes` and the descriptor it names live on this stack frame.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            0,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a valid handle CreateFileW just gave us; the File owns and closes it.
    let mut file = unsafe { std::fs::File::from_raw_handle(handle) };
    let written = file.write_all(content);
    drop(file);
    if written.is_err() {
        let _ = std::fs::remove_file(path);
    }
    written
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;
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
