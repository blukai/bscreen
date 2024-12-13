use std::ffi::{c_char, c_void};
use std::mem::zeroed;
use std::ops::Deref;
use std::ptr::{null, null_mut};

use anyhow::{anyhow, Context as _};

use crate::dynlib::DynLib;
use crate::gl::Texture2D;

pub mod sys {
    #[allow(non_camel_case_types)]
    #[allow(clippy::all)]
    mod generated {
        use std::ffi::{c_long, c_void};

        // NOTE: gl_generator specifies that platform-specific aliases are unknown and must be
        // defined by the user (we are the user).
        //
        // stolen from https://github.com/tomaka/glutin/blob/1f3b8360cb/src/api/egl/ffi.rs
        pub type khronos_utime_nanoseconds_t = khronos_uint64_t;
        pub type khronos_uint64_t = u64;
        pub type khronos_ssize_t = c_long;
        pub type EGLint = i32;
        pub type EGLNativeDisplayType = *const c_void;
        pub type EGLNativePixmapType = *const c_void;
        pub type EGLNativeWindowType = *const c_void;
        pub type NativeDisplayType = EGLNativeDisplayType;
        pub type NativePixmapType = EGLNativePixmapType;
        pub type NativeWindowType = EGLNativeWindowType;

        include!(concat!(env!("OUT_DIR"), "/egl_bindings.rs"));
    }

    pub use generated::*;
}

#[derive(Debug)]
pub enum Error {
    Raw(sys::types::EGLenum),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Raw(raw) => write!(f, "egl error 0x{:x}", raw),
        }
    }
}

impl std::error::Error for Error {}

pub struct Lib {
    _lib: DynLib,
    egl: sys::Egl,
}

impl Deref for Lib {
    type Target = sys::Egl;

    fn deref(&self) -> &Self::Target {
        &self.egl
    }
}

impl Lib {
    pub unsafe fn load() -> Result<Self, String> {
        let lib = DynLib::open(b"libEGL.so\0").or_else(|_| DynLib::open(b"libEGL.so.1\0"))?;

        #[allow(non_snake_case)]
        let eglGetProcAddress = lib
            .lookup::<unsafe extern "C" fn(*const c_char) -> *const c_void>(
                b"eglGetProcAddress\0",
            )?;
        let mut procname: [u8; 255] = [0u8; 255];
        let egl = sys::Egl::load_with(|symbol| {
            assert!(symbol.len() < procname.len());
            std::ptr::copy_nonoverlapping(symbol.as_ptr(), procname.as_mut_ptr(), symbol.len());
            procname[symbol.len()] = b'\0';
            eglGetProcAddress(procname.as_ptr() as _)
        });

        Ok(Self { _lib: lib, egl })
    }

    pub fn unwrap_err(&self) -> Error {
        match unsafe { self.GetError() } as sys::types::EGLenum {
            sys::SUCCESS => unreachable!(),
            raw => Error::Raw(raw),
        }
    }
}

pub struct Context {
    egl_lib: &'static Lib,
    pub display: sys::types::EGLDisplay,
    pub config: sys::types::EGLConfig,
    pub context: sys::types::EGLContext,
}

impl Context {
    pub unsafe fn make_current_surfaceless(&self) -> anyhow::Result<()> {
        if self
            .egl_lib
            .MakeCurrent(self.display, sys::NO_SURFACE, sys::NO_SURFACE, self.context)
            == sys::FALSE
        {
            Err(self.egl_lib.unwrap_err()).context("could not make current")
        } else {
            Ok(())
        }
    }

    pub unsafe fn make_current(&self, surface: sys::types::EGLSurface) -> anyhow::Result<()> {
        if self
            .egl_lib
            .MakeCurrent(self.display, surface, surface, self.context)
            == sys::FALSE
        {
            Err(self.egl_lib.unwrap_err()).context("could not make current")
        } else {
            Ok(())
        }
    }

    pub unsafe fn swap_buffers(&self, surface: sys::types::EGLSurface) -> anyhow::Result<()> {
        if self.egl_lib.SwapBuffers(self.display, surface) == sys::FALSE {
            Err(self.egl_lib.unwrap_err()).context("could not swap buffers")
        } else {
            Ok(())
        }
    }

    pub unsafe fn create(
        egl_lib: &'static Lib,
        display_id: sys::EGLNativeDisplayType,
    ) -> anyhow::Result<Self> {
        if egl_lib.BindAPI(sys::OPENGL_ES_API) == sys::FALSE {
            return Err(egl_lib.unwrap_err()).context("could not bind api");
        }

        let display = egl_lib.GetDisplay(display_id);
        if display == sys::NO_DISPLAY {
            return Err(egl_lib.unwrap_err()).context("could not get display");
        }

        let (mut major, mut minor) = (0, 0);
        if egl_lib.Initialize(display, &mut major, &mut minor) == sys::FALSE {
            return Err(egl_lib.unwrap_err()).context("could not initialize");
        }
        log::info!("initialized egl version {major}.{minor}");

        let config_attrs = &[
            sys::RED_SIZE,
            8,
            sys::GREEN_SIZE,
            8,
            sys::BLUE_SIZE,
            8,
            // NOTE: it is important to set EGL_ALPHA_SIZE, it enables transparency
            sys::ALPHA_SIZE,
            8,
            sys::CONFORMANT,
            sys::OPENGL_ES3_BIT,
            sys::RENDERABLE_TYPE,
            sys::OPENGL_ES3_BIT,
            // NOTE: EGL_SAMPLE_BUFFERS + EGL_SAMPLES enables some kind of don't care anti aliasing
            sys::SAMPLE_BUFFERS,
            1,
            sys::SAMPLES,
            4,
            sys::NONE,
        ];

        let mut num_configs = 0;
        if egl_lib.GetConfigs(display, null_mut(), 0, &mut num_configs) == sys::FALSE {
            return Err(egl_lib.unwrap_err()).context("could not get number of available configs");
        }
        let mut configs = vec![zeroed(); num_configs as usize];
        if egl_lib.ChooseConfig(
            display,
            config_attrs.as_ptr() as _,
            configs.as_mut_ptr(),
            num_configs,
            &mut num_configs,
        ) == sys::FALSE
        {
            return Err(egl_lib.unwrap_err()).context("could not choose config");
        }
        configs.set_len(num_configs as usize);
        if configs.is_empty() {
            return Err(anyhow!("could not choose config (/ no compatible ones)"));
        }
        let config = *configs.first().unwrap();

        let context_attrs = &[sys::CONTEXT_MAJOR_VERSION, 3, sys::NONE];
        let context = egl_lib.CreateContext(
            display,
            config,
            sys::NO_CONTEXT,
            context_attrs.as_ptr() as _,
        );
        if context == sys::NO_CONTEXT {
            return Err(egl_lib.unwrap_err()).context("could not create context");
        }

        let egl_context = Context {
            egl_lib,
            display,
            config,
            context,
        };
        egl_context.make_current_surfaceless()?;
        Ok(egl_context)
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            self.egl_lib.DestroyContext(self.display, self.context);
        }
    }
}

pub struct ImageKhr {
    egl_lib: &'static Lib,
    egl_context: &'static Context,
    pub handle: sys::types::EGLImageKHR,
}

impl ImageKhr {
    pub unsafe fn new(
        egl_lib: &'static Lib,
        egl_context: &'static Context,
        gl_texture: &Texture2D,
    ) -> anyhow::Result<Self> {
        let image = unsafe {
            egl_lib.CreateImageKHR(
                egl_context.display,
                egl_context.context,
                sys::GL_TEXTURE_2D,
                gl_texture.handle as _,
                null(),
            )
        };
        if image == sys::NO_IMAGE_KHR {
            return Err(egl_lib.unwrap_err()).context("could not create egl khr image");
        }
        Ok(Self {
            egl_lib,
            egl_context,
            handle: image,
        })
    }
}

impl Drop for ImageKhr {
    fn drop(&mut self) {
        unsafe {
            self.egl_lib
                .DestroyImageKHR(self.egl_context.display, self.handle);
        }
    }
}

pub struct WindowSurface {
    egl_lib: &'static Lib,
    egl_context: &'static Context,
    pub handle: sys::types::EGLSurface,
}

impl WindowSurface {
    pub unsafe fn new(
        egl_lib: &'static Lib,
        egl_context: &'static Context,
        window_id: sys::EGLNativeWindowType,
    ) -> anyhow::Result<Self> {
        let window_surface =
            egl_lib.CreateWindowSurface(egl_context.display, egl_context.config, window_id, null());
        if window_surface.is_null() {
            return Err(egl_lib.unwrap_err()).context("could not create window surface");
        }
        Ok(Self {
            egl_lib,
            egl_context,
            handle: window_surface,
        })
    }
}

impl Drop for WindowSurface {
    fn drop(&mut self) {
        unsafe {
            self.egl_lib
                .DestroySurface(self.egl_context.display, self.handle);
        }
    }
}
