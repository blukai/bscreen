use std::{
    ffi::{c_char, c_void},
    mem::zeroed,
    ops::Deref,
    ptr::{null, null_mut},
};

use anyhow::{anyhow, Context};

use crate::{dynlib::DynLib, gl::GlTexture2D};

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
pub enum EglError {
    Raw(sys::types::EGLenum),
}

impl std::fmt::Display for EglError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Raw(raw) => write!(f, "egl error 0x{:x}", raw),
        }
    }
}

impl std::error::Error for EglError {}

pub struct Egl {
    _lib: DynLib,
    egl: sys::Egl,
}

impl Deref for Egl {
    type Target = sys::Egl;

    fn deref(&self) -> &Self::Target {
        &self.egl
    }
}

impl Egl {
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

    pub fn unwrap_err(&self) -> EglError {
        match unsafe { self.GetError() } as sys::types::EGLenum {
            sys::SUCCESS => unreachable!(),
            raw => EglError::Raw(raw),
        }
    }
}

pub struct EglContext {
    egl: &'static Egl,
    pub display: sys::types::EGLDisplay,
    pub config: sys::types::EGLConfig,
    pub context: sys::types::EGLContext,
}

impl EglContext {
    pub unsafe fn make_current_surfaceless(&self) -> anyhow::Result<()> {
        if self
            .egl
            .MakeCurrent(self.display, sys::NO_SURFACE, sys::NO_SURFACE, self.context)
            == sys::FALSE
        {
            Err(self.egl.unwrap_err()).context("could not make current")
        } else {
            Ok(())
        }
    }

    pub unsafe fn make_current(&self, surface: sys::types::EGLSurface) -> anyhow::Result<()> {
        if self
            .egl
            .MakeCurrent(self.display, surface, surface, self.context)
            == sys::FALSE
        {
            Err(self.egl.unwrap_err()).context("could not make current")
        } else {
            Ok(())
        }
    }

    pub unsafe fn swap_buffers(&self, surface: sys::types::EGLSurface) -> anyhow::Result<()> {
        if self.egl.SwapBuffers(self.display, surface) == sys::FALSE {
            Err(self.egl.unwrap_err()).context("could not swap buffers")
        } else {
            Ok(())
        }
    }

    pub unsafe fn create(
        egl: &'static Egl,
        display_id: sys::EGLNativeDisplayType,
    ) -> anyhow::Result<Self> {
        if egl.BindAPI(sys::OPENGL_ES_API) == sys::FALSE {
            return Err(egl.unwrap_err()).context("could not bind api");
        }

        let display = egl.GetDisplay(display_id);
        if display == sys::NO_DISPLAY {
            return Err(egl.unwrap_err()).context("could not get display");
        }

        let (mut major, mut minor) = (0, 0);
        if egl.Initialize(display, &mut major, &mut minor) == sys::FALSE {
            return Err(egl.unwrap_err()).context("could not initialize");
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
        if egl.GetConfigs(display, null_mut(), 0, &mut num_configs) == sys::FALSE {
            return Err(egl.unwrap_err()).context("could not get number of available configs");
        }
        let mut configs = vec![zeroed(); num_configs as usize];
        if egl.ChooseConfig(
            display,
            config_attrs.as_ptr() as _,
            configs.as_mut_ptr(),
            num_configs,
            &mut num_configs,
        ) == sys::FALSE
        {
            return Err(egl.unwrap_err()).context("could not choose config");
        }
        configs.set_len(num_configs as usize);
        if configs.is_empty() {
            return Err(anyhow!("could not choose config (/ no compatible ones)"));
        }
        let config = *configs.first().unwrap();

        let context_attrs = &[sys::CONTEXT_MAJOR_VERSION, 3, sys::NONE];
        let context = egl.CreateContext(
            display,
            config,
            sys::NO_CONTEXT,
            context_attrs.as_ptr() as _,
        );
        if context == sys::NO_CONTEXT {
            return Err(egl.unwrap_err()).context("could not create context");
        }

        let egl_context = EglContext {
            egl,
            display,
            config,
            context,
        };
        egl_context.make_current_surfaceless()?;
        Ok(egl_context)
    }
}

impl Drop for EglContext {
    fn drop(&mut self) {
        unsafe {
            self.egl.DestroyContext(self.display, self.context);
        }
    }
}

pub struct EglImageKhr {
    egl: &'static Egl,
    egl_context: &'static EglContext,
    pub handle: sys::types::EGLImageKHR,
}

impl EglImageKhr {
    pub unsafe fn new(
        egl: &'static Egl,
        egl_context: &'static EglContext,
        gl_texture: &GlTexture2D,
    ) -> anyhow::Result<Self> {
        let image = unsafe {
            egl.CreateImageKHR(
                egl_context.display,
                egl_context.context,
                sys::GL_TEXTURE_2D,
                gl_texture.handle as _,
                null(),
            )
        };
        if image == sys::NO_IMAGE_KHR {
            return Err(egl.unwrap_err()).context("could not create egl khr image");
        }
        Ok(Self {
            egl,
            egl_context,
            handle: image,
        })
    }
}

impl Drop for EglImageKhr {
    fn drop(&mut self) {
        unsafe {
            self.egl
                .DestroyImageKHR(self.egl_context.display, self.handle);
        }
    }
}

pub struct EglWindowSurface {
    egl: &'static Egl,
    egl_context: &'static EglContext,
    pub handle: sys::types::EGLSurface,
}

impl EglWindowSurface {
    pub unsafe fn new(
        egl: &'static Egl,
        egl_context: &'static EglContext,
        window_id: sys::EGLNativeWindowType,
    ) -> anyhow::Result<Self> {
        let window_surface =
            egl.CreateWindowSurface(egl_context.display, egl_context.config, window_id, null());
        if window_surface.is_null() {
            return Err(egl.unwrap_err()).context("could not create window surface");
        }
        Ok(Self {
            egl,
            egl_context,
            handle: window_surface,
        })
    }
}

impl Drop for EglWindowSurface {
    fn drop(&mut self) {
        unsafe {
            self.egl
                .DestroySurface(self.egl_context.display, self.handle);
        }
    }
}
