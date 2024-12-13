use std::mem::offset_of;

use crate::{
    gfx::{DrawBuffer, Size, TextureFormat, Vertex},
    gl::{self, Gl, GlBuffer, GlProgram, GlTexture2D},
};

const VERT_SRC: &str = include_str!("vert.glsl");
const FRAG_SRC: &str = include_str!("frag.glsl");

pub struct Renderer {
    a_position_location: gl::sys::types::GLint,
    a_tex_coord_location: gl::sys::types::GLint,
    a_color_location: gl::sys::types::GLint,
    u_view_size_location: gl::sys::types::GLint,

    vbo: GlBuffer,
    ebo: GlBuffer,

    program: GlProgram,

    default_white_tex: GlTexture2D,
    gl: &'static Gl,
}

impl Renderer {
    pub unsafe fn new(gl: &'static Gl) -> anyhow::Result<Self> {
        let program = GlProgram::new(gl, VERT_SRC, FRAG_SRC)?;
        Ok(Self {
            a_position_location: gl.GetAttribLocation(program.handle, "a_position\0".as_ptr() as _),
            a_tex_coord_location: gl
                .GetAttribLocation(program.handle, "a_tex_coord\0".as_ptr() as _),
            a_color_location: gl.GetAttribLocation(program.handle, "a_color\0".as_ptr() as _),
            u_view_size_location: gl
                .GetUniformLocation(program.handle, "u_view_size\0".as_ptr() as _),

            vbo: GlBuffer::new(gl),
            ebo: GlBuffer::new(gl),

            program,

            default_white_tex: GlTexture2D::new(
                gl,
                1,
                1,
                TextureFormat::Rgba8Unorm,
                Some(&[255, 255, 255, 255]),
            ),
            gl,
        })
    }

    pub unsafe fn setup_buffers(&self) {
        // vertex
        self.gl.BindBuffer(gl::sys::ARRAY_BUFFER, self.vbo.handle);
        self.gl
            .EnableVertexAttribArray(self.a_position_location as _);
        self.gl.VertexAttribPointer(
            self.a_position_location as _,
            2,
            gl::sys::FLOAT,
            gl::sys::FALSE,
            size_of::<Vertex>() as _,
            offset_of!(Vertex, position) as *const usize as _,
        );
        self.gl
            .EnableVertexAttribArray(self.a_tex_coord_location as _);
        self.gl.VertexAttribPointer(
            self.a_tex_coord_location as _,
            2,
            gl::sys::FLOAT,
            gl::sys::FALSE,
            size_of::<Vertex>() as _,
            offset_of!(Vertex, tex_coord) as *const usize as _,
        );
        self.gl.EnableVertexAttribArray(self.a_color_location as _);
        self.gl.VertexAttribPointer(
            self.a_color_location as _,
            4,
            gl::sys::UNSIGNED_BYTE,
            gl::sys::FALSE,
            size_of::<Vertex>() as _,
            offset_of!(Vertex, color) as *const usize as _,
        );

        // index
        self.gl
            .BindBuffer(gl::sys::ELEMENT_ARRAY_BUFFER, self.ebo.handle);
    }

    pub unsafe fn draw(&self, logical_size: Size, fractional_scale: f64, draw_buffer: &DrawBuffer) {
        let physical_size = logical_size.to_physical(fractional_scale);

        self.gl.UseProgram(self.program.handle);

        self.gl.Enable(gl::sys::BLEND);
        self.gl
            .BlendFunc(gl::sys::SRC_ALPHA, gl::sys::ONE_MINUS_SRC_ALPHA);

        self.gl
            .Viewport(0, 0, physical_size.width as _, physical_size.height as _);

        self.gl.Uniform2f(
            self.u_view_size_location,
            logical_size.width as _,
            logical_size.height as _,
        );

        self.setup_buffers();

        self.gl.BufferData(
            gl::sys::ARRAY_BUFFER,
            (draw_buffer.vertices.len() * size_of::<Vertex>()) as _,
            draw_buffer.vertices.as_ptr() as _,
            gl::sys::STREAM_DRAW,
        );
        self.gl.BufferData(
            gl::sys::ELEMENT_ARRAY_BUFFER,
            (draw_buffer.indices.len() * size_of::<u32>()) as _,
            draw_buffer.indices.as_ptr() as _,
            gl::sys::STREAM_DRAW,
        );

        for draw_command in draw_buffer.draw_commands.iter() {
            self.gl.ActiveTexture(gl::sys::TEXTURE0);
            self.gl.BindTexture(
                gl::sys::TEXTURE_2D,
                draw_command
                    .texture_handle
                    .unwrap_or(self.default_white_tex.handle),
            );

            self.gl.DrawElements(
                gl::sys::TRIANGLES,
                (draw_command.end_index - draw_command.start_index) as _,
                gl::sys::UNSIGNED_INT,
                (draw_command.start_index * size_of::<u32>() as u32) as *const u32 as _,
            );
        }
    }
}
