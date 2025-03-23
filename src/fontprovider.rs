use anyhow::anyhow;

use crate::genvec::{GenVec, Handle};

// NOTE: font provider is a separate thing with the idea in mind that it might grow into something
// more then it is right now.. maybe it'll be able to look up and load system fonts, etc.

pub struct Font {
    pub inner: fontdue::Font,
    pub size: f32,
}

#[derive(Default)]
pub struct FontProvider {
    fonts: GenVec<Font>,
}

impl FontProvider {
    pub fn create_font<D>(&mut self, data: D, size: f32) -> anyhow::Result<Handle<Font>>
    where
        D: AsRef<[u8]>,
    {
        let font = fontdue::Font::from_bytes(data.as_ref(), fontdue::FontSettings {
            scale: size,
            ..Default::default()
        })
        .map_err(|err| anyhow!("could not construct a fontdue font: {err}"))?;

        // ensure that the given font+size does not already exist. this is not super efficient, but
        // i don't care, i don't want to use hash maps or sets or whatever.
        if self
            .fonts
            .iter_values()
            .find(|it| it.inner.file_hash() == font.file_hash() && it.size == size)
            .is_some()
        {
            return Err(anyhow!("such font already exists"));
        }

        Ok(self.fonts.insert(Font { inner: font, size }))
    }

    pub fn get_font(&self, font_handle: Handle<Font>) -> &Font {
        &self.fonts.get(font_handle)
    }
}
