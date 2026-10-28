# Chebrium

A small UI/UX toolkit for building pretty, animated and **very light** programs. It is the library behind every
Memrium program: the settings window of the desktop, the calculator, and (next) the terminal, the paint editor and the
file manager.

Chebrium draws with a tiny anti-aliased **software** painter into a shared-memory buffer. There is no GPU context, no
widget tree and no retained state beyond what a program keeps itself, and a program that is not moving draws nothing.

See [docs/DEVELOPING.md](docs/DEVELOPING.md) for the working reference (API cheat sheet, input model, testing, build recipes).

## Principles

- **Light by construction.** A window is one `wl_shm` buffer. Text lines and icons are rasterised once and cached.
  Frames are produced only after input and while something is animating, then the program sleeps.
- **Alive.** Motion is spring and exponential easing, not canned transitions: switches stretch as the knob travels,
  selection highlights glide and squeeze, hovers fade in.
- **One widget code, two painters.** Widgets are written against the `Draw` trait. In a program they paint with the
  software canvas; inside the Memrium desktop the same widgets paint with its GPU glass renderer.
- **Plain code layout.** Immediate mode: a frame is a function that calls widgets and reads their results. There is no
  layout engine to learn and no callbacks to wire up.

## A program in thirty lines

```rust
use chebrium::app::{self, App, Config, Host};
use chebrium::draw::Align;
use chebrium::ui::Ui;

struct Counter {
    n: i32,
}

impl App for Counter {
    fn draw(&mut self, ui: &mut Ui<'_>, size: [f32; 2], host: &mut Host) {
        ui.text(size[0] / 2.0, 80.0, &self.n.to_string(), 48.0, true, ui.ink, 1.0, Align::Center);
        if ui.button([20.0, 120.0, 100.0, 32.0], "−", false) {
            self.n -= 1;
        }
        if ui.button([140.0, 120.0, 100.0, 32.0], "+", true) {
            self.n += 1;
        }
        if ui.button([20.0, 170.0, 220.0, 32.0], "Quit", false) {
            host.quit = true;
        }
    }
}

fn main() {
    app::run(Config::new("Counter", "counter").size(260, 230), Counter { n: 0 });
}
```

Four complete programs live in the desktop crate:

- `Memrium Calc` (`src/bin/memrium_calc/`): a calculator whose logic is unit-tested and whose keypad animates hover and press feedback.
- `Memrium Files` (`src/bin/memrium_files/`): a file manager with a list that scrolls smoothly, inline rename, sorting and keyboard control.
- `Memrium View` (`src/bin/memrium_view/`): an image viewer with background decoding, glide-to-pointer zoom, panning and rotation.
- `Memrium Player` (`src/bin/memrium_player/`): a media player; libmpv decodes into a `Pic`, Chebrium draws everything else.

## Modules

| module | what it is |
|---|---|
| `app` | the window runtime (Wayland, shared memory, frame callbacks, pointer, keyboard with repeat, HiDPI). Feature `app`, on by default |
| `ui` | `State` (pointer, double click, wheel, modifiers, drag, text editing, animations) and `Ui`, the immediate-mode widgets |
| `form` | `Form`, a column of label + control rows that lines pages up without pixel arithmetic |
| `draw` | the `Draw` trait: fill, gradient, icon, text, measure, image |
| `record` | `Recorder` (a `Draw` that stores commands) and `Frame` (the last frame's pixels): a frame equal to the last paints nothing, a change repaints only the box it touches, pixel for pixel the same as painting everything |
| `picture` | `Pic`: a decoded image or a video frame, drawn scaled and cropped with `Draw::picture` (bilinear, with mips; plain row copies at 1:1) |
| `soft` | `SoftDraw`, the software `Draw` over a `Canvas`, and `Gfx`, the font and icon caches |
| `canvas` | the painter: rounded rectangles, gradients, strokes, lines, bitmaps, glyphs, on a premultiplied ARGB buffer |
| `spring`, `anim` | a damped spring, and keyed exponential easing that reports whether anything still moves |
| `lucide`, `text` | the bundled Lucide icon set and the Inter typeface (with `CHEBRIUM_FONT` to pick another family) |
| `theme` | `Appearance`: the look of the desktop (light or dark, accent, window shape, glass, language) read from the file the shell writes, and followed live |
| `i18n` | `tr` and `trf`: every text a widget shows goes through the catalogue (English is the key); `Ui::text_raw` shows data (names, titles, numbers) as it is |
| `chrome` | a window header in the desktop's style (macOS lights, GNOME circles, Fluent flat) for programs that draw their own, plus resize borders |
| `input`, `style` | the keys the widgets understand; ink, accent and dark/light |

### Widgets

`button`, `toggle`, `slider`, `gradient_slider`, `segmented`, `text_field`, plus `row` and `section` labels and the
`Form` helpers (`slider`, `toggle`, `pick`, `stepper`, `text`, `note`). Drawing primitives are available directly:
`fill`, `grad`, `glyph`, `text`, `measure`.

## How a frame works

```
input events ─▶ State (pointer, click, drag, text edit)
                  │
begin_frame() ─▶ dt
                  │
Ui::new(painter, &mut state, style, dt) ─▶ widgets ─▶ painter calls
                  │
end_frame(edit_seen)
                  │
state.animating() ─▶ true: ask for another frame   false: sleep until the next input
```

`app::run` does all of this for you, and since 0.1 it goes through `record`: the widgets draw into a recording, the
frame is compared with the last one, and only what changed is painted and sent. Other threads wake the program with
`Waker::wake()` (a finished decode, a new video frame), `Host::fullscreen` toggles fullscreen and `Host::scale` tells how
big a picture has to be rendered for a HiDPI window. To embed Chebrium in your own loop (or render offscreen, as
`examples/gallery.rs` does) build a `State`, a `Gfx` and a `Canvas` and follow the steps above.

## Programs that handle input themselves

`memrium_term` and `memrium_paint` run on the same runtime. A program that paints its own window (a terminal, an editor)
returns `true` from `App::raw_key` / `App::raw_pointer` (the key with its keysym and text, every button, both wheels),
hands its window over as one translucent picture (`Pic::from_bgra_window`), reads what other threads sent in `App::tick`
through `app::channel()` (a send wakes the program), and asks the runtime for the clipboard through `Host::copy` and
`Host::paste` (the text comes back in `App::clipboard`).

## Following the desktop

A program needs no code to look like the desktop it runs on. The Memrium shell writes its look to
`~/.config/memrium/appearance.toml` whenever it changes, and the runtime reads it at start and again when it changes:
widget ink and accent follow the theme (`App::style`), the window background is the theme's glass (`App::background`:
translucent, so the shell frosts what is behind the window) and the language follows the shell's setting. Both methods can
be overridden when a program wants its own colours (the viewer and the player stay dark). Without the file the defaults apply.

## Window decorations

`Config::decorations(..)` says who draws the header:

- `Decorations::Server` (the default): the shell draws it in the theme's style. Nothing to do.
- `Decorations::Client`: the program draws its own, and the shell still rounds the corners and casts the shadow. Use
  `chrome::titlebar` for a whole header, or `chrome::controls` for just the buttons on top of content (a viewer's overlay),
  `chrome::resize_borders` for resizing from the edges, and `Host::{drag, resize, minimize, toggle_maximize}` for anything
  else (`Host::maximized` tells the state).
- `Decorations::None`: no header at all (a splash screen, a widget).

```rust
app::run(Config::new("Calculator", "calc").decorations(Decorations::Client), calc);
// in draw():  let resizing = chrome::resize_borders(ui, host, size);
//             let top = chrome::titlebar(ui, host, w, "Calculator");   // returns the header's height
```

## Clips and glass

`Ui::push_clip(rect)` / `pop_clip()` limit drawing (nesting works) so a list can scroll under a translucent bar without
opaque covers, and `Ui::glass(rect, radius, strength)` draws a floating panel in the theme's glass look.

## Memory and speed

Measured with a 1600 x 900 desktop, a program running alone (resident memory after ten seconds):

| program | before | now |
|---|---|---|
| `memrium_calc` | 25 MB | 7 MB |
| `memrium_files` | 29 MB | 11 MB |
| `memrium_term` | 212 MB | 17 MB |
| `memrium_paint` | 215 MB | 16 MB |
| `memrium_view` (one 3 MB picture) | 98 MB | 72 MB |
| `memrium_player` (720p video) | 203 MB | 151 MB |
| `memrium_compositor` | 270 MB | 205 MB |

What made the difference: `glyphs` reads a font where it is and rasterises only the glyphs that are drawn (the font crate
used before held every outline of a ten-thousand-glyph Nerd Font, about 100 MB), fonts a program does not use are not
loaded, the allocator is limited to two arenas and `mem::trim` gives freed memory back every few seconds, and the player
caps mpv's demuxer cache. Unchanged frames cost nothing (`record`), small changes repaint a small box.

`CHEBRIUM_PERF=1` prints once a second where the frames of a program went: building, painting, copying, and how many
pixels were repainted. `cargo run -p chebrium --example gallery -- . bench` times the painter on its own.

## Animations

- `Ui::ease(key, target, speed)` glides a value towards a target; keys are plain `u64`s you choose. Values start at
  their target, so a screen that has just appeared is not mid-animation.
- `Spring` is for motion that should overshoot (dock magnification, jelly wobble).
- Both know when they are at rest. `State::animating()` is true while any eased value still moves, which is the only
  thing the runtime needs to decide whether to draw again. If you animate something by hand, set `host.redraw` until it
  settles.

## Painters

`Draw` is the whole contract between widgets and a painter:

```rust
pub trait Draw {
    fn fill(&mut self, r: [f32; 4], radius: f32, c: [f32; 3], alpha: f32);
    fn grad(&mut self, r: [f32; 4], radius: f32, top: [f32; 3], bottom: [f32; 3], alpha: f32);
    fn glyph(&mut self, cx: f32, cy: f32, size: f32, icon: &str, c: [f32; 3], alpha: f32);
    fn text(&mut self, x: f32, y: f32, text: &str, size: f32, bold: bool, c: [f32; 3], alpha: f32, align: Align);
    fn measure(&mut self, text: &str, size: f32, bold: bool) -> f32 { /* rough guess */ }
    fn image(&mut self, r: [f32; 4], radius: f32, handle: u32) { /* optional */ }
}
```

Rectangles are `[x, y, w, h]` in logical pixels, colours are straight RGB in 0..1, and a text `y` is the baseline.
`SoftDraw` implements it for a `Canvas` (with a HiDPI `scaled` factor); the desktop implements it with
`renderer::PanelDraw` to turn widgets into glass panels and labels.

## Icons and fonts

Icons are [Lucide](https://lucide.dev) (ISC licence), embedded as SVG and rasterised with `resvg`, tinted by
substituting `currentColor`. To add one, put its SVG in `assets/lucide/` and list it in `src/lucide.rs`. Text is Inter
(Regular and SemiBold) via `fontdue`; set `CHEBRIUM_FONT=Family` to use another font through fontconfig.

## Building and trying it

```
nix-shell --run 'cargo build --release -p chebrium'
nix-shell --run 'cargo run --release -p chebrium --example gallery -- out_dir'   # gallery-dark.png, gallery-light.png
nix-shell --run 'cargo run --release --bin memrium_calc'                          # needs a Wayland session
```

## Status and roadmap

Working: the widget kit, the software painter, the window runtime (wheel, double click, modifiers, Ctrl shortcuts),
Settings in the desktop built on the same widgets, the calculator and the file manager.

Not yet: a reusable scrolling list widget (the file manager scrolls by hand), menus and popups as separate surfaces,
a text cursor with selection and accessibility.

## Why "Chebrium"

*Chebrets* (чебрець) is thyme, a small, fragrant steppe herb. The `-ium` ending is the library part of the name.

## Menus in the desktop's menu bar

A program describes its menus with `App::menus` (a `Vec<appmenu::Menu>` of `Item`s: checkmarks, shortcuts, separators,
submenus) and gets a choice in `App::menu_action(id, host)`. The runtime sends the menus to the shell's
`io.github.supchik22.Memrium.AppMenu` service on the session bus (`Set(app_id, rows)`; the shell answers with the
`Activated(app_id, id)` signal) whenever they change. `Host::menu_bar` tells the program that the shell is showing them,
so it can leave its own pickers out of the window. Without such a shell nothing is sent. The bus name, path and interface
are the constants `appmenu::{BUS, PATH, IFACE}`.

Programs that know nothing of this protocol still get their menus in the bar when they export a `com.canonical.dbusmenu`:
Qt/KDE programs under Wayland say where it is with `org_kde_kwin_appmenu`, and X11 programs register with the Unity-style
`com.canonical.AppMenu.Registrar`; the shell serves both.
