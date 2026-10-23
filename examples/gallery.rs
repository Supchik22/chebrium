//! Renders the widget kit to PNG files with the software painter (no window, no GPU):
//!
//!     cargo run -p chebrium --example gallery -- out_dir
//!
//! Writes `gallery-dark.png` and `gallery-light.png`, plus a few frames of an animating switch.

use chebrium::canvas::Canvas;
use chebrium::draw::Align;
use chebrium::form::Form;
use chebrium::soft::{Gfx, SoftDraw};
use chebrium::style::Style;
use chebrium::ui::{State, Ui};

const W: usize = 520;
const H: usize = 430;

struct Page {
    on: bool,
    quiet: bool,
    volume: f32,
    bright: f32,
    mode: usize,
    hue: f32,
    name: String,
}

fn draw(ui: &mut Ui<'_>, p: &mut Page) {
    ui.text(28.0, 44.0, "Chebrium", 20.0, true, ui.ink, 1.0, Align::Left);
    ui.text(28.0, 64.0, "Animated, tiny, software-drawn", 12.5, false, ui.ink, 0.55, Align::Left);
    ui.glyph(W as f32 - 40.0, 40.0, 22.0, "sun", ui.accent, 1.0);
    ui.section(28.0, 100.0, "CONTROLS");
    let mut f = Form::new(28.0, 130.0, W as f32 - 56.0, 40.0);
    f.toggle(ui, "Wi-Fi", &mut p.on);
    f.toggle(ui, "Do not disturb", &mut p.quiet);
    f.slider(ui, "Volume", &mut p.volume, 0.0, 100.0, " %");
    f.slider(ui, "Brightness", &mut p.bright, 0.0, 1.0, "");
    f.pick(ui, "Mode", &mut p.mode, &[(0, "Auto"), (1, "Light"), (2, "Dark")]);
    f.text(ui, "Name", "name", &mut p.name, Some("Enter to apply"));
    ui.row(f.x, f.y, "Accent hue");
    ui.gradient_slider([f.x + 150.0, f.y - 7.0, f.cw - 150.0, 14.0], &mut p.hue, &|t| {
        let h = t * 6.0;
        let x = 1.0 - (h % 2.0 - 1.0).abs();
        match h as i32 {
            0 => [1.0, x, 0.0],
            1 => [x, 1.0, 0.0],
            2 => [0.0, 1.0, x],
            3 => [0.0, x, 1.0],
            4 => [x, 0.0, 1.0],
            _ => [1.0, 0.0, x],
        }
    });
    let by = f.y + 24.0;
    ui.button([28.0, by, 110.0, 32.0], "Cancel", false);
    ui.button([148.0, by, 110.0, 32.0], "Apply", true);
}

fn render(dark: bool, out: &str, gfx: &mut Gfx) {
    let style = Style::new(dark, [0.23, 0.55, 1.0]);
    let mut buf = vec![0u8; W * H * 4];
    let mut canvas = Canvas::new(&mut buf, W, H);
    canvas.clear(if dark { [0x18, 0x1b, 0x26] } else { [0xf4, 0xf6, 0xfb] }, 1.0);
    let mut state = State::default();
    let mut page = Page { on: true, quiet: false, volume: 64.0, bright: 0.35, mode: 1, hue: 0.6, name: "Memrium".into() };
    state.pointer_move([300.0, 350.0]);
    let mut painter = SoftDraw::new(&mut canvas, gfx);
    let dt = state.begin_frame();
    let mut ui = Ui::new(&mut painter, &mut state, style, dt);
    draw(&mut ui, &mut page);
    let seen = ui.edit_seen;
    state.end_frame(seen);
    // BGRA premultiplied -> RGBA for the PNG
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    image::save_buffer(out, &buf, W as u32, H as u32, image::ColorType::Rgba8).unwrap();
}

/// `gallery <dir> bench`: how long a frame of a window-sized program takes to paint (big bars and a list).
fn bench() {
    use chebrium::draw::Draw;
    let (w, h) = (960usize, 620usize);
    let mut gfx = Gfx::new();
    let mut buf = vec![0u8; w * h * 4];
    let frames = 200;
    let t = std::time::Instant::now();
    for _ in 0..frames {
        let mut canvas = Canvas::new(&mut buf, w, h);
        canvas.clear([0x1a, 0x1c, 0x26], 1.0);
        let mut d = SoftDraw::new(&mut canvas, &mut gfx);
        d.fill([0.0, 0.0, 200.0, h as f32], 0.0, [0.07, 0.08, 0.11], 1.0);
        d.fill([200.0, 0.0, 760.0, 52.0], 0.0, [0.1, 0.11, 0.15], 1.0);
        d.fill([200.0, 592.0, 760.0, 28.0], 0.0, [0.1, 0.11, 0.15], 1.0);
        for i in 0..16 {
            let y = 80.0 + i as f32 * 32.0;
            d.fill([208.0, y, 744.0, 30.0], 8.0, [1.0, 1.0, 1.0], 0.06);
            d.glyph(230.0, y + 15.0, 18.0, "folder", [0.3, 0.6, 1.0], 1.0);
            d.text(252.0, y + 20.0, "Some file name.txt", 13.5, false, [0.95; 3], 0.95, Align::Left);
            d.text(940.0, y + 20.0, "12.5 KiB", 12.0, false, [0.95; 3], 0.55, Align::Right);
        }
    }
    println!("{:.2} ms per {w}x{h} frame", t.elapsed().as_secs_f64() * 1000.0 / frames as f64);
    // the retained path the windows use: record, compare with the last frame, repaint only what changed
    {
        use chebrium::record::{Frame, Recorder};
        let scene = |hot: usize| {
            move |d: &mut Recorder| {
                d.fill([0.0, 0.0, 200.0, h as f32], 0.0, [0.07, 0.08, 0.11], 1.0);
                d.fill([200.0, 0.0, 760.0, 52.0], 0.0, [0.1, 0.11, 0.15], 1.0);
                d.fill([200.0, 592.0, 760.0, 28.0], 0.0, [0.1, 0.11, 0.15], 1.0);
                for i in 0..16 {
                    let y = 80.0 + i as f32 * 32.0;
                    d.fill([208.0, y, 744.0, 30.0], 8.0, [1.0, 1.0, 1.0], if i == hot { 0.14 } else { 0.06 });
                    d.glyph(230.0, y + 15.0, 18.0, "folder", [0.3, 0.6, 1.0], 1.0);
                    d.text(252.0, y + 20.0, "Some file name.txt", 13.5, false, [0.95; 3], 0.95, Align::Left);
                    d.text(940.0, y + 20.0, "12.5 KiB", 12.0, false, [0.95; 3], 0.55, Align::Right);
                }
            }
        };
        let mut frame = Frame::default();
        frame.resize(w, h);
        let bg = ([0.1, 0.11, 0.15], 1.0);
        let mut run = |name: &str, pick: &dyn Fn(usize) -> usize| {
            let t = std::time::Instant::now();
            let mut painted = 0;
            for n in 0..frames {
                let mut rec = Recorder::new(&mut gfx, 1.0);
                scene(pick(n))(&mut rec);
                if frame.update(rec.cmds, &mut gfx, bg, 1.0).is_some() {
                    painted += 1;
                }
            }
            println!("  {name}: {:.3} ms per frame ({painted} of {frames} painted)", t.elapsed().as_secs_f64() * 1000.0 / frames as f64);
        };
        run("pointer moves down the list (a new row lit each frame)", &|n| n % 16);
        run("pointer moves but nothing changes", &|_| 3);
    }
    // where does it go?
    let mut time = |name: &str, f: &mut dyn FnMut(&mut SoftDraw)| {
        let t = std::time::Instant::now();
        for _ in 0..frames {
            let mut canvas = Canvas::new(&mut buf, w, h);
            let mut d = SoftDraw::new(&mut canvas, &mut gfx);
            f(&mut d);
        }
        println!("  {name}: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / frames as f64);
    };
    time("clear", &mut |d| d.canvas.clear([0x1a, 0x1c, 0x26], 1.0));
    time("3 opaque bars", &mut |d| {
        d.fill([0.0, 0.0, 200.0, 620.0], 0.0, [0.07, 0.08, 0.11], 1.0);
        d.fill([200.0, 0.0, 760.0, 52.0], 0.0, [0.1, 0.11, 0.15], 1.0);
        d.fill([200.0, 592.0, 760.0, 28.0], 0.0, [0.1, 0.11, 0.15], 1.0);
    });
    time("16 translucent rows", &mut |d| {
        for i in 0..16 {
            d.fill([208.0, 80.0 + i as f32 * 32.0, 744.0, 30.0], 8.0, [1.0, 1.0, 1.0], 0.06);
        }
    });
    time("16 icons", &mut |d| {
        for i in 0..16 {
            d.glyph(230.0, 95.0 + i as f32 * 32.0, 18.0, "folder", [0.3, 0.6, 1.0], 1.0);
        }
    });
    time("32 texts", &mut |d| {
        for i in 0..16 {
            d.text(252.0, 100.0 + i as f32 * 32.0, "Some file name.txt", 13.5, false, [0.95; 3], 0.95, Align::Left);
            d.text(940.0, 100.0 + i as f32 * 32.0, "12.5 KiB", 12.0, false, [0.95; 3], 0.55, Align::Right);
        }
    });
}

fn main() {
    if std::env::args().nth(2).as_deref() == Some("bench") {
        return bench();
    }
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let mut gfx = Gfx::new();
    render(true, &format!("{dir}/gallery-dark.png"), &mut gfx);
    render(false, &format!("{dir}/gallery-light.png"), &mut gfx);
    println!("wrote {dir}/gallery-{{dark,light}}.png");
}
