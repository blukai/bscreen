use glam::{UVec2, Vec2};

// NOTE: TextureFormat is modeled after webgpu, see:
// - https://github.com/webgpu-native/webgpu-headers/blob/449359147fae26c07efe4fece25013df396287db/webgpu.h
// - https://www.w3.org/TR/webgpu/#texture-formats
pub enum TextureFormat {
    // Bgra8Unorm is compatible with VK_FORMAT_B8G8R8A8_UNORM, it is also
    // compativle with DRM_FORMAT_XRGB8888 (x is not alpha, x means that the byte is
    // wasted).
    Bgra8Unorm,
    Rgba8Unorm,
}

// NOTE: my definition of logical size matches wayland. but my defintion of
// physical size does not, in wayland's terminology what i call physical size
// most likely is buffer size.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// rounds away from zero
    pub fn to_physical(&self, scale_factor: f64) -> Self {
        Self {
            width: ((self.width as f64) * scale_factor).round() as u32,
            height: ((self.height as f64) * scale_factor).round() as u32,
        }
    }

    /// rounds away from zero
    pub fn to_logical(&self, scale_factor: f64) -> Self {
        Self {
            width: ((self.width as f64) / scale_factor).round() as u32,
            height: ((self.height as f64) / scale_factor).round() as u32,
        }
    }

    pub fn as_uvec2(&self) -> UVec2 {
        UVec2::new(self.width, self.height)
    }
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const WHITE: Self = Self::new(u8::MAX, u8::MAX, u8::MAX, u8::MAX);
    pub const BLACK: Self = Self::new(u8::MIN, u8::MIN, u8::MIN, u8::MAX);
    pub const RED: Self = Self::new(u8::MAX, u8::MIN, u8::MIN, u8::MAX);
    pub const GREEN: Self = Self::new(u8::MIN, u8::MAX, u8::MIN, u8::MAX);
    pub const BLUE: Self = Self::new(u8::MIN, u8::MIN, u8::MAX, u8::MAX);

    #[inline]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

#[derive(PartialEq)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub fn new(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }

    pub fn top_left(&self) -> Vec2 {
        self.min
    }

    pub fn top_right(&self) -> Vec2 {
        Vec2::new(self.max.x, self.min.y)
    }

    pub fn bottom_left(&self) -> Vec2 {
        Vec2::new(self.min.x, self.max.y)
    }

    pub fn bottom_right(&self) -> Vec2 {
        self.max
    }
}

pub enum RectFill {
    TextureHandle(u32),
    Color(Rgba8),
}

#[repr(C)]
#[derive(Debug, Clone)]
pub struct Vertex {
    /// screen pixel coordinates.
    /// 0, 0 is the top left corner of the screen.
    pub position: Vec2,
    /// normalized texture coordinates.
    /// 0, 0 is the top left corner of the texture.
    /// 1, 1 is the bottom right corner of the texture.
    pub tex_coord: Vec2,
    pub color: Rgba8,
}

#[derive(Debug)]
pub struct DrawCommand {
    pub start_index: u32,
    pub end_index: u32,
    /// a non-owning handle, (de)init is someone else's responsibility.
    pub texture_handle: Option<u32>,
}

#[derive(Debug, Default)]
pub struct DrawBuffer {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub pending_indices: usize,
    pub draw_commands: Vec<DrawCommand>,
}

impl DrawBuffer {
    pub fn clear(&mut self) {
        assert!(self.pending_indices == 0);
        self.vertices.clear();
        self.indices.clear();
        self.draw_commands.clear();
    }

    fn push_vertex(&mut self, vertex: Vertex) {
        self.vertices.push(vertex);
    }

    fn push_triangle(&mut self, zero: u32, ichi: u32, ni: u32) {
        self.indices.push(zero);
        self.indices.push(ichi);
        self.indices.push(ni);
        self.pending_indices += 3;
    }

    fn commit(&mut self, texture_handle: Option<u32>) {
        if self.pending_indices == 0 {
            return;
        }
        self.draw_commands.push(DrawCommand {
            start_index: (self.indices.len() - self.pending_indices) as u32,
            end_index: self.indices.len() as u32,
            texture_handle,
        });
        self.pending_indices = 0;
    }

    pub fn push_rect_filled(&mut self, rect: Rect, rect_fill: RectFill) {
        let idx = self.vertices.len() as u32;

        let (color, texture_handle) = match rect_fill {
            RectFill::Color(color) => (color, None),
            RectFill::TextureHandle(texture_handle) => (Rgba8::WHITE, Some(texture_handle)),
        };

        // top left
        self.push_vertex(Vertex {
            position: rect.top_left(),
            tex_coord: Vec2::new(0.0, 0.0),
            color,
        });
        // top right
        self.push_vertex(Vertex {
            position: rect.top_right(),
            tex_coord: Vec2::new(1.0, 0.0),
            color,
        });
        // bottom right
        self.push_vertex(Vertex {
            position: rect.bottom_right(),
            tex_coord: Vec2::new(1.0, 1.0),
            color,
        });
        // bottom left
        self.push_vertex(Vertex {
            position: rect.bottom_left(),
            tex_coord: Vec2::new(0.0, 1.0),
            color,
        });

        // top left -> top right -> bottom right
        self.push_triangle(idx + 0, idx + 1, idx + 2);
        // bottom right -> bottom left -> top left
        self.push_triangle(idx + 2, idx + 3, idx + 0);

        self.commit(texture_handle);
    }
}
