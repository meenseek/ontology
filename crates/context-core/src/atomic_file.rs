use std::{fs, io, path::Path};

#[cfg(target_os = "macos")]
mod macos_acl {
    use std::{
        ffi::{CString, c_char, c_int, c_void},
        io::{self, ErrorKind},
        os::unix::ffi::OsStrExt as _,
        path::Path,
        ptr::NonNull,
    };

    const ACL_TYPE_EXTENDED: c_int = 0x0000_0100;

    unsafe extern "C" {
        fn acl_free(value: *mut c_void) -> c_int;
        fn acl_get_file(path: *const c_char, acl_type: c_int) -> *mut c_void;
        fn acl_set_file(path: *const c_char, acl_type: c_int, acl: *mut c_void) -> c_int;
        #[cfg(test)]
        fn acl_to_text(acl: *mut c_void, length: *mut isize) -> *mut c_char;
    }

    struct ExtendedAcl(NonNull<c_void>);

    impl ExtendedAcl {
        fn read(path: &Path) -> io::Result<Option<Self>> {
            let path = c_path(path)?;
            // SAFETY: `path` is a live NUL-terminated C string and the returned ACL is owned.
            let acl = unsafe { acl_get_file(path.as_ptr(), ACL_TYPE_EXTENDED) };
            let Some(acl) = NonNull::new(acl) else {
                let error = io::Error::last_os_error();
                return if error.kind() == ErrorKind::NotFound {
                    Ok(None)
                } else {
                    Err(error)
                };
            };
            Ok(Some(Self(acl)))
        }

        fn write_to(&self, path: &Path) -> io::Result<()> {
            let path = c_path(path)?;
            // SAFETY: the ACL and C path remain valid for the duration of this call.
            let result = unsafe { acl_set_file(path.as_ptr(), ACL_TYPE_EXTENDED, self.0.as_ptr()) };
            if result == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }

        #[cfg(test)]
        fn text(&self) -> io::Result<Vec<u8>> {
            let mut length = 0_isize;
            // SAFETY: the ACL is live and `length` points to writable storage for this call.
            let text = unsafe { acl_to_text(self.0.as_ptr(), &raw mut length) };
            let Some(text) = NonNull::new(text) else {
                return Err(io::Error::last_os_error());
            };
            let result = usize::try_from(length).map_or_else(
                |_| {
                    Err(io::Error::new(
                        ErrorKind::InvalidData,
                        "macOS returned a negative ACL text length",
                    ))
                },
                |length| {
                    // SAFETY: `acl_to_text` returned `length` readable bytes owned by `text`.
                    Ok(
                        unsafe { std::slice::from_raw_parts(text.as_ptr().cast(), length) }
                            .to_vec(),
                    )
                },
            );
            // SAFETY: `text` was allocated by `acl_to_text` and is freed exactly once here.
            let _ = unsafe { acl_free(text.as_ptr().cast()) };
            result
        }
    }

    impl Drop for ExtendedAcl {
        fn drop(&mut self) {
            // SAFETY: `self.0` was returned by `acl_get_file` and is freed exactly once here.
            let _ = unsafe { acl_free(self.0.as_ptr()) };
        }
    }

    fn c_path(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(
                ErrorKind::InvalidInput,
                "file path contains an interior NUL byte",
            )
        })
    }

    pub(super) fn copy_extended_acl(source: &Path, target: &Path) -> io::Result<()> {
        if let Some(acl) = ExtendedAcl::read(source)? {
            acl.write_to(target)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn extended_acl_text(path: &Path) -> io::Result<Option<Vec<u8>>> {
        ExtendedAcl::read(path)?.map(|acl| acl.text()).transpose()
    }
}

pub(crate) fn copy_permission_metadata(source: &Path, target: &Path) -> io::Result<()> {
    let permissions = fs::metadata(source)?.permissions();
    fs::set_permissions(target, permissions)?;
    #[cfg(target_os = "macos")]
    macos_acl::copy_extended_acl(source, target)?;
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn extended_acl_text(path: &Path) -> io::Result<Option<Vec<u8>>> {
    macos_acl::extended_acl_text(path)
}
