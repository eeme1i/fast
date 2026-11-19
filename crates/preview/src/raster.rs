use std::{
    collections::VecDeque,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use typst::utils::Scalar;
use typst_layout::{Page, PagedDocument};
use typst_render::RenderOptions;

use crate::{Bitmap, RasterRequest};

const CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_PIXELS: f32 = 16_000_000.0;

#[derive(Default)]
pub(crate) struct Cache {
    entries: VecDeque<(Key, Bitmap)>,
    bytes: usize,
    next_id: u64,
}

#[derive(PartialEq, Eq)]
struct Key {
    hash: u64,
    scale: u16,
    inverted: bool,
}

impl Cache {
    pub fn draw(
        &mut self,
        doc: &PagedDocument,
        request: &RasterRequest,
    ) -> Result<Vec<Bitmap>, String> {
        let mut result = Vec::new();
        for index in request.pages.clone() {
            let Some(page) = doc.pages().get(index) else {
                break;
            };
            let mut hasher = DefaultHasher::new();
            page.hash(&mut hasher);
            let key = Key {
                hash: hasher.finish(),
                scale: request.scale,
                inverted: request.inverted,
            };
            let bitmap = if let Some(hit) = self.entries.iter().position(|(old, _)| *old == key) {
                let Some((_, bitmap)) = self.entries.remove(hit) else {
                    continue;
                };
                self.bytes -= bitmap.rgba.len();
                bitmap
            } else {
                let size = page.frame.size();
                let requested = f32::from(request.scale.max(1)) / 8.0;
                let width = size.x.to_pt() as f32;
                let height = size.y.to_pt() as f32;
                let limit = (MAX_PIXELS / (width * height)).sqrt();
                let scale = requested
                    .min(limit * 0.99)
                    .min(8192.0 / width)
                    .min(8192.0 / height);
                let options = raster_options(page, scale)?;
                let pixmap = typst_render::render(page, &options);
                let width = pixmap.width();
                let height = pixmap.height();
                let mut rgba = pixmap.take();
                straight_rgba(&mut rgba, request.inverted);
                self.next_id += 1;
                Bitmap {
                    page: index,
                    id: self.next_id,
                    width,
                    height,
                    rgba: Arc::new(rgba),
                }
            };
            let bitmap = Bitmap {
                page: index,
                ..bitmap
            };
            result.push(Bitmap {
                rgba: Arc::clone(&bitmap.rgba),
                ..bitmap
            });
            self.bytes += bitmap.rgba.len();
            self.entries.push_front((key, bitmap));
            while self.bytes > CACHE_BYTES {
                let Some((_, bitmap)) = self.entries.pop_back() else {
                    break;
                };
                self.bytes -= bitmap.rgba.len();
            }
        }
        Ok(result)
    }
}

pub(crate) fn raster_options(page: &Page, scale: f32) -> Result<RenderOptions, String> {
    let size = page.frame.size();
    let width = (scale * size.x.to_pt() as f32).round().max(1.0);
    let height = (scale * size.y.to_pt() as f32).round().max(1.0);
    if !scale.is_finite()
        || scale <= 0.0
        || width * height > MAX_PIXELS
        || width.max(height) > 16384.0
    {
        return Err("The page is too large at this resolution".into());
    }
    Ok(RenderOptions {
        pixel_per_pt: Scalar::new(f64::from(scale)),
        render_bleed: false,
    })
}

fn straight_rgba(bytes: &mut [u8], inverted: bool) {
    for pixel in bytes.chunks_exact_mut(4) {
        let [red, green, blue, alpha] = pixel else {
            continue;
        };
        if *alpha == 0 {
            continue;
        }
        if *alpha != 255 {
            let alpha = u32::from(*alpha);
            for channel in [&mut *red, &mut *green, &mut *blue] {
                *channel = ((255 * u32::from(*channel) + alpha / 2) / alpha).min(255) as u8;
            }
        }
        if inverted {
            let max = f32::from((*red).max(*green).max(*blue));
            let min = f32::from((*red).min(*green).min(*blue));
            let luminance =
                0.2126 * f32::from(*red) + 0.7152 * f32::from(*green) + 0.0722 * f32::from(*blue);
            let blend = (1.0 - (max - min) / 80.0).max(0.0);
            let shift = (232.0 - 1.82 * luminance) * blend;
            for channel in [red, green, blue] {
                *channel = (f32::from(*channel) + shift).clamp(0.0, 255.0) as u8;
            }
        }
    }
}
