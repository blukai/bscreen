use std::{ops::Deref, ptr::null};

use anyhow::anyhow;

use crate::{egl::Egl, gfx};

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

pub struct Gl {
    gl: sys::Gles2,
}

impl Deref for Gl {
    type Target = sys::Gles2;

    fn deref(&self) -> &Self::Target {
        &self.gl
    }
}

impl Gl {
    pub unsafe fn load(egl: &'static Egl) -> Self {
        let mut procname: [u8; 255] = [0u8; 255];
        let gl = sys::Gles2::load_with(|symbol| {
            assert!(symbol.len() < procname.len());
            std::ptr::copy_nonoverlapping(symbol.as_ptr(), procname.as_mut_ptr(), symbol.len());
            procname[symbol.len()] = b'\0';
            egl.GetProcAddress(procname.as_ptr() as _) as _
        });

        Self { gl }
    }
}

pub struct GlTexture2D {
    gl: &'static Gl,
    pub handle: sys::types::GLuint,
}

impl GlTexture2D {
    pub unsafe fn new(
        gl: &'static Gl,
        width: u32,
        height: u32,
        format: gfx::TextureFormat,
        pixels: Option<&[u8]>,
    ) -> Self {
        let mut texture = 0;
        gl.GenTextures(1, &mut texture);
        gl.BindTexture(sys::TEXTURE_2D, texture);

        // NOTE: to deal with min and mag filters, etc. - you might want to consider
        // introducing SamplerDescriptor and TextureViewDescriptor
        gl.TexParameteri(sys::TEXTURE_2D, sys::TEXTURE_MIN_FILTER, sys::NEAREST as _);
        gl.TexParameteri(sys::TEXTURE_2D, sys::TEXTURE_MAG_FILTER, sys::NEAREST as _);

        let format_desc = describe_texture_format(format);
        gl.TexImage2D(
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
            gl,
            handle: texture,
        }
    }
}

impl Drop for GlTexture2D {
    fn drop(&mut self) {
        unsafe {
            self.gl.DeleteTextures(1, &self.handle);
        }
    }
}

pub struct GlShader {
    gl: &'static Gl,
    pub handle: sys::types::GLuint,
}

impl GlShader {
    pub unsafe fn new(gl: &'static Gl, src: &str, ty: sys::types::GLenum) -> anyhow::Result<Self> {
        let shader = gl.CreateShader(ty);
        gl.ShaderSource(shader, 1, &(src.as_ptr() as _), &(src.len() as _));
        gl.CompileShader(shader);

        let mut shader_compiled = 0;
        gl.GetShaderiv(shader, sys::COMPILE_STATUS, &mut shader_compiled);
        if shader_compiled == sys::FALSE as _ {
            let mut len = 0;
            gl.GetShaderiv(shader, sys::INFO_LOG_LENGTH, &mut len);

            let mut msg = String::with_capacity(len as usize);
            msg.extend(std::iter::repeat('\0').take(len as usize));
            gl.GetShaderInfoLog(shader, len, &mut len, msg.as_mut_ptr() as _);
            msg.truncate(len as usize);

            return Err(anyhow!("{msg}"));
        }

        Ok(Self { gl, handle: shader })
    }
}

impl Drop for GlShader {
    fn drop(&mut self) {
        unsafe {
            self.gl.DeleteShader(self.handle);
        }
    }
}

pub struct GlProgram {
    gl: &'static Gl,
    pub handle: sys::types::GLuint,
}

impl GlProgram {
    pub unsafe fn new(gl: &'static Gl, vert_src: &str, frag_src: &str) -> anyhow::Result<Self> {
        let vert_shader = GlShader::new(gl, vert_src, sys::VERTEX_SHADER)?;
        let frag_shader = GlShader::new(gl, frag_src, sys::FRAGMENT_SHADER)?;

        let program = gl.CreateProgram();

        gl.AttachShader(program, vert_shader.handle);
        gl.AttachShader(program, frag_shader.handle);
        gl.LinkProgram(program);
        gl.DetachShader(program, vert_shader.handle);
        gl.DetachShader(program, frag_shader.handle);

        let mut program_linked = 0;
        gl.GetProgramiv(program, sys::LINK_STATUS, &mut program_linked);
        if program_linked == sys::FALSE as _ {
            let mut len = 0;
            gl.GetProgramiv(program, sys::INFO_LOG_LENGTH, &mut len);

            let mut msg = String::with_capacity(len as usize);
            msg.extend(std::iter::repeat('\0').take(len as usize));
            gl.GetProgramInfoLog(program, len, &mut len, msg.as_mut_ptr() as _);
            msg.truncate(len as usize);

            return Err(anyhow!("{msg}"));
        }

        Ok(Self {
            gl,
            handle: program,
        })
    }
}

impl Drop for GlProgram {
    fn drop(&mut self) {
        unsafe {
            self.gl.DeleteProgram(self.handle);
        }
    }
}

pub struct GlBuffer {
    gl: &'static Gl,
    pub handle: sys::types::GLuint,
}

impl GlBuffer {
    pub unsafe fn new(gl: &'static Gl) -> Self {
        let mut handle = 0;
        gl.GenBuffers(1, &mut handle);
        Self { gl, handle }
    }
}

impl Drop for GlBuffer {
    fn drop(&mut self) {
        unsafe {
            self.gl.DeleteBuffers(1, &self.handle);
        }
    }
}
