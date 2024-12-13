use std::mem::offset_of;

use crate::{
    gfx::{DrawBuffer, Size, TextureFormat, Vertex},
    gl,
};

const VERT_SRC: &str = include_str!("vert.glsl");
const FRAG_SRC: &str = include_str!("frag.glsl");

pub struct Renderer {
    a_position_location: gl::sys::types::GLint,
    a_tex_coord_location: gl::sys::types::GLint,
    a_color_location: gl::sys::types::GLint,
    u_view_size_location: gl::sys::types::GLint,

    vbo: gl::Buffer,
    ebo: gl::Buffer,

    program: gl::Program,

    default_white_tex: gl::Texture2D,
    gl_lib: &'static gl::Lib,
}

impl Renderer {
    pub unsafe fn new(gl_lib: &'static gl::Lib) -> anyhow::Result<Self> {
        let program = gl::Program::new(gl_lib, VERT_SRC, FRAG_SRC)?;
        Ok(Self {
            a_position_location: gl_lib
                .GetAttribLocation(program.handle, "a_position\0".as_ptr() as _),
            a_tex_coord_location: gl_lib
                .GetAttribLocation(program.handle, "a_tex_coord\0".as_ptr() as _),
            a_color_location: gl_lib.GetAttribLocation(program.handle, "a_color\0".as_ptr() as _),
            u_view_size_location: gl_lib
                .GetUniformLocation(program.handle, "u_view_size\0".as_ptr() as _),

            vbo: gl::Buffer::new(gl_lib),
            ebo: gl::Buffer::new(gl_lib),

            program,

            default_white_tex: gl::Texture2D::new(
                gl_lib,
                1,
                1,
                TextureFormat::Rgba8Unorm,
                Some(&[255, 255, 255, 255]),
            ),
            gl_lib,
        })
    }

    pub unsafe fn setup_buffers(&self) {
        // vertex
        self.gl_lib
            .BindBuffer(gl::sys::ARRAY_BUFFER, self.vbo.handle);
        self.gl_lib
            .EnableVertexAttribArray(self.a_position_location as _);
        self.gl_lib.VertexAttribPointer(
            self.a_position_location as _,
            2,
            gl::sys::FLOAT,
            gl::sys::FALSE,
            size_of::<Vertex>() as _,
            offset_of!(Vertex, position) as *const usize as _,
        );
        self.gl_lib
            .EnableVertexAttribArray(self.a_tex_coord_location as _);
        self.gl_lib.VertexAttribPointer(
            self.a_tex_coord_location as _,
            2,
            gl::sys::FLOAT,
            gl::sys::FALSE,
            size_of::<Vertex>() as _,
            offset_of!(Vertex, tex_coord) as *const usize as _,
        );
        self.gl_lib
            .EnableVertexAttribArray(self.a_color_location as _);
        self.gl_lib.VertexAttribPointer(
            self.a_color_location as _,
            4,
            gl::sys::UNSIGNED_BYTE,
            gl::sys::FALSE,
            size_of::<Vertex>() as _,
            offset_of!(Vertex, color) as *const usize as _,
        );

        // index
        self.gl_lib
            .BindBuffer(gl::sys::ELEMENT_ARRAY_BUFFER, self.ebo.handle);
    }

    pub unsafe fn draw(&self, logical_size: Size, fractional_scale: f64, draw_buffer: &DrawBuffer) {
        let physical_size = logical_size.to_physical(fractional_scale);

        self.gl_lib.UseProgram(self.program.handle);

        self.gl_lib.Enable(gl::sys::BLEND);
        self.gl_lib
            .BlendFunc(gl::sys::SRC_ALPHA, gl::sys::ONE_MINUS_SRC_ALPHA);

        self.gl_lib
            .Viewport(0, 0, physical_size.width as _, physical_size.height as _);

        self.gl_lib.Uniform2f(
            self.u_view_size_location,
            logical_size.width as _,
            logical_size.height as _,
        );

        self.setup_buffers();

        self.gl_lib.BufferData(
            gl::sys::ARRAY_BUFFER,
            (draw_buffer.vertices.len() * size_of::<Vertex>()) as _,
            draw_buffer.vertices.as_ptr() as _,
            gl::sys::STREAM_DRAW,
        );
        self.gl_lib.BufferData(
            gl::sys::ELEMENT_ARRAY_BUFFER,
            (draw_buffer.indices.len() * size_of::<u32>()) as _,
            draw_buffer.indices.as_ptr() as _,
            gl::sys::STREAM_DRAW,
        );

        for draw_command in draw_buffer.draw_commands.iter() {
            self.gl_lib.ActiveTexture(gl::sys::TEXTURE0);
            self.gl_lib.BindTexture(
                gl::sys::TEXTURE_2D,
                draw_command
                    .texture_handle
                    .unwrap_or(self.default_white_tex.handle),
            );

            self.gl_lib.DrawElements(
                gl::sys::TRIANGLES,
                (draw_command.end_index - draw_command.start_index) as _,
                gl::sys::UNSIGNED_INT,
                (draw_command.start_index * size_of::<u32>() as u32) as *const u32 as _,
            );
        }
    }
}
