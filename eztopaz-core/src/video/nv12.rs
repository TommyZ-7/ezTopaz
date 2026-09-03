//! BGRA → NV12 conversion (BT.601 limited range, integer).
//!
//! HW encoders (NVENC/QSV/VAAPI/AMF/Vulkan) take NV12 system memory directly,
//! so sending NV12 through the pipe skips ffmpeg's BGRA→YUV420p swscale and
//! cuts pipe bandwidth to 3/8 (720p60: ~221MB/s → ~83MB/s).
//! Width and height must be even (all built-in profiles are).

/// NV12 byte size for `w×h` (Y plane + interleaved UV).
pub fn nv12_frame_size(w: u32, h: u32) -> usize {
    (w as usize) * (h as usize) * 3 / 2
}

fn clamp8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// One BGRA pixel → (Y, U, V) BT.601 limited range.
#[inline]
fn bgra_to_yuv_pixel(b: u8, g: u8, r: u8) -> (u8, i32, i32) {
    let (b, g, r) = (b as i32, g as i32, r as i32);
    let y = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
    // U/V kept unoffset here; caller adds 128 after averaging.
    let u = -38 * r - 74 * g + 112 * b;
    let v = 112 * r - 94 * g - 18 * b;
    (clamp8(y), u, v)
}

/// BGRA `w×h` → NV12. `src.len()` must be `w*h*4`; `w,h` even.
/// Wrong sizes yield a black frame (never panics on the capture path).
pub fn bgra_to_nv12(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut dst = vec![0u8; w * h * 3 / 2];
    bgra_to_nv12_into(&mut dst, src, w as u32, h as u32);
    dst
}

pub fn bgra_to_nv12_into(dst: &mut [u8], src: &[u8], w: u32, h: u32) {
    let (w, h) = (w as usize, h as usize);
    if w % 2 == 1 || h % 2 == 1 || w == 0 || h == 0 {
        fill_nv12_black(dst, w as u32, h as u32);
        return;
    }
    let need_src = w * h * 4;
    let need_dst = w * h * 3 / 2;
    if src.len() < need_src || dst.len() < need_dst {
        fill_nv12_black(dst, w as u32, h as u32);
        return;
    }
    let (y_plane, uv_plane) = dst[..need_dst].split_at_mut(w * h);
    // Y per pixel.
    for y in 0..h {
        for x in 0..w {
            let s = (y * w + x) * 4;
            let (yy, _, _) = bgra_to_yuv_pixel(src[s], src[s + 1], src[s + 2]);
            y_plane[y * w + x] = yy;
        }
    }
    // UV averaged over each 2x2 block.
    for by in 0..h / 2 {
        for bx in 0..w / 2 {
            let mut su = 0i32;
            let mut sv = 0i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let x = bx * 2 + dx;
                    let y = by * 2 + dy;
                    let s = (y * w + x) * 4;
                    let (_, u, v) = bgra_to_yuv_pixel(src[s], src[s + 1], src[s + 2]);
                    su += u;
                    sv += v;
                }
            }
            // average of 4 pixels: (sum/4 +128)>>8 +128
            let o = (by * (w / 2) + bx) * 2;
            uv_plane[o] = clamp8(((su / 4 + 128) >> 8) + 128);
            uv_plane[o + 1] = clamp8(((sv / 4 + 128) >> 8) + 128);
        }
    }
}

/// Scale BGRA `src_w×src_h` → NV12 `dst_w×dst_h` with aspect-fit letterbox.
/// Black bars are NV12 black (Y=16, UV=128).
pub fn scale_bgra_to_nv12(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Vec<u8> {
    // Reuse the tested integer BGRA scaler, then convert. One extra BGRA
    // allocation, but the pipe + swscale savings dominate.
    let bgra = super::sink::scale_bgra(src, src_w, src_h, dst_w, dst_h);
    bgra_to_nv12(&bgra, dst_w, dst_h)
}

fn fill_nv12_black(dst: &mut [u8], w: u32, h: u32) {
    let need = (w as usize) * (h as usize) * 3 / 2;
    if dst.len() < need {
        return;
    }
    let (y, uv) = dst[..need].split_at_mut((w as usize) * (h as usize));
    y.fill(16);
    uv.fill(128);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_is_w_h_3_2() {
        assert_eq!(nv12_frame_size(1280, 720), 1280 * 720 * 3 / 2);
        assert_eq!(nv12_frame_size(640, 360), 640 * 360 * 3 / 2);
    }

    #[test]
    fn black_maps_to_16_128() {
        let src = vec![0u8; 4 * 2 * 4]; // 4x2 black BGRA (alpha 0, ignored)
        let nv12 = bgra_to_nv12(&src, 4, 2);
        assert_eq!(nv12.len(), 12);
        assert!(nv12[..8].iter().all(|&y| y == 16));
        assert!(nv12[8..].iter().all(|&c| c == 128));
    }

    #[test]
    fn white_maps_near_235() {
        let mut src = vec![0u8; 2 * 2 * 4];
        for px in src.chunks_exact_mut(4) {
            px[0] = 255;
            px[1] = 255;
            px[2] = 255;
            px[3] = 255;
        }
        let nv12 = bgra_to_nv12(&src, 2, 2);
        assert!(nv12[0] >= 230, "Y for white ≈235, got {}", nv12[0]);
        assert!((nv12[4] as i32 - 128).abs() <= 2);
        assert!((nv12[5] as i32 - 128).abs() <= 2);
    }

    #[test]
    fn scale_to_nv12_letterboxes() {
        let src = vec![255u8; 4 * 4 * 4];
        let nv12 = scale_bgra_to_nv12(&src, 4, 4, 8, 4);
        assert_eq!(nv12.len(), 8 * 4 * 3 / 2);
        // Y plane: side bars are 16 (black), center is bright.
        assert_eq!(nv12[0], 16);
        assert!(nv12[4] >= 230, "center Y bright, got {}", nv12[4]);
    }

    #[test]
    fn bad_sizes_yield_black_not_panic() {
        let nv12 = bgra_to_nv12(&[0u8; 3], 4, 2);
        assert_eq!(nv12.len(), 12);
        assert!(nv12[..8].iter().all(|&y| y == 16));
    }
}
