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

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl std::ops::Add<Vec2> for Vec2 {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x.add(rhs.x),
            y: self.y.add(rhs.y),
        }
    }
}

impl std::ops::Sub<Vec2> for Vec2 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x.sub(rhs.x),
            y: self.y.sub(rhs.y),
        }
    }
}

impl std::ops::Mul<Vec2> for Vec2 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: Self) -> Self {
        Self {
            x: self.x.mul(rhs.x),
            y: self.y.mul(rhs.y),
        }
    }
}

impl std::ops::Div<Vec2> for Vec2 {
    type Output = Self;

    #[inline]
    fn div(self, rhs: Self) -> Self {
        Self {
            x: self.x.div(rhs.x),
            y: self.y.div(rhs.y),
        }
    }
}

impl std::ops::Mul<f32> for Vec2 {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f32) -> Self {
        Self {
            x: self.x.mul(rhs),
            y: self.y.mul(rhs),
        }
    }
}

impl std::ops::Div<f32> for Vec2 {
    type Output = Self;

    #[inline]
    fn div(self, rhs: f32) -> Self {
        Self {
            x: self.x.div(rhs),
            y: self.y.div(rhs),
        }
    }
}

impl Vec2 {
    pub const ZERO: Self = Self::splat(0.0);

    #[inline]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    #[inline]
    pub const fn splat(v: f32) -> Self {
        Self { x: v, y: v }
    }

    #[inline]
    pub fn dot(self, rhs: Self) -> f32 {
        (self.x * rhs.x) + (self.y * rhs.y)
    }

    /// computes the length (magnitude) of the vector.
    #[inline]
    pub fn length(self) -> f32 {
        f32::sqrt(self.dot(self))
    }

    /// returns `self` normalized to length 1 if possible, else returns zero.
    /// in particular, if the input is zero, or non-finite, the result of
    /// this operation will be zero.
    #[inline]
    pub fn normalize_or_zero(self) -> Self {
        // reciprocal is also called multiplicative inverse
        let reciprocal_length = 1.0 / self.length();
        if reciprocal_length.is_finite() && reciprocal_length > 0.0 {
            self * reciprocal_length
        } else {
            Self::splat(0.0)
        }
    }

    #[inline]
    pub fn perp(self) -> Self {
        Self {
            x: -self.y,
            y: self.x,
        }
    }
}

// NOTE: my definition of logical size matches wayland. but my defintion of
// physical size does not, in wayland's terminology what i call physical size
// most likely is buffer size.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
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

    #[inline]
    pub fn as_vec2(&self) -> Vec2 {
        Vec2::new(self.width as f32, self.height as f32)
    }
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const WHITE: Self = Self::new(u8::MAX, u8::MAX, u8::MAX, u8::MAX);

    #[inline]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
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

    pub fn set_top_left(&mut self, top_left: Vec2) {
        self.min = top_left;
    }

    pub fn set_top_right(&mut self, top_right: Vec2) {
        self.min = Vec2::new(self.min.x, top_right.y);
        self.max = Vec2::new(top_right.x, self.max.y);
    }

    pub fn set_bottom_right(&mut self, bottom_right: Vec2) {
        self.max = bottom_right;
    }

    pub fn set_bottom_left(&mut self, bottom_left: Vec2) {
        self.min = Vec2::new(bottom_left.x, self.min.y);
        self.max = Vec2::new(self.max.x, bottom_left.y);
    }

    pub fn from_center_size(center: Vec2, size: f32) -> Self {
        let radius = Vec2::splat(size / 2.0);
        Self {
            min: center - radius,
            max: center + radius,
        }
    }

    pub fn contains(&self, p: &Vec2) -> bool {
        let x = p.x >= self.min.x && p.x <= self.max.x;
        let y = p.y >= self.min.y && p.y <= self.max.y;
        x && y
    }

    pub fn normalize(&self) -> Self {
        let mut ret = Self::default();
        ret.min.x = self.min.x.min(self.max.x);
        ret.min.y = self.min.y.min(self.max.y);
        ret.max.x = self.min.x.max(self.max.x);
        ret.max.y = self.min.y.max(self.max.y);
        ret
    }

    pub fn constrain_to(&self, other: &Self) -> Self {
        let mut ret = Self::default();
        ret.min.x = self.min.x.max(other.min.x);
        ret.min.y = self.min.y.max(other.min.y);
        ret.max.x = self.max.x.min(other.max.x);
        ret.max.y = self.max.y.min(other.max.y);
        ret
    }

    pub fn translate(&self, delta: &Vec2) -> Self {
        Self::new(self.min + *delta, self.max + *delta)
    }

    pub fn width(&self) -> f32 {
        self.max.x - self.min.x
    }

    pub fn height(&self) -> f32 {
        self.max.y - self.min.y
    }

    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }
}

impl std::ops::Mul<f32> for Rect {
    type Output = Self;

    fn mul(self, factor: f32) -> Self::Output {
        Self::new(self.min * factor, self.max * factor)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RectFill {
    TextureHandle(u32),
    Color(Rgba8),
}

/// computes the vertex position offset away the from center caused by line width.
fn compute_line_width_offset(a: &Vec2, b: &Vec2, width: f32) -> Vec2 {
    // direction defines how the line is oriented in space. it allows to know
    // which way to move the vertices to create the desired thickness.
    let dir: Vec2 = *b - *a;

    // normalizing the direction vector converts it into a unit vector (length
    // of 1). normalization ensures that the offset is proportional to the line
    // width, regardless of the line's length.
    let norm_dir: Vec2 = dir.normalize_or_zero();

    // create a vector that points outward from the line. we want to move the
    // vertices away from the center of the line, not along its length.
    let perp: Vec2 = norm_dir.perp();

    // to distribute the offset evenly on both sides of the line
    let offset = perp * (width * 0.5);

    offset
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

    pub fn push_line(&mut self, a: Vec2, b: Vec2, width: f32, color: Rgba8) {
        let idx = self.vertices.len() as u32;
        let perp = compute_line_width_offset(&a, &b, width);

        // top left
        self.push_vertex(Vertex {
            position: a - perp,
            tex_coord: Vec2::new(0.0, 0.0),
            color,
        });
        // top right
        self.push_vertex(Vertex {
            position: b - perp,
            tex_coord: Vec2::new(1.0, 0.0),
            color,
        });
        // bottom right
        self.push_vertex(Vertex {
            position: b + perp,
            tex_coord: Vec2::new(1.0, 1.0),
            color,
        });
        // bottom left
        self.push_vertex(Vertex {
            position: a + perp,
            tex_coord: Vec2::new(0.0, 1.0),
            color,
        });

        // top left -> top right -> bottom right
        self.push_triangle(idx + 0, idx + 1, idx + 2);
        // bottom right -> bottom left -> top left
        self.push_triangle(idx + 2, idx + 3, idx + 0);

        self.commit(None);
    }

    pub fn push_rect_filled(&mut self, rect: Rect, fill: RectFill) {
        let idx = self.vertices.len() as u32;

        let (color, texture_handle) = match fill {
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

    pub fn push_rect_outlined(&mut self, rect: Rect, width: f32, color: Rgba8) {
        let top_left = rect.min;
        let top_right = Vec2::new(rect.max.x, rect.min.y);
        let bottom_right = rect.max;
        let bottom_left = Vec2::new(rect.min.x, rect.max.y);

        let offset = width * 0.5;

        // horizontal lines:
        // extened to left and right by outline width, shifted to top by half of
        // outline width.
        self.push_line(
            Vec2::new(top_left.x - width, top_left.y - offset),
            Vec2::new(top_right.x + width, top_right.y - offset),
            width,
            color,
        );
        self.push_line(
            Vec2::new(bottom_left.x - width, bottom_left.y + offset),
            Vec2::new(bottom_right.x + width, bottom_right.y + offset),
            width,
            color,
        );

        // vertical lines:
        // shifted to right and left by half of outlined width
        self.push_line(
            Vec2::new(top_right.x + offset, top_right.y),
            Vec2::new(bottom_right.x + offset, bottom_right.y),
            width,
            color,
        );
        self.push_line(
            Vec2::new(top_left.x - offset, top_left.y),
            Vec2::new(bottom_left.x - offset, bottom_left.y),
            width,
            color,
        );

        self.commit(None);
    }

    pub fn push_rect(
        &mut self,
        rect: Rect,
        fill: Option<RectFill>,
        outline_width: Option<f32>,
        outline_color: Option<Rgba8>,
    ) {
        if let Some(fill) = fill {
            self.push_rect_filled(rect.clone(), fill);
        }
        if let (Some(width), Some(color)) = (outline_width, outline_color) {
            self.push_rect_outlined(rect, width, color);
        }
    }
}
