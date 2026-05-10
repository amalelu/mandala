// SPDX-License-Identifier: MPL-2.0

//! Screenshot helpers + PNG encoding. The wgpu side lives in
//! `src/application/renderer/render.rs` (`Renderer::capture_frame`);
//! this module owns the pure, GPU-free pieces:
//!
//! - `padded_bytes_per_row` — wgpu requires `bytes_per_row` to be
//!   a multiple of 256 in `CopyTextureToBuffer`. The capture buffer
//!   is allocated with the padded stride and we strip the padding
//!   row-by-row before encoding.
//! - `strip_padding` — copies the live `width * 4` bytes per row
//!   out of the padded buffer into a tight contiguous RGBA8 vec.
//! - `encode_png` — feeds the tight vec to the `png` crate.

/// wgpu's `bytes_per_row` alignment in `CopyTextureToBuffer`.
pub const COPY_BYTES_PER_ROW_ALIGNMENT: u32 = 256;

/// Compute the padded bytes-per-row for an RGBA8 capture of `width`
/// pixels. Always a multiple of 256.
pub fn padded_bytes_per_row(width: u32) -> u32 {
    let unpadded = width * 4;
    let pad = (COPY_BYTES_PER_ROW_ALIGNMENT - unpadded % COPY_BYTES_PER_ROW_ALIGNMENT)
        % COPY_BYTES_PER_ROW_ALIGNMENT;
    unpadded + pad
}

/// Drop wgpu's per-row alignment bytes, producing a contiguous
/// `width * height * 4` RGBA8 buffer.
pub fn strip_padding(padded: &[u8], width: u32, height: u32) -> Vec<u8> {
    let padded_stride = padded_bytes_per_row(width) as usize;
    let live_stride = (width * 4) as usize;
    let mut out = Vec::with_capacity(live_stride * height as usize);
    for row in 0..height as usize {
        let start = row * padded_stride;
        let end = start + live_stride;
        out.extend_from_slice(&padded[start..end]);
    }
    out
}

/// Encode tight RGBA8 bytes (no padding) into a PNG `Vec<u8>`.
/// Errors when input length doesn't match `width * height * 4`.
pub fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    if rgba.len() != (width as usize * height as usize * 4) {
        return Err(format!(
            "rgba buffer wrong size: have {}, want {}",
            rgba.len(),
            width * height * 4
        ));
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("png header: {e}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| format!("png write: {e}"))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_padded_bytes_per_row_table() {
        // (width pixels, expected padded bytes_per_row)
        for (w, want) in [
            (1, 256),
            (64, 256),
            (65, 512),
            (128, 512),
            (1024, 4096),
            (2, 256),
            (63, 256),
        ] {
            assert_eq!(padded_bytes_per_row(w), want, "width = {}", w);
        }
    }

    #[test]
    fn test_strip_padding_removes_alignment_bytes() {
        // 2x2 RGBA: live stride = 8 bytes, padded stride = 256.
        let width = 2u32;
        let height = 2u32;
        let padded_stride = padded_bytes_per_row(width) as usize;
        let mut buf = vec![0u8; padded_stride * height as usize];
        // Live bytes for row 0: bytes 0..8 = 1..=8.
        for (i, b) in buf[..8].iter_mut().enumerate() {
            *b = (i + 1) as u8;
        }
        // Live bytes for row 1: bytes padded_stride..padded_stride+8 = 9..=16.
        for (i, b) in buf[padded_stride..padded_stride + 8]
            .iter_mut()
            .enumerate()
        {
            *b = (i + 9) as u8;
        }
        let tight = strip_padding(&buf, width, height);
        assert_eq!(tight, (1u8..=16).collect::<Vec<u8>>());
    }

    #[test]
    fn test_encode_png_rejects_wrong_size() {
        let bad = vec![0u8; 5];
        let err = encode_png(&bad, 1, 1).unwrap_err();
        assert!(err.contains("rgba buffer wrong size"));
    }

    #[test]
    fn test_encode_png_minimal_smoke() {
        let bytes = encode_png(&[0xFF; 16], 2, 2).expect("encode");
        // PNG magic.
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }
}
