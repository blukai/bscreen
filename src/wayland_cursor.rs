#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_uint};

use crate::{
    dynlib::{DynLib, opaque_struct},
    wayland,
};

opaque_struct!(wl_cursor_theme);

#[repr(C)]
pub struct wl_cursor_image {
    pub width: u32,
    pub height: u32,
    pub hotspot_x: u32,
    pub hotspot_y: u32,
    pub delay: u32,
}

#[repr(C)]
pub struct wl_cursor {
    pub image_count: c_uint,
    pub images: *mut *mut wl_cursor_image,
    pub name: *const c_char,
}

#[expect(dead_code)]
pub struct Lib {
    pub wl_cursor_theme_load: unsafe extern "C" fn(
        name: *const c_char,
        size: c_int,
        shm: *mut wayland::wl_shm,
    ) -> *mut wl_cursor_theme,
    pub wl_cursor_theme_destroy: unsafe extern "C" fn(theme: *mut wl_cursor_theme),
    pub wl_cursor_theme_get_cursor:
        unsafe extern "C" fn(theme: *mut wl_cursor_theme, name: *const c_char) -> *mut wl_cursor,
    pub wl_cursor_image_get_buffer:
        unsafe extern "C" fn(image: *mut wl_cursor_image) -> *mut wayland::wl_buffer,

    _lib: DynLib,
}

unsafe impl Sync for Lib {}
unsafe impl Send for Lib {}

impl Lib {
    pub fn load() -> anyhow::Result<Self> {
        let lib = DynLib::open(b"libwayland-cursor.so\0")
            .or_else(|_| DynLib::open(b"libwayland-cursor.so.0\0"))?;

        Ok(Self {
            wl_cursor_theme_load: lib.lookup(b"wl_cursor_theme_load\0")?,
            wl_cursor_theme_destroy: lib.lookup(b"wl_cursor_theme_destroy\0")?,
            wl_cursor_theme_get_cursor: lib.lookup(b"wl_cursor_theme_get_cursor\0")?,
            wl_cursor_image_get_buffer: lib.lookup(b"wl_cursor_image_get_buffer\0")?,

            _lib: lib,
        })
    }

    pub(crate) fn leak(self) -> &'static Self {
        Box::leak(Box::new(self))
    }
}
