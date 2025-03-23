use crate::{
    fontprovider::{Font, FontProvider},
    fonttexturecache::{FontTextureCache, FontTextureCacheContext},
    genvec::Handle,
    gfx::{DrawBuffer, Rect, RectFill, Vec2},
    gl,
    input::Event,
};

const PADDING: f32 = 24.0;

pub struct WelcomeUpdateData<'a> {
    pub view_rect: Rect,
    pub any_crop_has_selection: bool,
    pub this_screen_focused: bool,
    pub font_provider: &'a FontProvider,
    pub font_handle: Handle<Font>,
}

pub struct WelcomeDrawData<'a> {
    pub font_provider: &'a FontProvider,
    pub font_handle: Handle<Font>,
    pub font_texture_cache: &'a mut FontTextureCache,
    pub gl_lib: &'static gl::Lib,
}

pub struct Welcome {
    text_layout: fontdue::layout::Layout,
}

impl Default for Welcome {
    fn default() -> Self {
        Self {
            text_layout: fontdue::layout::Layout::new(
                fontdue::layout::CoordinateSystem::PositiveYDown,
            ),
        }
    }
}

impl Welcome {
    pub fn update(&mut self, _event: &Event, data: WelcomeUpdateData) {
        if data.any_crop_has_selection || !data.this_screen_focused {
            self.text_layout.clear();
            return;
        }

        self.text_layout.reset(&fontdue::layout::LayoutSettings {
            x: PADDING,
            y: PADDING,
            max_width: Some(data.view_rect.width() - PADDING * 2.0),
            max_height: Some(data.view_rect.height() - PADDING * 2.0),
            horizontal_align: fontdue::layout::HorizontalAlign::Center,
            vertical_align: fontdue::layout::VerticalAlign::Middle,
            line_height: 1.33,
            ..fontdue::layout::LayoutSettings::default()
        });

        let font = data.font_provider.get_font(data.font_handle);
        self.text_layout.append(
            &[&font.inner],
            &fontdue::layout::TextStyle::new(
                concat!(
                    "to select a region, click and hold your mouse or trackpad button while dragging the crosshair.\n",
                    "to select the entire screen, press ctrl+a.\n",
                    "to save a screenshot to the clipboard, press ctrl+c.\n",
                    "to exit, press esc.",
                ),
                font.size, 0),
        );
    }

    pub fn draw(&mut self, draw_buffer: &mut DrawBuffer, data: WelcomeDrawData) {
        let glyphs = self.text_layout.glyphs();
        for glyph in glyphs.iter() {
            let (tex, x1, y1, x2, y2) = data.font_texture_cache.get_texture_for_char(
                data.font_handle,
                glyph.parent,
                &FontTextureCacheContext {
                    font_provider: data.font_provider,
                    gl_lib: data.gl_lib,
                },
            );

            let min = Vec2::new(glyph.x, glyph.y);
            let size = Vec2::new(glyph.width as f32, glyph.height as f32);
            draw_buffer.push_rect_filled(Rect::new(min, min + size), RectFill::Texture {
                handle: tex.handle,
                coords: Rect::new(Vec2::new(x1, y1), Vec2::new(x2, y2)),
            });
        }
    }
}
