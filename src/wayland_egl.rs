#![allow(non_camel_case_types)]

use std::ffi::c_int;

use crate::{
    dynlib::{DynLib, opaque_struct},
    wayland,
};

opaque_struct!(wl_egl_window);

pub struct Lib {
    pub wl_egl_window_create: unsafe extern "C" fn(
        surface: *mut wayland::wl_surface,
        width: c_int,
        height: c_int,
    ) -> *mut wl_egl_window,
    pub wl_egl_window_destroy: unsafe extern "C" fn(egl_window: *mut wl_egl_window),

    _lib: DynLib,
}

unsafe impl Sync for Lib {}
unsafe impl Send for Lib {}

impl Lib {
    pub fn load() -> anyhow::Result<Self> {
        let lib = DynLib::open(b"libwayland-egl.so\0")
            .or_else(|_| DynLib::open(b"libwayland-egl.so.1\0"))?;

        Ok(Self {
            wl_egl_window_create: lib.lookup(b"wl_egl_window_create\0")?,
            wl_egl_window_destroy: lib.lookup(b"wl_egl_window_destroy\0")?,

            _lib: lib,
        })
    }

    pub(crate) fn leak(self) -> &'static Self {
        Box::leak(Box::new(self))
    }
}
