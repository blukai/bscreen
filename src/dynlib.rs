use std::ffi::CString;
use std::mem::transmute_copy;
use std::ptr::NonNull;

use libc::{c_void, dlclose, dlerror, dlopen, dlsym};

pub(crate) struct DynLib(NonNull<c_void>);

impl DynLib {
    pub(crate) fn open(filename: &[u8]) -> Result<Self, String> {
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

    pub(crate) fn lookup<F: Sized>(&self, name: &[u8]) -> Result<F, String> {
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

macro_rules! opaque_struct {
    ($name:ident) => {
        #[repr(C)]
        pub(crate) struct $name {
            _data: [u8; 0],
            _marker: PhantomData<(*mut u8, PhantomPinned)>,
        }
    };
}
pub(crate) use opaque_struct;
