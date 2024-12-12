use std::{ffi::CString, mem::transmute_copy, ptr::NonNull};

use libc::{c_void, dlclose, dlerror, dlopen, dlsym};

pub struct DynLib(NonNull<c_void>);

impl DynLib {
    pub fn open(filename: &[u8]) -> Result<Self, String> {
        unsafe {
            let handle = dlopen(filename.as_ptr() as _, libc::RTLD_LAZY);

            if handle.is_null() {
                Err(CString::from_raw(dlerror())
                    .into_string()
                    .unwrap_or_default())
            } else {
                Ok(Self(NonNull::new_unchecked(handle)))
            }
        }
    }

    pub fn lookup<F: Sized>(&self, name: &[u8]) -> Result<F, String> {
        unsafe {
            _ = dlerror();

            let addr = dlsym(self.0.as_ptr(), name.as_ptr() as _);

            let err = dlerror();
            if !err.is_null() {
                Err(CString::from_raw(err).into_string().unwrap_or_default())
            } else {
                Ok(transmute_copy(&addr))
            }
        }
    }
}

impl Drop for DynLib {
    fn drop(&mut self) {
        unsafe {
            dlclose(self.0.as_ptr());
        }
    }
}
