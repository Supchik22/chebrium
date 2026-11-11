# Developing a Chebrium app: working reference

A cheat sheet for writing and testing programs on `chebrium` (the README says *why*, this says *how*). Everything here
was read from the sources (`chebrium/src/*.rs`) and from the biggest app, `src/bin/memrium_icons/main.rs`. For the
look of a window see the skill `.claude/skills/chebrium-app-design`.

## 1. Mental model

- **Immediate mode.** `App::draw(&mut self, ui, size, host)` runs once per frame and calls widgets; there is no widget
  tree. State lives in the app struct. Rectangles are `[x, y, w, h]` in **logical** pixels, colours `[f32; 3]` straight
  RGB 0..1, alpha is a separate argument.
- A frame is produced only after input or while something animates (`State::animating()`); `host.redraw = true`
  forces one more. Idle programs draw nothing.
- Input is level + edge: `ui.mouse`, `ui.held` (button down), `ui.click` (the frame the press began), `ui.double`,
  `ui.scroll` (wheel, + is down), `ui.mods` (`ctrl/alt/shift`). There is **no release event**: a drag ends when
  `ui.held` turns false. Only the left button reaches `Ui`; other buttons come through `App::raw_pointer`.
- Hover = `ui.hit(rect)`. Clicking = `ui.hit(r) && ui.click`. Ask for a cursor with `ui.want_cursor(Cursor::..)`
  (`Default Text Pointer Grab Grabbing Move ResizeH ResizeV ResizeNwse ResizeNesw NotAllowed Wait Crosshair`).
- To make something under a popup deaf, overwrite `ui.mouse = [-1000.0; 2]; ui.click = false; ui.scroll = 0.0` for the
  calls under it and restore afterwards (the composer's `block` closure).

## 2. Skeleton

```rust
use chebrium::app::{self, App, Config, Decorations, Host, KeyEvent, Keysym, Modifiers};
use chebrium::draw::Align;
use chebrium::ui::{inside, Cursor, Ui};

struct My { /* state */ }
impl App for My {
    fn draw(&mut self, ui: &mut Ui<'_>, size: [f32; 2], host: &mut Host) { /* widgets */ }
    // optional: start(waker), tick(host), raw_key, raw_pointer, key, clipboard, dropped,
    //           menus()/menu_action(), style(look), background(look)
}
fn main() { app::run(Config::new("Title", "app_id").size(1100, 700).decorations(Decorations::Client), My {}); }
```

`Config`: `.size(w,h)`, `.decorations(Server|Client|None)`, `.resizable(bool)`, fields `min_size`, `app_id`.

`Host` (flags you set, runtime reads after the frame): `quit`, `title`, `redraw`, `fullscreen`, `set_size`,
`set_min_size`, `minimize`, `toggle_maximize`, `drag`, `resize`, `copy: Option<String>`, `copy_image`, `paste`,
`cursor`. Values the runtime sets: `scale` (physical px per logical px: render your own pictures at `n * scale`),
`appearance`, `maximized`, `menu_bar` (the shell shows `App::menus`, so draw your own menu strip only if false),
`activated`, `fixed`.

Keyboard: `raw_key(&KeyEvent{keysym, utf8, raw_code, time}, &Modifiers{ctrl,alt,shift,..}, serial, host) -> bool`
(true = consumed). Match `Keysym::Left/Escape/Return/BackSpace/Delete/...`; letters via `event.keysym.key_char()`.
For shortcuts independent of the layout match `raw_code` (Z=44, Y=21, C=46, V=47, A=30, X=45; evdev codes) so Ctrl+Z
works on a Ukrainian layout. `App::key(Key, KeyMods, host)` is the simplified path (`Key::Char/Up/Down/Enter/Esc/
Delete/F2...`), called only when no text field used the key. Clipboard: set `host.copy`, or `host.paste = true` and
read the text in `App::clipboard`.

Other threads: `app::channel()` / `Inbox`, `Waker::wake()` (keep the waker from `App::start`), read in `App::tick`.

## 3. Painting API (`Ui` derefs the `Draw` trait)

| call | notes |
|---|---|
| `ui.fill(r, radius, rgb, alpha)` | rounded rectangle; `radius` ≥ h/2 gives a pill/circle |
| `ui.grad(r, radius, top, bottom, alpha)` | vertical gradient |
| `ui.glyph(cx, cy, size, "lucide-name", rgb, alpha)` | Lucide icon, **centre** coordinates; name must be in the atlas |
| `ui.text(x, baseline_y, s, size, bold, rgb, alpha, Align::{Left,Center,Right})` | goes through i18n `tr`; `text_raw` shows data untranslated |
| `ui.measure(s, size, bold)` / `measure_raw` | width |
| `ui.push_clip(r)` / `pop_clip()` | nest freely; for scroll areas |
| `ui.picture(rect, &Arc<Pic>, src_rect, alpha)` | bitmap, bilinear + mips. `Pic::from_rgba(w,h,&straight_rgba)` |
| `ui.glass(r, radius, strength)` | theme glass panel |

There is **no line / path / polygon primitive**. For vector overlays rasterise yourself with `resvg::tiny_skia`
(`resvg` is a dependency of the desktop crate: `resvg::{tiny_skia, usvg}`) into a `Pixmap`, convert and upload as a
`Pic` (cache it; rebuild only when something changed). `svgedit::rasterize(svg, px)` turns SVG text into an
`RgbaImage` (fitted and centred, straight alpha).

Icons: names live in `chebrium/src/lucide.rs` (`ICONS`, atlas max 256). Add one = drop the SVG into
`chebrium/assets/lucide/` + list it; the full Lucide set is in `chebrium/assets/lucide-all/` to copy from.
Check with `grep '"name"' chebrium/src/lucide.rs`.

## 4. Widgets

`ui.button(r, label, primary) -> bool`, `icon_button(r, glyph, label, primary) -> bool`, `toggle(x, y, &mut bool)`,
`slider(r, &mut f32, min, max, unit)`, `gradient_slider(r, &mut f32 /*0..1*/, &|t| rgb)`,
`segmented(r, &[labels], &mut usize) -> changed`, `text_field(r, key, &mut String) -> changed`,
`password_field`, `begin_edit(key, text)` (start editing with everything selected, e.g. F2). Fields have a caret, selection (Shift+arrows, Ctrl+A), Home/End/Delete, Ctrl+Backspace for a word and Ctrl+C/X/V through the runtime (`State::clip_out`, `State::want_paste`, `State::insert_text`), `row(x,y,label)`, `section(x,y,label)`,
`tinted(Tone, |ui| ..)`. `ui.changed` is set when a control changed a value this frame.

`Form::new(x, y, content_width, pitch)` stacks rows: `slider(ui,label,&mut v,lo,hi,unit)`, `toggle`, `pick(ui,label,
&mut v,&[(value,"name")])`, `stepper`, `text`, `note`, `skip(dy)`; fields `x y cw`. The composer also has local
helpers (`swatch`, `pick`, `percent`, `section_icon`, `reset_link`) in `src/bin/memrium_icons/main.rs`; `fit` is `chebrium::widgets::fit`.

`ui.slider_bare(r, &mut v, min, max) -> dragging` is the slider without the number after it (true while held, so act
on the release). For a widget of your own: `ui.grab(key, hot) -> held` claims the pointer for a drag (key with high
bits, not a small `id`), `ui.grabbed(key)`, and `ui.recall(key)` reads a value kept with `ui.snap` (where a drag began).

### Shared widgets (`chebrium::widgets`)

Use these instead of drawing your own; they keep the programs alike.

- **Scrolling.** `let s = widgets::scroll_area(ui, rect, content_h, &mut self.scroll);` takes the wheel/touchpad over
  `rect` (used up only when it moved something), clamps `self.scroll` (the target, 0 = top; set it yourself to jump),
  glides the shown offset, handles the bar (thin thumb at the right edge, widens on hover; drag it, or press beside it
  to jump; hidden when everything fits; a press on it is not seen by the content) and **pushes a clip**. Draw the
  content at `s.y(content_y)`, loop `for i in s.rows(row_h, n)` for uniform rows, light rows with `s.hit(ui, r)`,
  then **`s.end(ui)`** (pops the clip, draws the bar on top). `.fence(ui)` after the call makes the widgets inside deaf
  to a pointer outside the viewport (settings pages scrolling under a header). `s.offset` (shown), `s.max`, `s.scrolls()`.
  `widgets::reveal(&mut offset, top, bottom, viewport_h)` keeps a keyboard cursor row in view. Leave `widgets::BAR_ZONE`
  (12 px) at the right edge free of controls or accept that the bar sits over them.
- `widgets::scrollbar(ui, track, content_h, viewport_h, &mut offset) -> dragging` is the bar alone, for views that
  scroll their own way (bottom-anchored chat/history: convert `from_top = max - from_bottom`; a terminal's history).
  `track = widgets::bar_track(viewport)`. Call it before the content so its press is not also the content's.
- `widgets::list_row(ui, r, radius, key, hovered, selected)`: a row's background, accent 0.28 when selected, an eased
  ink wash under the pointer.
- `widgets::round_button(ui, cx, cy, "glyph", Round::ink(30.0).glyph(17.0).enabled(b)) -> pressed`: round icon
  button with an eased wash; `Round::light(d)` is white for over pictures/film, `.alpha(a)` fades it (below 0.3 it
  takes no presses), `.on(b)` puts it on the accent.
- `widgets::seek_bar(ui, r, &mut v /*0..1*/, alpha, fill) -> Seek { hover, dragging, at }`: a seek line or volume over
  a picture/film; the rail and knob grow under the pointer; `dragging` turning false is the release.
- `widgets::fit(ui, text, size, bold, max_w)` / `fit_start(..)`: text cut with an ellipsis at the end / start.
- `widgets::facts_panel(ui, x, y, w, &[("Name", value)])`: glass card of name/value lines (photo or film details).

### Context menus and drops

`chebrium::popmenu::PopMenu::new(at, Vec<appmenu::Item>)` is a right-click menu drawn in the window (submenus on hover,
keys, red `Item::danger()` entries). Open it from `App::raw_pointer` (button 0x111). Each frame: `let held = popmenu::shield(ui)`
before the window's widgets (they go deaf), then `menu.draw(ui, held, size)` last -> `Outcome::{Open, Chosen(id), Closed}`;
while it is open send keys to `PopMenu::key` first. `Host::drag_over` is where files dragged from another program are over
the window (also set while `App::dropped` runs, so a drop can go to the row under it). `ui.edit_text(key)` reads a field's
text while it is typed (a live filter). Reference: `src/bin/memrium_files`.

## 5. Animation

- `ui.ease(key_u64, target, speed)` → current value (speed ~22 for hovers). Keys are yours; hash an id into them.
  `ui.snap(key, v)` jumps. `ui.dt()` = frame seconds.
- `chebrium::spring::Spring::new(x, stiffness, damping_ratio)`, `.update(dt)`, `.at_rest()`, `.kick(impulse)`, `.x`.
  Call `host.redraw = true` while a spring is not at rest.
- Menus/pop-ins use the theme's named effects (`an.step(..)`); see the design skill's Motion section.

## 6. Window chrome (client-side decorations)

In `draw`: `chrome::resize_borders(ui, host, size)` first; paint content; **last**
`chrome::integrated(ui, host, size, bar_h, used)` (window buttons over your content + drag area; `used` = a press hit
your own toolbar widget). `chrome::titlebar(ui, host, w, title) -> height` for a classic header, `chrome::controls`
for just the buttons. `App::background(look)` is the glass (don't paint the window opaque).

## 7. Menus in the shell's bar

`fn menus(&self) -> Vec<Menu>`: `Menu::new("Edit", vec![Item::new(10, "Undo").key("Ctrl+Z").enabled(b).checked(b)
.icon("undo-2"), Item::sep(), Item::parent("Sub", vec![..])])`; handle `menu_action(id, host)`. Called every frame,
sent only on change. Without `host.menu_bar` draw your own strip (see `draw_menu_strip` in memrium_icons).

## 8. Tests without a window (`Rig` pattern)

From `memrium_icons` tests: build `State::default()`, `Gfx::new()`, `Host::default()`, then per frame
```rust
let mut buf = vec![0u8; W*H*4];
let mut canvas = Canvas::new(&mut buf, W, H);
let mut painter = SoftDraw::new(&mut canvas, &mut gfx);
let dt = state.begin_frame();
let mut ui = Ui::new(&mut painter, &mut state, Style::new(true, [0.23,0.55,1.0]), dt);
app.draw(&mut ui, [W as f32, H as f32], &mut host);
let seen = ui.edit_seen; state.end_frame(seen);
```
Input: `state.pointer_move(p)`, `pointer_down(p)`, `pointer_up()`, `pointer_scroll(dy)`; keys through
`app.raw_key(&KeyEvent{time:0, raw_code:0, keysym, utf8}, &Modifiers{ctrl,..Default}, 0, &mut host)`. Run a frame after
each step. Screenshot: the buffer is premultiplied **BGRA**; swap channels 0/2 and save with `image::save_buffer`
(the composer does it when `TMP_SHOT=path` is set). Tests that share on-disk state need a `Mutex` and `XDG_DATA_HOME`
pointing to a temp dir.

## 9. Build / run recipes (NixOS)

- Everything needs the nix shell: `nix-shell --run 'cargo build --release --bin memrium_icons'`.
- Run: `nix-shell --run ./target/release/memrium_icons` (needs `LD_LIBRARY_PATH` from `shell.nix`).
- One test with screenshot: `TMP_SHOT=/path/out.png nix-shell --run 'cargo test --release --bin memrium_icons <name>'`.
- Examples: `cargo run --release --example glass_shot -- <icon> out.png`; chebrium gallery:
  `cargo run -p chebrium --example gallery -- . bench`. `CHEBRIUM_PERF=1` prints frame costs.

## 10. GPU-heavy apps (pictures, video, camera)

The toolkit itself is CPU-only: widgets, text and glass go through `SoftDraw` into a shared-memory buffer, so a plain
app needs no GPU context. Big or fast-changing pixels (photo zoom/pan, video, a live camera preview) should not be
pushed through that path; they go to the GPU through **one** mechanism, the picture layer (`src/layer.rs`, feature `gpu`).

How it works
- The runtime opens a subsurface *below* the window surface and wgpu presents to it (`GpuLayer`). Your toolkit pixels
  stay on top; the window background is made see-through where the layer shows.
- You do not touch wgpu. Each frame set `host.layer = Some(PictureLayer { pic: Arc<Pic>, dest, src, alpha, ground })`
  (`dest` in logical window px, `src` = visible part in picture px, `ground` = opaque colour filling the window).
  Only do it when `host.gpu_layer` is true; otherwise draw with `ui.picture(..)` / `Canvas::blit_pic` on the CPU.
  Setting `host.layer = None` hides the layer. Reference use: `memrium_view` (`main.rs`, search `host.layer`).
- Build a `Pic` with `Pic::from_bgra(w, h, data, opaque, mips)` (premultiplied BGRA) or `Pic::from_rgba`. `Pic::id` is the
  identity: the GPU keeps the last 3 textures (`KEEP`) and re-uploads only when the id is new. `mips = true` for stills
  (trilinear when shown small), `false` for video/camera frames (upload cost matters, they are shown near 1:1).
- `render` skips drawing and presenting when pic id, size, rect, src, alpha and ground are unchanged.
- Zoom is sampled bilinear, switching to nearest above 3x so pixels stay crisp; lod is picked from the on-screen scale.

Device policy and switches
- The adapter is requested with `PowerPreference::LowPower` (the integrated card): the layer only samples a texture, and
  waking the dedicated GPU on a hybrid laptop for that would cost far more power than it saves. Heavy *compute* (see
  `iconrender/gpu.rs` in the shell crate) is a separate decision with its own device choice.
- `CHEBRIUM_NO_GPU=1` forces the CPU path. If device creation fails the runtime logs it, sets `host.gpu_layer = false`
  and the app falls back by itself, so always keep the CPU path working and tested (the `Rig` in section 8 is CPU-only).
- Surface format is non-sRGB on purpose: bytes go through unchanged, matching the CPU path.

Live frames (camera, video) — `chebrium::live`
- A capture thread builds `Frame::new(w, h, PixelFormat::{Bgra|Nv12|Yuyv}, Color::default(), bytes)` (tightly packed; `None`
  if the length does not fit) and `Mailbox::put`s it, then `Waker::wake()` (keep the waker from `App::start`). The mailbox
  holds only the newest frame, so a slow window drops stale ones (`Mailbox::dropped()` counts them) instead of queueing.
- In `draw`: `if let Some(f) = mailbox.take() { self.frame = Some(f) }`, then
  `host.live = Some(LiveLayer::fitted(frame, [0.0, 0.0, w, h], GROUND).mirrored(selfie))` when `host.gpu_layer`.
  `LiveLayer` takes `dest`/`src`/`alpha`/`ground` like `PictureLayer`; `live::fit` letterboxes. Set `host.redraw = true`
  while frames keep coming. `host.live` wins over `host.layer`.
- On the card (`GpuLayer::render_live`): one texture set per format and size, kept across frames and written in place
  (`Bgra8` / NV12 = `R8` + `Rg8` / YUYV = `Rgba8` at half width); YUV -> RGB runs in the shader. A frame already shown, in
  the same place, costs nothing; a new size or format makes new textures. `Color { limited, bt709 }` picks range and matrix
  (webcams: limited, BT.601 = the default; JPEG-style: `limited: false`).
- Without a card (`gpu_layer` false) draw `ui.picture(.., &Arc::new(frame.to_pic()), ..)`; `to_pic` is the same
  conversion on the processor and the reference the shader is checked against. Mirroring is card-only.
- Live textures have no mips: a 1080p frame in a small window is sampled bilinear and can shimmer. YUYV is read with exact
  texel loads (no chroma smoothing). NV12 chroma is sampled bilinear at its half-size grid, no siting correction.
- Example: `cargo run -p chebrium --release --example live_frames` (`1/2/3` format, `m` mirror, `p` CPU path;
  `LIVE_FORMAT`, `LIVE_CPU` for headless). Headless recipe:
  `target/release/memrium_compositor --headless --exec 'env LIVE_FORMAT=nv12 <path>/live_frames' --shot out.png --after-ms 4000`.
  Verified headless 2026-10-29: colour bars correct in all three formats; ~25 fps on the card vs ~17 fps on the processor
  from a 60 fps source (the headless output is probably what caps it). Not checked in a live session.
- Not done: dmabuf import from V4L2/PipeWire (frames are copied in), MJPEG decoding (do it on the capture thread, then hand
  over YUV/BGRA), mips for downscaled frames, GPU-side effects (filters, crop, histogram), chroma siting, frame stride/padding
  (frames must be tightly packed).
- Keep capture and conversion off the UI thread. `CHEBRIUM_PERF=1` prints frame costs. A real-session check on the hybrid
  laptop is still needed before calling a GPU path done (see the compositor notes about PRIME).

Your own drawing on the card — `chebrium::scene` (feature `gpu`)
- For 3D views and anything a picture cannot carry: implement `scene::Scene::render(&mut self, gpu: &Gpu, target, size)`
  (`gpu.device`, `gpu.queue`, `gpu.id`; make pipelines on the first call and again when `gpu.id` changes; `target` is a
  `scene::FORMAT` (Rgba8Unorm, display values, no sRGB) texture of `size` device px, clear it yourself).
- Each frame (only when `host.gpu_layer`): `host.scene = Some(SceneLayer { scene: Arc<Mutex<dyn Scene>>, version, dest,
  ground })`. Bump `version` whenever anything shown changed: same version + same size = nothing is drawn. The runtime
  renders the scene into a texture of `dest`'s size on the picture layer's card, then shows it like a picture (glass
  panels over it work). Priority: `live` > `scene` > `layer`.
- `scene::Offscreen::new(low_power)?.render(&mut scene, [w, h])` draws on a card of its own and reads back BGRA bytes:
  use it in `Rig` tests (skip the test when it returns Err). Reference: `src/bin/memrium_maps` (`gpu.rs`, `card.rs`,
  `tests.rs::card_frame`).
- Keep a processor fallback for `gpu_layer == false` (the maps app keeps its old CPU renderer).

## 11. Conventions

- Docs and comments in English; talk to the user in Ukrainian.
- Use only `ui.ink` / `ui.accent` / `ui.dark` for colours so every theme works; status colours excepted.
- Never leave a new motion static: ease hovers, spring pop-ins, glide selections.
- Keep a module per feature next to `main.rs` (`src/bin/<app>/<module>.rs`, `mod x;`) instead of growing `main.rs`
  (memrium_icons main.rs is already ~3800 lines).
