//! Live frames: a camera or a video shown on the graphics card without a texture made per frame.
//!
//! A [`Frame`] is one picture from a capture thread in the format the device delivers it (BGRA, NV12 or YUYV), tightly
//! packed. The GPU layer keeps **one** texture per format and size and writes the new bytes into it; the YUV -> RGB step
//! runs in the shader, so the processor never converts a pixel. [`Frame::to_pic`] does the same conversion on the
//! processor, for the case where there is no card (`Host::gpu_layer` false) and as the reference the shader is held to.
//!
//! The capture thread puts frames into a [`Mailbox`], which keeps only the newest one: a slow window never builds a queue
//! and never shows stale frames. The program wakes the window with [`crate::app::Waker`] and takes the frame in `draw`.

use crate::picture::Pic;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// How the bytes of a [`Frame`] are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// 4 bytes per pixel: blue, green, red, alpha (alpha ignored: a camera frame is opaque)
    Bgra,
    /// full-size luma plane (`w * h`), then one interleaved U,V pair per 2 x 2 pixels (`w * h / 2`); `w` and `h` even
    Nv12,
    /// packed `Y0 U Y1 V` for every two pixels (`w * h * 2`); `w` even. The common format of USB cameras
    Yuyv,
}

/// The YUV flavour of a frame (ignored for [`PixelFormat::Bgra`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    /// luma 16..235 and chroma 16..240 (video) instead of the whole 0..255 (JPEG-style)
    pub limited: bool,
    /// BT.709 coefficients (HD video) instead of BT.601 (what USB cameras send)
    pub bt709: bool,
}

impl Default for Color {
    /// What a webcam delivers: limited range, BT.601.
    fn default() -> Color {
        Color { limited: true, bt709: false }
    }
}

/// One live picture. Cheap to share (`Arc<Frame>`); the id says whether the bytes are new.
#[derive(Debug)]
pub struct Frame {
    pub id: u64,
    pub w: u32,
    pub h: u32,
    pub format: PixelFormat,
    pub color: Color,
    pub data: Vec<u8>,
}

/// The bytes a frame of this format and size must have, or `None` for a size the format cannot have.
pub fn frame_len(format: PixelFormat, w: u32, h: u32) -> Option<usize> {
    let (w, h) = (w as usize, h as usize);
    if w == 0 || h == 0 {
        return None;
    }
    match format {
        PixelFormat::Bgra => Some(w * h * 4),
        PixelFormat::Nv12 if w % 2 == 0 && h % 2 == 0 => Some(w * h * 3 / 2),
        PixelFormat::Yuyv if w % 2 == 0 => Some(w * h * 2),
        _ => None,
    }
}

impl Frame {
    /// A frame from tightly packed bytes. `None` when the length does not fit the format and size.
    pub fn new(w: u32, h: u32, format: PixelFormat, color: Color, data: Vec<u8>) -> Option<Frame> {
        if frame_len(format, w, h)? != data.len() {
            return None;
        }
        Some(Frame { id: NEXT_ID.fetch_add(1, Ordering::Relaxed), w, h, format, color, data })
    }

    /// The same picture converted on the processor into a `Pic` (premultiplied BGRA, opaque, no mips). The fallback when
    /// the card cannot show the frame.
    pub fn to_pic(&self) -> Pic {
        let (w, h) = (self.w as usize, self.h as usize);
        let mut out = vec![255u8; w * h * 4];
        match self.format {
            PixelFormat::Bgra => {
                out.copy_from_slice(&self.data);
                for p in out.chunks_exact_mut(4) {
                    p[3] = 255;
                }
            }
            PixelFormat::Nv12 => {
                let (luma, chroma) = self.data.split_at(w * h);
                for y in 0..h {
                    for x in 0..w {
                        let c = (y / 2) * w + (x / 2) * 2;
                        let px = yuv_to_bgr(luma[y * w + x], chroma[c], chroma[c + 1], self.color);
                        out[(y * w + x) * 4..(y * w + x) * 4 + 3].copy_from_slice(&px);
                    }
                }
            }
            PixelFormat::Yuyv => {
                for y in 0..h {
                    for x in 0..w {
                        let q = (y * w + x / 2) * 4;
                        let luma = self.data[q + (x % 2) * 2];
                        let px = yuv_to_bgr(luma, self.data[q + 1], self.data[q + 3], self.color);
                        out[(y * w + x) * 4..(y * w + x) * 4 + 3].copy_from_slice(&px);
                    }
                }
            }
        }
        Pic::from_bgra(self.w, self.h, out, true, false)
    }
}

/// One pixel from YUV, with the same arithmetic as the shader (floating point, then rounded).
fn yuv_to_bgr(y: u8, u: u8, v: u8, c: Color) -> [u8; 3] {
    let (y, u, v) = (y as f32 / 255.0, u as f32 / 255.0, v as f32 / 255.0);
    let (y, cb, cr) = if c.limited {
        ((y - 16.0 / 255.0) * 255.0 / 219.0, (u - 128.0 / 255.0) * 255.0 / 224.0, (v - 128.0 / 255.0) * 255.0 / 224.0)
    } else {
        (y, u - 128.0 / 255.0, v - 128.0 / 255.0)
    };
    let (r, g, b) = if c.bt709 {
        (y + 1.5748 * cr, y - 0.1873 * cb - 0.4681 * cr, y + 1.8556 * cb)
    } else {
        (y + 1.402 * cr, y - 0.344136 * cb - 0.714136 * cr, y + 1.772 * cb)
    };
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [q(b), q(g), q(r)]
}

/// A live frame on the card, under the window's controls (see `Host::live`). Like `PictureLayer`, but the source is a
/// [`Frame`] whose texture is updated in place.
#[derive(Clone, Debug)]
pub struct LiveLayer {
    pub frame: Arc<Frame>,
    /// where it goes, in logical window pixels
    pub dest: [f32; 4],
    /// the part of the frame shown (x, y, w, h in frame pixels); [`LiveLayer::whole`] for all of it
    pub src: [f32; 4],
    pub alpha: f32,
    /// the opaque ground behind it, which fills the whole window
    pub ground: [f32; 3],
    /// flip left and right, as a selfie preview does
    pub mirror: bool,
}

impl LiveLayer {
    /// The whole frame, fitted inside `area` (x, y, w, h in logical pixels) without distortion and centred; the rest of
    /// the window shows `ground`.
    pub fn fitted(frame: Arc<Frame>, area: [f32; 4], ground: [f32; 3]) -> LiveLayer {
        let dest = fit(frame.w as f32, frame.h as f32, area);
        LiveLayer { src: [0.0, 0.0, frame.w as f32, frame.h as f32], frame, dest, alpha: 1.0, ground, mirror: false }
    }

    /// The same layer showing the frame as a mirror would.
    pub fn mirrored(mut self, on: bool) -> LiveLayer {
        self.mirror = on;
        self
    }
}

/// The largest box with the aspect of `w` x `h` that fits inside `area`, centred in it.
pub fn fit(w: f32, h: f32, area: [f32; 4]) -> [f32; 4] {
    if w <= 0.0 || h <= 0.0 || area[2] <= 0.0 || area[3] <= 0.0 {
        return [area[0], area[1], 0.0, 0.0];
    }
    let k = (area[2] / w).min(area[3] / h);
    let (dw, dh) = (w * k, h * k);
    [area[0] + (area[2] - dw) / 2.0, area[1] + (area[3] - dh) / 2.0, dw, dh]
}

/// Hands the newest frame from a capture thread to the window. A frame put in while another is waiting replaces it.
#[derive(Clone, Default)]
pub struct Mailbox {
    slot: Arc<Mutex<Option<Arc<Frame>>>>,
    dropped: Arc<AtomicU64>,
}

impl Mailbox {
    pub fn new() -> Mailbox {
        Mailbox::default()
    }

    /// Leave a frame for the window; one that was never taken is dropped (and counted).
    pub fn put(&self, frame: Frame) {
        let old = self.slot.lock().unwrap_or_else(|e| e.into_inner()).replace(Arc::new(frame));
        if old.is_some() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The frame that arrived since the last call, if any.
    pub fn take(&self) -> Option<Arc<Frame>> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// How many frames were replaced before the window took them (a measure of how far behind the window runs).
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(p: &Pic, x: usize, y: usize) -> [u8; 3] {
        let (d, w, _) = p.pixels();
        let i = (y * w + x) * 4;
        [d[i + 2], d[i + 1], d[i]]
    }

    #[test]
    fn lengths_follow_the_format() {
        assert_eq!(frame_len(PixelFormat::Bgra, 4, 2), Some(32));
        assert_eq!(frame_len(PixelFormat::Nv12, 4, 2), Some(12));
        assert_eq!(frame_len(PixelFormat::Yuyv, 4, 2), Some(16));
        assert_eq!(frame_len(PixelFormat::Nv12, 3, 2), None);
        assert_eq!(frame_len(PixelFormat::Yuyv, 3, 2), None);
        assert!(Frame::new(4, 2, PixelFormat::Yuyv, Color::default(), vec![0; 15]).is_none());
    }

    #[test]
    fn limited_range_black_and_white() {
        // Y 16 / 235 with neutral chroma are black and white in video range
        let f = Frame::new(2, 2, PixelFormat::Nv12, Color::default(), vec![16, 235, 16, 235, 128, 128]).unwrap();
        let p = f.to_pic();
        assert_eq!(px(&p, 0, 0), [0, 0, 0]);
        assert_eq!(px(&p, 1, 0), [255, 255, 255]);
    }

    #[test]
    fn full_range_passes_luma_through() {
        let c = Color { limited: false, bt709: false };
        let f = Frame::new(2, 1, PixelFormat::Yuyv, c, vec![0, 128, 255, 128]).unwrap();
        let p = f.to_pic();
        assert_eq!(px(&p, 0, 0), [0, 0, 0]);
        assert_eq!(px(&p, 1, 0), [255, 255, 255]);
    }

    #[test]
    fn yuyv_pixels_share_chroma_but_not_luma() {
        // red-ish chroma (V high, U low): the brighter pixel is the brighter red
        let c = Color { limited: false, bt709: false };
        let f = Frame::new(2, 1, PixelFormat::Yuyv, c, vec![60, 90, 120, 240]).unwrap();
        let p = f.to_pic();
        let (a, b) = (px(&p, 0, 0), px(&p, 1, 0));
        assert!(a[0] > a[1] && a[0] > a[2], "{a:?}");
        assert!(b[0] > a[0], "{a:?} {b:?}");
    }

    #[test]
    fn bt709_differs_from_bt601_in_green() {
        let f = |bt709| Frame::new(2, 2, PixelFormat::Nv12, Color { limited: false, bt709 }, vec![128; 4].into_iter().chain([90, 200]).collect()).unwrap().to_pic();
        assert_ne!(px(&f(false), 0, 0), px(&f(true), 0, 0));
    }

    #[test]
    fn mailbox_keeps_only_the_newest() {
        let m = Mailbox::new();
        let mk = || Frame::new(2, 2, PixelFormat::Bgra, Color::default(), vec![0; 16]).unwrap();
        m.put(mk());
        let second = mk();
        let id = second.id;
        m.put(second);
        assert_eq!(m.dropped(), 1);
        assert_eq!(m.take().map(|f| f.id), Some(id));
        assert!(m.take().is_none());
    }

    #[test]
    fn fit_letterboxes_and_centres() {
        let r = fit(1920.0, 1080.0, [0.0, 0.0, 800.0, 800.0]);
        assert_eq!(r[2], 800.0);
        assert!((r[3] - 450.0).abs() < 0.01 && (r[1] - 175.0).abs() < 0.01);
        let r = fit(1000.0, 2000.0, [10.0, 10.0, 400.0, 400.0]);
        assert!((r[2] - 200.0).abs() < 0.01 && (r[0] - 110.0).abs() < 0.01);
    }
}
