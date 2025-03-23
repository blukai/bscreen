use std::collections::HashMap;

use crate::{
    fontprovider::{Font, FontProvider},
    genvec::Handle,
    gl,
    ntree::NTreeNode,
    texturepacker::{
        DEFAULT_TEXTURE_HEIGHT, DEFAULT_TEXTURE_WIDTH, TexturePacker, TexturePackerEntry,
    },
};

pub struct FontTextureCacheContext<'a> {
    pub font_provider: &'a FontProvider,
    pub gl_lib: &'static gl::Lib,
}

struct Page {
    texture_packer: TexturePacker,
    texture: gl::Texture2D,
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
struct CharKey {
    font_handle: Handle<Font>,
    ch: char,
}

struct CharValue {
    page_index: usize,
    entry_handle: Handle<NTreeNode<TexturePackerEntry>>,
}

#[derive(Default)]
pub struct FontTextureCache {
    pages: Vec<Page>,
    // TODO: rb tree or something might perform better?
    chars: HashMap<CharKey, CharValue>,
}

impl FontTextureCache {
    fn allocate_page(&mut self, ctx: &FontTextureCacheContext) -> usize {
        let texture_packer = TexturePacker::default();
        let texture = unsafe {
            gl::Texture2D::new(
                ctx.gl_lib,
                DEFAULT_TEXTURE_WIDTH,
                DEFAULT_TEXTURE_HEIGHT,
                crate::gfx::TextureFormat::R8Unorm,
                None,
            )
        };

        let page_index = self.pages.len();
        self.pages.push(Page {
            texture_packer,
            texture,
        });

        page_index
    }

    fn allocate_char(
        &mut self,
        font_handle: Handle<Font>,
        ch: char,
        ctx: &FontTextureCacheContext,
    ) {
        let font = ctx.font_provider.get_font(font_handle);
        let (metrics, bitmap) = font.inner.rasterize(ch, font.size);

        // TODO: maybe do not assert, but return an error indicating that the page is too small to
        // fit font of this size.
        assert!(metrics.width as u32 <= DEFAULT_TEXTURE_WIDTH);
        assert!(metrics.height as u32 <= DEFAULT_TEXTURE_HEIGHT);

        let mut page_index = self.pages.len().saturating_sub(1);
        let mut entry_handle = self.pages.get_mut(page_index).and_then(|page| {
            page.texture_packer
                .insert(metrics.width as u32, metrics.height as u32)
        });

        // new page is needed
        if entry_handle.is_none() {
            page_index = self.allocate_page(ctx);
            entry_handle = self.pages[page_index]
                .texture_packer
                .insert(metrics.width as u32, metrics.height as u32);
            assert!(entry_handle.is_some());
        }

        let entry_handle = entry_handle.unwrap();

        let page = &self.pages[page_index];
        let entry = page.texture_packer.get(entry_handle);

        unsafe {
            ctx.gl_lib
                .BindTexture(gl::sys::TEXTURE_2D, page.texture.handle);
            ctx.gl_lib.TexSubImage2D(
                gl::sys::TEXTURE_2D,
                0,
                entry.x as _,
                entry.y as _,
                entry.w as _,
                entry.h as _,
                page.texture.format_desc.format,
                page.texture.format_desc.ty,
                bitmap.as_ptr() as _,
            );
        }

        self.chars.insert(CharKey { font_handle, ch }, CharValue {
            page_index,
            entry_handle,
        });
    }

    /// returns a texture and coords for the given character and font; generates and uploads
    /// texture if necessary.
    pub fn get_texture_for_char(
        &mut self,
        font_handle: Handle<Font>,
        ch: char,
        ctx: &FontTextureCacheContext,
    ) -> (&gl::Texture2D, f32, f32, f32, f32) {
        let char_key = CharKey { font_handle, ch };

        if !self.chars.contains_key(&char_key) {
            self.allocate_char(font_handle, ch, ctx);
        }

        let ch = self.chars.get(&char_key).unwrap();
        let page = &self.pages[ch.page_index];
        let entry = page.texture_packer.get(ch.entry_handle);

        (
            &page.texture,
            entry.x as f32 / DEFAULT_TEXTURE_WIDTH as f32, // x1
            entry.y as f32 / DEFAULT_TEXTURE_HEIGHT as f32, // y1
            (entry.x + entry.w) as f32 / DEFAULT_TEXTURE_WIDTH as f32, // x2
            (entry.y + entry.h) as f32 / DEFAULT_TEXTURE_HEIGHT as f32, // y2
        )
    }
}
