//! Live frames on the card with a synthetic source: a moving test pattern in BGRA, NV12 or YUYV, made on a thread the way
//! a camera would deliver it. `1` `2` `3` pick the format, `m` mirrors, `p` shows the processor path (`Frame::to_pic`).
//!   cargo run -p chebrium --release --example live_frames
//! `LIVE_FORMAT=bgra|nv12|yuyv` and `LIVE_CPU=1` set the start state (for headless screenshots).

use chebrium::app::{self, App, Config, Decorations, Host, Waker};
use chebrium::draw::Align;
use chebrium::input::{Key, KeyMods};
use chebrium::live::{Color, Frame, LiveLayer, Mailbox, PixelFormat};
use chebrium::ui::Ui;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const W: u32 = 1280;
const H: u32 = 720;

/// Colour bars with a white square sliding over them: the bars show chroma, the square shows motion.
fn rgb_at(x: u32, y: u32, t: f32) -> [f32; 3] {
    let bars = [[1.0, 1.0, 1.0], [1.0, 1.0, 0.0], [0.0, 1.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.1, 0.1, 0.1]];
    let cx = ((t * 240.0) as u32) % (W - 120);
    if x >= cx && x < cx + 120 && (H / 2 - 60..H / 2 + 60).contains(&y) {
        return [1.0, 1.0, 1.0];
    }
    if y > H - 80 {
        let g = x as f32 / W as f32;
        return [g, g, g];
    }
    bars[(x * 8 / W) as usize]
}

/// Full-range BT.601 YUV of an RGB colour, limited-range scaled when asked (what a camera would send).
fn yuv(c: [f32; 3]) -> [u8; 3] {
    let y = 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    let (cb, cr) = ((c[2] - y) / 1.772, (c[0] - y) / 1.402);
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let (y, cb, cr) = (16.0 / 255.0 + y * 219.0 / 255.0, 0.5 + cb * 224.0 / 255.0, 0.5 + cr * 224.0 / 255.0);
    [q(y), q(cb), q(cr)]
}

fn make(format: PixelFormat, t: f32) -> Frame {
    let (w, h) = (W as usize, H as usize);
    let data = match format {
        PixelFormat::Bgra => {
            let mut d = Vec::with_capacity(w * h * 4);
            for y in 0..H {
                for x in 0..W {
                    let c = rgb_at(x, y, t);
                    d.extend_from_slice(&[(c[2] * 255.0) as u8, (c[1] * 255.0) as u8, (c[0] * 255.0) as u8, 255]);
                }
            }
            d
        }
        PixelFormat::Nv12 => {
            let mut d = vec![0u8; w * h * 3 / 2];
            for y in 0..H {
                for x in 0..W {
                    let p = yuv(rgb_at(x, y, t));
                    d[(y * W + x) as usize] = p[0];
                    if x % 2 == 0 && y % 2 == 0 {
                        let i = w * h + ((y / 2) * W + x) as usize;
                        d[i] = p[1];
                        d[i + 1] = p[2];
                    }
                }
            }
            d
        }
        PixelFormat::Yuyv => {
            let mut d = vec![0u8; w * h * 2];
            for y in 0..H {
                for x in (0..W).step_by(2) {
                    let (a, b) = (yuv(rgb_at(x, y, t)), yuv(rgb_at(x + 1, y, t)));
                    let i = ((y * W + x) * 2) as usize;
                    d[i..i + 4].copy_from_slice(&[a[0], a[1], b[0], a[2]]);
                }
            }
            d
        }
    };
    Frame::new(W, H, format, Color::default(), data).expect("frame size")
}

fn code(f: PixelFormat) -> u8 {
    match f {
        PixelFormat::Bgra => 0,
        PixelFormat::Nv12 => 1,
        PixelFormat::Yuyv => 2,
    }
}

fn format_of(c: u8) -> PixelFormat {
    [PixelFormat::Bgra, PixelFormat::Nv12, PixelFormat::Yuyv][c as usize % 3]
}

struct Demo {
    mail: Mailbox,
    format: Arc<AtomicU8>,
    frame: Option<Arc<Frame>>,
    mirror: bool,
    cpu: bool,
    shown: u32,
    since: Instant,
    fps: f32,
}

impl App for Demo {
    fn start(&mut self, waker: Waker) {
        let (mail, format) = (self.mail.clone(), self.format.clone());
        std::thread::spawn(move || {
            // the pattern is made once per format and phase, so the rate measured is the window's, not the generator's
            let mut cache: std::collections::HashMap<(u8, u32), Vec<u8>> = std::collections::HashMap::new();
            let mut n = 0u32;
            loop {
                let (code, phase) = (format.load(Ordering::Relaxed), n % 60);
                let f = format_of(code);
                let data = cache.entry((code, phase)).or_insert_with(|| make(f, phase as f32 / 30.0).data).clone();
                if let Some(frame) = Frame::new(W, H, f, Color::default(), data) {
                    mail.put(frame);
                    waker.wake();
                }
                n += 1;
                std::thread::sleep(Duration::from_millis(16));
            }
        });
    }

    fn key(&mut self, key: Key, _: KeyMods, host: &mut Host) {
        match key {
            Key::Char('1') => self.format.store(0, Ordering::Relaxed),
            Key::Char('2') => self.format.store(1, Ordering::Relaxed),
            Key::Char('3') => self.format.store(2, Ordering::Relaxed),
            Key::Char('m') => self.mirror = !self.mirror,
            Key::Char('p') => self.cpu = !self.cpu,
            Key::Esc => host.quit = true,
            _ => {}
        }
    }

    fn draw(&mut self, ui: &mut Ui<'_>, size: [f32; 2], host: &mut Host) {
        if let Some(f) = self.mail.take() {
            self.frame = Some(f);
            self.shown += 1;
        }
        if self.since.elapsed() >= Duration::from_secs(1) {
            self.fps = self.shown as f32 / self.since.elapsed().as_secs_f32();
            eprintln!("live_frames: {:.1} fps shown, {} dropped", self.fps, self.mail.dropped());
            (self.shown, self.since) = (0, Instant::now());
        }
        let ground = [0.06, 0.06, 0.07];
        let mut label = String::from("waiting for a frame");
        if let Some(f) = &self.frame {
            let layer = LiveLayer::fitted(f.clone(), [0.0, 0.0, size[0], size[1]], ground).mirrored(self.mirror);
            let on_card = host.gpu_layer && !self.cpu;
            label = format!("{:?}  {}x{}  {:.0} fps  {}  dropped {}", f.format, f.w, f.h, self.fps, if on_card { "card" } else { "processor" }, self.mail.dropped());
            if on_card {
                host.live = Some(layer);
            } else {
                // the processor path cannot mirror; it converts and blits the frame as it is
                ui.picture(layer.dest, &Arc::new(f.to_pic()), layer.src, 1.0);
            }
            host.redraw = true;
        }
        ui.text(12.0, 12.0, &label, 13.0, true, [1.0, 1.0, 1.0], 0.9, Align::Left);
    }
}

fn main() {
    let start = match std::env::var("LIVE_FORMAT").as_deref() {
        Ok("nv12") => PixelFormat::Nv12,
        Ok("yuyv") => PixelFormat::Yuyv,
        _ => PixelFormat::Bgra,
    };
    let demo = Demo {
        mail: Mailbox::new(),
        format: Arc::new(AtomicU8::new(code(start))),
        frame: None,
        mirror: false,
        cpu: std::env::var_os("LIVE_CPU").is_some(),
        shown: 0,
        since: Instant::now(),
        fps: 0.0,
    };
    app::run(Config::new("Live frames", "chebrium-live-frames").size(960, 540).min_size(320, 200).decorations(Decorations::Client), demo);
}
