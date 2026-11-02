//! Encoded material-map animation. Channels are data: no sRGB conversion or alpha blending.
//! Frames and their mip chains are prepared once; a sprite uploads only when its timeline changes.
use crate::sprites::{FrameBlend, SpriteAnimation};

struct Sprite {
    animation: SpriteAnimation,
    shown: Option<FrameBlend>,
    frames: Vec<Vec<u8>>,
}
pub(super) struct MaterialAnimator {
    sprites: Vec<Sprite>,
    scratch: Vec<u8>,
    changed: Vec<u32>,
    size: u32,
}
impl MaterialAnimator {
    pub fn new(animations: &[SpriteAnimation], size: u32) -> Self {
        Self {
            sprites: animations
                .iter()
                .map(|animation| Sprite {
                    animation: animation.clone(),
                    shown: None,
                    frames: (0..animation.frame_count())
                        .map(|frame| super::box_mip_chain(animation.pixels(frame), size, 1))
                        .collect(),
                })
                .collect(),
            scratch: Vec::new(),
            changed: Vec::new(),
            size,
        }
    }
    pub fn animate(
        &mut self,
        queue: &wgpu::Queue,
        arrays: &[wgpu::Texture],
        split: u32,
        tick: u64,
    ) -> &[u32] {
        self.changed.clear();
        for sprite in &mut self.sprites {
            let blend = sprite.animation.blend(tick);
            if sprite.shown == Some(blend) {
                continue;
            }
            let layer = sprite.animation.layer;
            let Some(texture) = arrays.get((layer / split) as usize) else {
                continue;
            };
            sprite.shown = Some(blend);
            self.changed.push(layer);
            let first = &sprite.frames[blend.frame];
            let next = &sprite.frames[blend.next];
            self.scratch.resize(first.len(), 0);
            interpolate(first, next, blend.millis, &mut self.scratch);
            let mut offset = 0;
            for level in 0..texture.mip_level_count() {
                let side = (self.size >> level).max(1);
                let bytes = (side * side * 4) as usize;
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: level,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer % split,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &self.scratch[offset..offset + bytes],
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(side * 4),
                        rows_per_image: Some(side),
                    },
                    wgpu::Extent3d {
                        width: side,
                        height: side,
                        depth_or_array_layers: 1,
                    },
                );
                offset += bytes;
            }
        }
        &self.changed
    }
}
fn interpolate(first: &[u8], next: &[u8], millis: u32, out: &mut [u8]) {
    let millis = millis.min(1000);
    for ((out, &a), &b) in out.iter_mut().zip(first).zip(next) {
        *out = ((u32::from(a) * (1000 - millis) + u32::from(b) * millis) / 1000) as u8;
    }
}

#[cfg(test)]
#[path = "pack_materials/tests.rs"]
mod tests;
