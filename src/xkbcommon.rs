#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int};
use std::ptr::null_mut;

use anyhow::anyhow;

use crate::dynlib::{DynLib, opaque_struct};
use crate::input::KeyboardMods;

pub const XKB_MOD_NAME_CTRL: &[u8] = b"Control\0";

opaque_struct!(xkb_context);
opaque_struct!(xkb_keymap);
opaque_struct!(xkb_state);

pub type xkb_layout_index_t = u32;
pub type xkb_mod_index_t = u32;
pub type xkb_mod_mask_t = u32;

#[expect(dead_code)]
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum xkb_context_flags {
    XKB_CONTEXT_NO_FLAGS = 0,
    XKB_CONTEXT_NO_DEFAULT_INCLUDES = (1 << 0),
    XKB_CONTEXT_NO_ENVIRONMENT_NAMES = (1 << 1),
}

#[expect(dead_code)]
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum xkb_keymap_format {
    XKB_KEYMAP_USE_ORIGINAL_FORMAT = 0,
    XKB_KEYMAP_FORMAT_TEXT_V1 = 1,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum xkb_keymap_compile_flags {
    XKB_KEYMAP_COMPILE_NO_FLAGS = 0,
}

#[expect(dead_code)]
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum xkb_state_component {
    XKB_STATE_MODS_DEPRESSED = (1 << 0),
    XKB_STATE_MODS_LATCHED = (1 << 1),
    XKB_STATE_MODS_LOCKED = (1 << 2),
    XKB_STATE_MODS_EFFECTIVE = (1 << 3),
    XKB_STATE_LAYOUT_DEPRESSED = (1 << 4),
    XKB_STATE_LAYOUT_LATCHED = (1 << 5),
    XKB_STATE_LAYOUT_LOCKED = (1 << 6),
    XKB_STATE_LAYOUT_EFFECTIVE = (1 << 7),
    XKB_STATE_LEDS = (1 << 8),
}

pub struct Lib {
    _lib: DynLib,
    pub xkb_context_new: unsafe extern "C" fn(flags: xkb_context_flags) -> *mut xkb_context,
    pub xkb_context_unref: unsafe extern "C" fn(context: *mut xkb_context),
    pub xkb_keymap_mod_get_index:
        unsafe extern "C" fn(keymap: *mut xkb_keymap, name: *const c_char) -> xkb_mod_index_t,
    pub xkb_keymap_new_from_string: unsafe extern "C" fn(
        ctx: *mut xkb_context,
        string: *const c_char,
        format: xkb_keymap_format,
        flags: xkb_keymap_compile_flags,
    ) -> *mut xkb_keymap,
    pub xkb_keymap_unref: unsafe extern "C" fn(keymap: *mut xkb_keymap),
    pub xkb_state_mod_index_is_active: unsafe extern "C" fn(
        state: *mut xkb_state,
        idx: xkb_mod_index_t,
        ty: xkb_state_component,
    ) -> c_int,
    pub xkb_state_new: unsafe extern "C" fn(keymap: *mut xkb_keymap) -> *mut xkb_state,
    pub xkb_state_unref: unsafe extern "C" fn(state: *mut xkb_state),
    pub xkb_state_update_mask: unsafe extern "C" fn(
        state: *mut xkb_state,
        base_mods: xkb_mod_mask_t,
        latched_mods: xkb_mod_mask_t,
        locked_mods: xkb_mod_mask_t,
        base_group: xkb_layout_index_t,
        latched_group: xkb_layout_index_t,
        locked_group: xkb_layout_index_t,
    ) -> c_int, // xkb_state_component
}

impl Lib {
    pub fn load() -> anyhow::Result<Self> {
        let lib = DynLib::open(b"libxkbcommon.so\0")
            .or_else(|_| DynLib::open(b"libxkbcommon.so.0\0"))
            .or_else(|_| DynLib::open(b"libxkbcommon.so.0.0.0\0"))?;
        Ok(Self {
            xkb_context_new: lib.lookup(b"xkb_context_new\0")?,
            xkb_context_unref: lib.lookup(b"xkb_context_unref\0")?,
            xkb_keymap_mod_get_index: lib.lookup(b"xkb_keymap_mod_get_index\0")?,
            xkb_keymap_new_from_string: lib.lookup(b"xkb_keymap_new_from_string\0")?,
            xkb_keymap_unref: lib.lookup(b"xkb_keymap_unref\0")?,
            xkb_state_mod_index_is_active: lib.lookup(b"xkb_state_mod_index_is_active\0")?,
            xkb_state_new: lib.lookup(b"xkb_state_new\0")?,
            xkb_state_unref: lib.lookup(b"xkb_state_unref\0")?,
            xkb_state_update_mask: lib.lookup(b"xkb_state_update_mask\0")?,
            _lib: lib,
        })
    }

    pub(crate) fn leak(self) -> &'static Self {
        Box::leak(Box::new(self))
    }
}

#[derive(Debug)]
pub struct KeyboardModIndices {
    pub ctrl: xkb_mod_index_t,
}

pub struct Context {
    xkbcommon: &'static Lib,
    pub context: *mut xkb_context,
    pub keymap: *mut xkb_keymap,
    pub state: *mut xkb_state,
    pub mod_indices: KeyboardModIndices,
    pub mods: KeyboardMods,
}

impl Context {
    pub unsafe fn from_fd(
        xkbcommon_lib: &'static Lib,
        fd: c_int,
        size: u32,
    ) -> anyhow::Result<Self> {
        let context = (xkbcommon_lib.xkb_context_new)(xkb_context_flags::XKB_CONTEXT_NO_FLAGS);
        if context.is_null() {
            return Err(anyhow!("xkb_context_new failed"));
        }

        let keymap_string = libc::mmap(
            null_mut(),
            size as _,
            libc::PROT_READ,
            libc::MAP_PRIVATE,
            fd,
            0,
        );
        // defer posix.munmap(keymap_string);
        let keymap = (xkbcommon_lib.xkb_keymap_new_from_string)(
            context,
            keymap_string as _,
            xkb_keymap_format::XKB_KEYMAP_FORMAT_TEXT_V1,
            xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS,
        );
        if keymap.is_null() {
            libc::munmap(keymap_string, size as _);
            return Err(anyhow!("could not create keymap from string"));
        }

        let state = (xkbcommon_lib.xkb_state_new)(keymap);
        if state.is_null() {
            libc::munmap(keymap_string, size as _);
            return Err(anyhow!("could not create state"));
        }

        libc::munmap(keymap_string, size as _);

        Ok(Self {
            context,
            keymap,
            state,
            mod_indices: KeyboardModIndices {
                ctrl: (xkbcommon_lib.xkb_keymap_mod_get_index)(
                    keymap,
                    XKB_MOD_NAME_CTRL.as_ptr() as _,
                ),
            },
            mods: KeyboardMods { ctrl: false },
            xkbcommon: xkbcommon_lib,
        })
    }

    pub unsafe fn update_mods(
        &mut self,
        depressed_mods: xkb_mod_mask_t,
        latched_mods: xkb_mod_mask_t,
        locked_mods: xkb_mod_mask_t,
        depressed_layout: xkb_layout_index_t,
        latched_layout: xkb_layout_index_t,
        locked_layout: xkb_layout_index_t,
    ) {
        let mask = (self.xkbcommon.xkb_state_update_mask)(
            self.state,
            depressed_mods,
            latched_mods,
            locked_mods,
            depressed_layout,
            latched_layout,
            locked_layout,
        );
        if (mask & xkb_state_component::XKB_STATE_MODS_EFFECTIVE as c_int) != 0 {
            self.mods.ctrl = (self.xkbcommon.xkb_state_mod_index_is_active)(
                self.state,
                self.mod_indices.ctrl,
                xkb_state_component::XKB_STATE_MODS_EFFECTIVE,
            ) == 1;
        }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            (self.xkbcommon.xkb_state_unref)(self.state);
            (self.xkbcommon.xkb_keymap_unref)(self.keymap);
            (self.xkbcommon.xkb_context_unref)(self.context);
        }
    }
}
