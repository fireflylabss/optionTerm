//! Kitty graphics helpers.
//!
//! libghostty-vt parses and stores the images/placements; this module holds
//! the PNG decoder it needs for `f=100` payloads and the conversion into the
//! RGBA8 buffers `Frame::kitty.images` carries.

use std::sync::Arc;

use libghostty_vt::{
    alloc::{Allocator, Bytes},
    kitty::graphics::{self, DecodePng, DecodedImage},
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

/// Expand stored pixels to RGBA8.
///
/// `Image::data` is already decoded and uncompressed but keeps the
/// transmission's channel layout (RGB stays 3 bytes/px, grayscale 1), so the
/// channel count is derived from the buffer length rather than
/// `Image::format` — PNGs report `Png` yet arrive decoded as RGBA.
pub fn to_rgba8(data: &[u8], width: u32, height: u32) -> Option<Arc<[u8]>> {
    let pixels = (width as usize).checked_mul(height as usize)?;
    let channels = data
        .len()
        .checked_div(pixels)
        .filter(|c| (1..=4).contains(c))?;

    let mut out = Vec::with_capacity(pixels * 4);
    for px in 0..pixels {
        let src = px * channels;
        let (r, g, b, a) = match channels {
            1 => (data[src], data[src], data[src], 255),
            2 => (data[src], data[src], data[src], data[src + 1]),
            3 => (data[src], data[src + 1], data[src + 2], 255),
            _ => (data[src], data[src + 1], data[src + 2], data[src + 3]),
        };
        out.extend_from_slice(&[r, g, b, a]);
    }
    Some(out.into())
}
