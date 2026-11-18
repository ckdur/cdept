//! The frame of an xschem session as an egui texture.

use egui::{Context, TextureHandle, TextureOptions};
use libcdept::xschem::Session;

/// A texture kept in step with the frames of one session: the whole frame when it was
/// replaced, otherwise only the rectangle that changed.
#[derive(Default)]
pub struct Picture {
    texture: Option<TextureHandle>,
    /// Generation of the frame the texture was filled from; 0 before the first.
    generation: u64,
    size: [u32; 2],
    scratch: Vec<u8>,
}

impl Picture {
    /// Bring the texture up to date. `name` names the texture for egui's debugging.
    pub fn update(&mut self, session: &Session, ctx: &Context, name: &str) {
        let Self {
            texture,
            generation,
            size,
            scratch,
        } = self;
        session.frame(|frame| {
            let now = [frame.width(), frame.height()];
            let image = |rect: [u32; 2], pixels: &[u8]| {
                egui::ColorImage::from_rgba_premultiplied(
                    [rect[0] as usize, rect[1] as usize],
                    pixels,
                )
            };
            if texture.is_none() || *generation != frame.generation || *size != now {
                frame.copy(frame.full(), scratch);
                let image = image(now, scratch);
                match texture {
                    Some(texture) => texture.set(image, TextureOptions::NEAREST),
                    None => *texture = Some(ctx.load_texture(name, image, TextureOptions::NEAREST)),
                }
                *generation = frame.generation;
                *size = now;
            } else if let (Some(damage), Some(texture)) = (frame.damage, texture.as_mut()) {
                let rect = frame.copy(damage, scratch);
                if !rect.is_empty() {
                    texture.set_partial(
                        [rect.x as usize, rect.y as usize],
                        image([rect.width, rect.height], scratch),
                        TextureOptions::NEAREST,
                    );
                }
            }
        });
    }

    /// The texture and its size in pixels, once a frame has arrived.
    pub fn texture(&self) -> Option<(&TextureHandle, [u32; 2])> {
        let texture = self.texture.as_ref()?;
        (self.generation > 0).then_some((texture, self.size))
    }
}
