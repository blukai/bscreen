use std::ffi::{c_char, c_int, c_void};

use crate::dynlib::DynLib;

pub const WL_MARSHAL_FLAG_DESTROY: u32 = 1 << 0;

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Debug, Clone)]
pub struct wl_message {
    pub name: *const c_char,
    pub signature: *const c_char,
    pub types: *const *const wl_interface,
}

unsafe impl Sync for wl_message {}

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Debug, Clone)]
pub struct wl_interface {
    pub name: *const c_char,
    pub version: c_int,
    pub method_count: c_int,
    pub methods: *const wl_message,
    pub event_count: c_int,
    pub events: *const wl_message,
}

unsafe impl Sync for wl_interface {}

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Debug, Clone)]
pub struct wl_proxy {
    _data: [u8; 0],
    _marker: std::marker::PhantomData<(*mut u8, std::marker::PhantomPinned)>,
}

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct wl_array {
    pub size: usize,
    pub alloc: usize,
    pub data: *mut c_void,
}

#[allow(non_camel_case_types)]
pub type wl_fixed = i32;

pub fn wl_fixed_to_f32(f: wl_fixed) -> f32 {
    (f as f32) / 256.0
}

pub struct Lib {
    pub wl_display_connect: unsafe extern "C" fn(name: *const c_char) -> *mut wl_display,
    pub wl_display_disconnect: unsafe extern "C" fn(display: *mut wl_display) -> *mut c_void,
    pub wl_display_dispatch: unsafe extern "C" fn(display: *mut wl_display) -> c_int,
    pub wl_display_roundtrip: unsafe extern "C" fn(display: *mut wl_display) -> c_int,
    pub wl_display_flush: unsafe extern "C" fn(display: *mut wl_display) -> c_int,

    pub wl_proxy_add_listener: unsafe extern "C" fn(
        proxy: *mut wl_proxy,
        implementation: *mut unsafe extern "C" fn(),
        data: *mut c_void,
    ) -> c_int,
    pub wl_proxy_destroy: unsafe extern "C" fn(proxy: *mut wl_proxy),
    pub wl_proxy_get_version: unsafe extern "C" fn(proxy: *mut wl_proxy) -> u32,
    pub wl_proxy_marshal_flags: unsafe extern "C" fn(
        proxy: *mut wl_proxy,
        opcode: u32,
        interface: *const wl_interface,
        version: u32,
        flags: u32,
        ...
    ) -> *mut wl_proxy,

    _lib: DynLib,
}

unsafe impl Sync for Lib {}
unsafe impl Send for Lib {}

impl Lib {
    pub fn load() -> anyhow::Result<Self> {
        let lib = DynLib::open(b"libwayland-client.so\0")
            .or_else(|_| DynLib::open(b"libwayland-client.so.0\0"))?;

        Ok(Self {
            wl_display_connect: lib.lookup(b"wl_display_connect\0")?,
            wl_display_disconnect: lib.lookup(b"wl_display_disconnect\0")?,
            wl_display_dispatch: lib.lookup(b"wl_display_dispatch\0")?,
            wl_display_roundtrip: lib.lookup(b"wl_display_roundtrip\0")?,
            wl_display_flush: lib.lookup(b"wl_display_flush\0")?,

            wl_proxy_add_listener: lib.lookup(b"wl_proxy_add_listener\0")?,
            wl_proxy_destroy: lib.lookup(b"wl_proxy_destroy\0")?,
            wl_proxy_get_version: lib.lookup(b"wl_proxy_get_version\0")?,
            wl_proxy_marshal_flags: lib.lookup(b"wl_proxy_marshal_flags\0")?,

            _lib: lib,
        })
    }

    pub(crate) fn leak(self) -> &'static Self {
        Box::leak(Box::new(self))
    }
}

mod generated {
    #![allow(non_camel_case_types)]
    #![allow(non_upper_case_globals)]
    #![allow(dead_code)]

    include!(concat!(env!("OUT_DIR"), "/wayland_bindings.rs"));
}
pub use generated::*;

unsafe extern "C" fn __noop_listener() {}
pub(crate) const __NOOP_LISTENER: unsafe extern "C" fn() = __noop_listener;
macro_rules! noop_listener {
    () => {
        unsafe { std::mem::transmute(crate::wayland::__NOOP_LISTENER) }
    };
}
pub(crate) use noop_listener;
