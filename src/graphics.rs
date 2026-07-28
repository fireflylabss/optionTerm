//! Kitty graphics protocol rendering.
//!
//! libghostty-vt parses and stores the images/placements; this module turns
//! them into Cairo surfaces and paints them under and over the text layer.

use std::collections::HashMap;

use anyhow::{Result, anyhow};
use gtk4::cairo;
use libghostty_vt::{
    Terminal,
    alloc::{Allocator, Bytes},
    kitty::graphics::{self, DecodePng, DecodedImage, Layer, PlacementIterator},
};

/// How much image data a single terminal may hold (matches Ghostty's default).
pub const STORAGE_LIMIT: u64 = 320 * 1024 * 1024;

/// PNG decoder backed by the `png` crate.
///
/// libghostty ships one, but its buffer is only `reserve`d and never resized,
/// so `next_frame` always fails; decode into our own buffer instead.
struct PngDecoder;

impl DecodePng for PngDecoder {
    fn decode_png<'alloc>(
        &mut self,
        alloc: &'alloc Allocator<'_>,
        data: &[u8],
    ) -> Option<DecodedImage<'alloc>> {
        use png::{Decoder, Transformations};

        let mut decoder = Decoder::new(std::io::Cursor::new(data));
        // libghostty only accepts RGBA8: expand palette/grayscale and drop
        // 16-bit channels down to 8-bit.
        decoder.set_transformations(Transformations::ALPHA | Transformations::STRIP_16);

        let mut reader = decoder.read_info().ok()?;
        let mut buf = vec![0u8; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut buf).ok()?;

        let mut bytes = Bytes::new_with_alloc(alloc, info.buffer_size()).ok()?;
        bytes.copy_from_slice(&buf[..info.buffer_size()]);

        Some(DecodedImage {
            width: info.width,
            height: info.height,
            data: bytes,
        })
    }
}

/// Install the PNG decoder for the current thread.
///
/// libghostty stores the callback in thread-local storage, so this must be
/// per-thread rather than a process-wide `Once`: with a `Once` only the first
/// thread would get a decoder and every other thread would silently reject
/// every PNG.
pub fn install_png_decoder() {
    thread_local! {
        static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    INSTALLED.with(|installed| {
        if installed.get() {
            return;
        }
        match graphics::set_png_decoder(Some(Box::new(PngDecoder))) {
            Ok(()) => installed.set(true),
            Err(err) => tracing::warn!("could not install the PNG decoder: {err:?}"),
        }
    });
}

/// Cairo surfaces for decoded images, keyed by image id.
///
/// Images are immutable once stored, so a cached surface only needs to be
/// rebuilt when the storage generation for that id changes.
#[derive(Default)]
pub struct ImageCache {
    surfaces: HashMap<u32, (u64, cairo::ImageSurface)>,
    seen: Vec<u32>,
}

impl ImageCache {
    /// Start a frame. Eviction is frame-scoped rather than per-`draw` call
    /// because each frame paints several layers, and an image only appears in
    /// the layer matching its z-index.
    pub fn begin_frame(&mut self) {
        self.seen.clear();
    }

    /// Drop surfaces for images that were not placed anywhere this frame.
    pub fn end_frame(&mut self) {
        let seen = std::mem::take(&mut self.seen);
        self.surfaces.retain(|id, _| seen.contains(id));
        self.seen = seen;
    }
}

/// Convert stored pixel data into premultiplied BGRA, the memory layout Cairo
/// expects for `Format::ARgb32` on little-endian machines.
fn to_cairo_bgra(data: &[u8], width: u32, height: u32, stride: usize) -> Option<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    let pixels = w.checked_mul(h)?;
    // The stored format is not always what `Image::format` reports (PNGs are
    // decoded to RGBA), so derive the channel count from the buffer length.
    let channels = data
        .len()
        .checked_div(pixels)
        .filter(|c| (1..=4).contains(c))?;

    let mut out = vec![0u8; stride * h];
    for y in 0..h {
        for x in 0..w {
            let src = (y * w + x) * channels;
            let (r, g, b, a) = match channels {
                1 => (data[src], data[src], data[src], 255),
                2 => (data[src], data[src], data[src], data[src + 1]),
                3 => (data[src], data[src + 1], data[src + 2], 255),
                _ => (data[src], data[src + 1], data[src + 2], data[src + 3]),
            };
            // Cairo stores ARGB32 premultiplied by alpha.
            let pm = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
            let dst = y * stride + x * 4;
            out[dst] = pm(b);
            out[dst + 1] = pm(g);
            out[dst + 2] = pm(r);
            out[dst + 3] = a;
        }
    }
    Some(out)
}

/// Geometry needed to place images on the surface.
#[derive(Clone, Copy)]
pub struct Metrics {
    pub padding_x: f64,
    pub padding_y: f64,
    pub cell_width: f64,
    pub cell_height: f64,
    pub width: f64,
    pub height: f64,
}

/// Paint every placement in `layer`.
///
/// `terminal` and `iter` are passed separately (rather than through a single
/// `&mut Session`) because the graphics handle borrows the terminal immutably
/// while the iterator needs a mutable borrow of itself.
pub fn draw(
    terminal: &Terminal<'static, 'static>,
    iter: &mut PlacementIterator<'static>,
    cache: &mut ImageCache,
    cr: &cairo::Context,
    metrics: Metrics,
    layer: Layer,
) -> Result<()> {
    let graphics = terminal.kitty_graphics().map_err(|e| anyhow!("{e:?}"))?;
    let mut placements = iter.update(&graphics).map_err(|e| anyhow!("{e:?}"))?;
    placements.set_layer(layer).map_err(|e| anyhow!("{e:?}"))?;

    while let Some(placement) = placements.next() {
        let Ok(image_id) = placement.image_id() else {
            continue;
        };
        let Some(image) = graphics.image(image_id) else {
            continue;
        };
        cache.seen.push(image_id);

        // Where and how big, in device pixels.
        let Ok(Some(pos)) = placement.viewport_pos(&image, terminal) else {
            continue;
        };
        let (Ok(size), Ok(src)) = (
            placement.pixel_size(&image, terminal),
            placement.source_rect(&image),
        ) else {
            continue;
        };
        if size.width == 0 || size.height == 0 || src.width == 0 || src.height == 0 {
            continue;
        }

        let surface = match cached_surface(cache, &graphics, image_id) {
            Some(surface) => surface,
            None => continue,
        };

        let x_offset = placement.x_offset().unwrap_or(0) as f64;
        let y_offset = placement.y_offset().unwrap_or(0) as f64;
        let dest_x = metrics.padding_x + pos.col as f64 * metrics.cell_width + x_offset;
        let dest_y = metrics.padding_y + pos.row as f64 * metrics.cell_height + y_offset;
        let dest_w = size.width as f64;
        let dest_h = size.height as f64;

        // Skip placements scrolled fully out of view.
        if dest_x > metrics.width || dest_y > metrics.height {
            continue;
        }
        if dest_x + dest_w < 0.0 || dest_y + dest_h < 0.0 {
            continue;
        }

        cr.save().ok();
        cr.rectangle(dest_x, dest_y, dest_w, dest_h);
        cr.clip();
        cr.translate(dest_x, dest_y);
        cr.scale(dest_w / src.width as f64, dest_h / src.height as f64);
        // Negative origin selects the source rect out of the full image.
        if cr
            .set_source_surface(surface, -(src.x as f64), -(src.y as f64))
            .is_ok()
        {
            cr.paint().ok();
        }
        cr.restore().ok();
    }

    Ok(())
}

fn cached_surface<'a>(
    cache: &'a mut ImageCache,
    graphics: &graphics::Graphics<'_>,
    image_id: u32,
) -> Option<&'a cairo::ImageSurface> {
    let image = graphics.image(image_id)?;
    let generation = image.generation().ok()?;

    let stale = cache
        .surfaces
        .get(&image_id)
        .is_none_or(|(cached, _)| *cached != generation);

    if stale {
        let width = image.width().ok()?;
        let height = image.height().ok()?;
        let data = image.data().ok()?;
        if width == 0 || height == 0 {
            return None;
        }
        let stride = cairo::Format::ARgb32.stride_for_width(width).ok()? as usize;
        let bgra = to_cairo_bgra(data, width, height, stride)?;
        let surface = cairo::ImageSurface::create_for_data(
            bgra,
            cairo::Format::ARgb32,
            width as i32,
            height as i32,
            stride as i32,
        )
        .ok()?;
        cache.surfaces.insert(image_id, (generation, surface));
    }

    cache.surfaces.get(&image_id).map(|(_, surface)| surface)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libghostty_vt::{Terminal, TerminalOptions};

    /// 4x2 RGBA PNG: red / semi-transparent blue checkerboard.
    const PNG_4X2: &str = "iVBORw0KGgoAAAANSUhEUgAAAAQAAAACCAYAAAB/qH1jAAAAFUlEQVR4nGP4z8AAQg0wmgGZAxIBAOhYDfV2dXyMAAAAAElFTkSuQmCC";

    fn terminal_with_image() -> Terminal<'static, 'static> {
        install_png_decoder();
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 80,
            rows: 24,
            max_scrollback: 0,
        })
        .expect("terminal");
        terminal.resize(80, 24, 8, 16).expect("resize");
        terminal
            .set_kitty_image_storage_limit(STORAGE_LIMIT)
            .expect("enable kitty graphics");
        // a=T (transmit + display), f=100 (PNG), q=2 (suppress responses).
        let cmd = format!("\x1b_Ga=T,f=100,q=2;{PNG_4X2}\x1b\\");
        terminal.vt_write(cmd.as_bytes());
        terminal
    }

    /// The PNG must actually be decoded — libghostty rejects images whose
    /// decoder returns None, so a broken decoder yields zero placements.
    #[test]
    fn decodes_and_places_a_png() {
        let terminal = terminal_with_image();
        let graphics = terminal.kitty_graphics().expect("graphics handle");
        let mut iter = PlacementIterator::new().expect("iterator");
        let mut placements = iter.update(&graphics).expect("update");

        let placement = placements.next().expect("one placement was stored");
        let image_id = placement.image_id().expect("image id");
        let image = graphics.image(image_id).expect("image in storage");

        assert_eq!(image.width().unwrap(), 4);
        assert_eq!(image.height().unwrap(), 2);
        // Decoded to RGBA8 => 4 bytes per pixel.
        assert_eq!(image.data().unwrap().len(), 4 * 2 * 4);
    }

    /// Pixels must reach Cairo as premultiplied BGRA, otherwise images render
    /// with swapped channels and wrong alpha.
    #[test]
    fn converts_rgba_to_premultiplied_bgra() {
        // One opaque red pixel, one 50% blue pixel.
        let rgba = [255, 0, 0, 255, 0, 0, 255, 128];
        let stride = 8; // 2 px * 4 bytes
        let out = to_cairo_bgra(&rgba, 2, 1, stride).expect("conversion");

        // Opaque red -> B=0 G=0 R=255 A=255
        assert_eq!(&out[0..4], &[0, 0, 255, 255]);
        // Half-alpha blue -> B premultiplied to 128, A=128
        assert_eq!(out[7], 128);
        assert_eq!(out[4], 128, "blue channel must be premultiplied");
        assert_eq!(&out[5..7], &[0, 0]);
    }

    /// Grayscale and RGB buffers are detected from the buffer length.
    #[test]
    fn infers_channel_count_from_buffer_length() {
        let rgb = [10, 20, 30];
        let out = to_cairo_bgra(&rgb, 1, 1, 4).expect("rgb");
        assert_eq!(&out[0..4], &[30, 20, 10, 255]);

        let gray = [77];
        let out = to_cairo_bgra(&gray, 1, 1, 4).expect("gray");
        assert_eq!(&out[0..4], &[77, 77, 77, 255]);
    }

    /// End-to-end: a transmitted image must actually reach the Cairo surface.
    /// Guards the geometry math (viewport pos, source rect, scaling) which a
    /// pure decoder test would not catch.
    #[test]
    fn draws_placement_onto_a_cairo_surface() {
        let terminal = terminal_with_image();
        let mut iter = PlacementIterator::new().expect("iterator");
        let mut cache = ImageCache::default();

        let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, 64, 64).expect("surface");
        let cr = cairo::Context::new(&surface).expect("context");
        // Paint it black so any image pixel is a visible change.
        cr.set_source_rgb(0.0, 0.0, 0.0);
        cr.paint().expect("clear");

        let metrics = Metrics {
            padding_x: 0.0,
            padding_y: 0.0,
            cell_width: 8.0,
            cell_height: 16.0,
            width: 64.0,
            height: 64.0,
        };
        cache.begin_frame();
        draw(&terminal, &mut iter, &mut cache, &cr, metrics, Layer::All).expect("draw");
        cache.end_frame();
        drop(cr);

        assert_eq!(cache.surfaces.len(), 1, "image surface should be cached");

        let data = surface.take_data().expect("surface data");
        let painted = data.chunks_exact(4).any(|px| px[..3] != [0, 0, 0]);
        assert!(painted, "the placement did not paint any pixels");
    }
}
