use std::{ops::Deref, ptr::null};

use anyhow::anyhow;

use crate::{egl, gfx};

pub mod sys {
    #[allow(non_camel_case_types)]
    #[allow(clippy::all)]
    mod generated {
        include!(concat!(env!("OUT_DIR"), "/gl_bindings.rs"));
    }

    pub use generated::*;
}

struct TextureFormatDescriptor {
    internalformat: sys::types::GLint,
    format: sys::types::GLenum,
    ty: sys::types::GLenum,
}

fn describe_texture_format(format: gfx::TextureFormat) -> TextureFormatDescriptor {
    use gfx::TextureFormat::*;
    match format {
        // https://gitlab.freedesktop.org/wlroots/wlroots/-/blob/3fdbfb0be82224d472ad6de3a91813064f4cd4b2/render/gles2/pixel_format.c
        Bgra8Unorm => TextureFormatDescriptor {
            internalformat: sys::BGRA_EXT as _,
            format: sys::BGRA_EXT,
            ty: sys::UNSIGNED_BYTE,
        },
        Rgba8Unorm => TextureFormatDescriptor {
            internalformat: sys::RGBA as _,
            format: sys::RGBA,
            ty: sys::UNSIGNED_BYTE,
        },
    }
}

pub struct Lib {
    gl: sys::Gles2,
}

impl Deref for Lib {
    type Target = sys::Gles2;

    fn deref(&self) -> &Self::Target {
        &self.gl
    }
}

impl Lib {
    pub unsafe fn load(egl_lib: &'static egl::Lib) -> Self {
        let mut procname: [u8; 255] = [0u8; 255];
        let gl = sys::Gles2::load_with(|symbol| {
            assert!(symbol.len() < procname.len());
            std::ptr::copy_nonoverlapping(symbol.as_ptr(), procname.as_mut_ptr(), symbol.len());
            procname[symbol.len()] = b'\0';
            egl_lib.GetProcAddress(procname.as_ptr() as _) as _
        });

        Self { gl }
    }
}

pub struct Texture2D {
    gl_lib: &'static Lib,
    pub handle: sys::types::GLuint,
}

impl Texture2D {
    pub unsafe fn new(
        gl_lib: &'static Lib,
        width: u32,
        height: u32,
        format: gfx::TextureFormat,
        pixels: Option<&[u8]>,
    ) -> Self {
        let mut texture = 0;
        gl_lib.GenTextures(1, &mut texture);
        gl_lib.BindTexture(sys::TEXTURE_2D, texture);

        // NOTE: to deal with min and mag filters, etc. - you might want to consider
        // introducing SamplerDescriptor and TextureViewDescriptor
        gl_lib.TexParameteri(sys::TEXTURE_2D, sys::TEXTURE_MIN_FILTER, sys::NEAREST as _);
        gl_lib.TexParameteri(sys::TEXTURE_2D, sys::TEXTURE_MAG_FILTER, sys::NEAREST as _);

        let format_desc = describe_texture_format(format);
        gl_lib.TexImage2D(
            sys::TEXTURE_2D,
            0,
            format_desc.internalformat,
            width as _,
            height as _,
            0,
            format_desc.format,
            format_desc.ty,
            pixels.map(|pixels| pixels.as_ptr()).unwrap_or(null()) as _,
        );

        Self {
            gl_lib,
            handle: texture,
        }
    }
}

impl Drop for Texture2D {
    fn drop(&mut self) {
        unsafe {
            self.gl_lib.DeleteTextures(1, &self.handle);
        }
    }
}

pub struct Shader {
    gl_lib: &'static Lib,
    pub handle: sys::types::GLuint,
}

impl Shader {
    pub unsafe fn new(
        gl_lib: &'static Lib,
        src: &str,
        ty: sys::types::GLenum,
    ) -> anyhow::Result<Self> {
        let shader = gl_lib.CreateShader(ty);
        gl_lib.ShaderSource(shader, 1, &(src.as_ptr() as _), &(src.len() as _));
        gl_lib.CompileShader(shader);

        let mut shader_compiled = 0;
        gl_lib.GetShaderiv(shader, sys::COMPILE_STATUS, &mut shader_compiled);
        if shader_compiled == sys::FALSE as _ {
            let mut len = 0;
            gl_lib.GetShaderiv(shader, sys::INFO_LOG_LENGTH, &mut len);

            let mut msg = String::with_capacity(len as usize);
            msg.extend(std::iter::repeat('\0').take(len as usize));
            gl_lib.GetShaderInfoLog(shader, len, &mut len, msg.as_mut_ptr() as _);
            msg.truncate(len as usize);

            return Err(anyhow!("{msg}"));
        }

        Ok(Self {
            gl_lib,
            handle: shader,
        })
    }
}

impl Drop for Shader {
    fn drop(&mut self) {
        unsafe {
            self.gl_lib.DeleteShader(self.handle);
        }
    }
}

pub struct Program {
    gl_lib: &'static Lib,
    pub handle: sys::types::GLuint,
}

impl Program {
    pub unsafe fn new(
        gl_lib: &'static Lib,
        vert_src: &str,
        frag_src: &str,
    ) -> anyhow::Result<Self> {
        let vert_shader = Shader::new(gl_lib, vert_src, sys::VERTEX_SHADER)?;
        let frag_shader = Shader::new(gl_lib, frag_src, sys::FRAGMENT_SHADER)?;

        let program = gl_lib.CreateProgram();

        gl_lib.AttachShader(program, vert_shader.handle);
        gl_lib.AttachShader(program, frag_shader.handle);
        gl_lib.LinkProgram(program);
        gl_lib.DetachShader(program, vert_shader.handle);
        gl_lib.DetachShader(program, frag_shader.handle);

        let mut program_linked = 0;
        gl_lib.GetProgramiv(program, sys::LINK_STATUS, &mut program_linked);
        if program_linked == sys::FALSE as _ {
            let mut len = 0;
            gl_lib.GetProgramiv(program, sys::INFO_LOG_LENGTH, &mut len);

            let mut msg = String::with_capacity(len as usize);
            msg.extend(std::iter::repeat('\0').take(len as usize));
            gl_lib.GetProgramInfoLog(program, len, &mut len, msg.as_mut_ptr() as _);
            msg.truncate(len as usize);

            return Err(anyhow!("{msg}"));
        }

        Ok(Self {
            gl_lib,
            handle: program,
        })
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        unsafe {
            self.gl_lib.DeleteProgram(self.handle);
        }
    }
}

pub struct Buffer {
    gl_lib: &'static Lib,
    pub handle: sys::types::GLuint,
}

impl Buffer {
    pub unsafe fn new(gl_lib: &'static Lib) -> Self {
        let mut handle = 0;
        gl_lib.GenBuffers(1, &mut handle);
        Self { gl_lib, handle }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            self.gl_lib.DeleteBuffers(1, &self.handle);
        }
    }
}
