//! Restrict temporary file DACLs at creation to avoid inheriting broader directory read access.

use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::Path;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: The pointer comes from a Windows conversion function requiring LocalFree.
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn current_user_sid() -> io::Result<String> {
    let mut raw = null_mut();
    // SAFETY: The process pseudo-handle and output pointer are valid;
    // OwnedHandle takes ownership of the token on success.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut length = 0;
    unsafe {
        GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut length);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
        return Err(error);
    }
    if (length as usize) < size_of::<TOKEN_USER>() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    // TOKEN_USER contains pointers; use usize for alignment and query the buffer size from Windows.
    let mut buffer = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let mut sid = null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _allocation = LocalAllocation(sid.cast());
    let mut length = 0;
    // SAFETY: Successful conversion returns NUL-terminated UTF-16;
    // the allocation remains alive until the copy completes.
    unsafe {
        while *sid.add(length) != 0 {
            length += 1;
        }
        Ok(String::from_utf16_lossy(std::slice::from_raw_parts(
            sid, length,
        )))
    }
}

pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
    // Specify the user SID explicitly: an elevated process's default owner may be Administrators.
    // P disables parent ACL inheritance; grant access only to the current user and SYSTEM.
    let user = current_user_sid()?;
    let sddl: Vec<u16> = format!("O:{user}D:P(A;;FA;;;{user})(A;;FA;;;SY)")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let _descriptor = LocalAllocation(descriptor);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // Support absolute paths like std file operations; canonicalizing only the parent
    // does not require the new file to exist.
    let parent = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let mut wide: Vec<u16> = parent.join(name).as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    wide.push(0);
    // SAFETY: The path and security descriptor remain valid during the call;
    // CREATE_NEW prevents overwriting an existing file.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: Creation succeeded; File takes sole ownership of the handle.
    // The security descriptor can be freed immediately.
    Ok(unsafe { File::from_raw_handle(handle) })
}
