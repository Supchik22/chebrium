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
helpers (`swatch`, `pick`, `percent`, `section_icon`, `reset_link`, `fit`) in `src/bin/memrium_icons/main.rs`.

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

## 10. Conventions

- Docs and comments in English; talk to the user in Ukrainian.
- Use only `ui.ink` / `ui.accent` / `ui.dark` for colours so every theme works; status colours excepted.
- Never leave a new motion static: ease hovers, spring pop-ins, glide selections.
- Keep a module per feature next to `main.rs` (`src/bin/<app>/<module>.rs`, `mod x;`) instead of growing `main.rs`
  (memrium_icons main.rs is already ~3800 lines).
