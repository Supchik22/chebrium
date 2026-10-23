//! Chebrium: the UI/UX toolkit behind every Memrium program.
//!
//! It is made to build pretty, animated and *very light* programs:
//!
//! * [`spring`] and [`anim`]: the motion that gives everything its liquid feel. Both know when they are at
//!   rest, so a program that is not moving draws nothing and uses no CPU.
//! * [`canvas`]: a small anti-aliased software painter over a shared-memory ARGB buffer. No GPU context, no
//!   toolkit runtime: a window costs a few megabytes.
//! * [`lucide`], [`text`]: the bundled icon set and the Inter typeface.
//! * [`record`]: the frame store behind [`app`]: widgets are recorded as commands, a frame identical to the last
//!   one is skipped, and otherwise only the changed rectangle is painted again.
//! * [`draw`], [`ui`], [`form`]: an immediate-mode widget kit (buttons, switches, sliders, segmented
//!   controls, text fields, settings forms) written against the [`draw::Draw`] trait, so the same code
//!   paints with the software canvas here ([`soft`]) or with the desktop's GPU glass renderer.
//! * [`app`]: the window runtime (Wayland, shared memory, frame callbacks): a program is one `App` impl and a
//!   call to [`app::run`]. It redraws only on input and while something animates.
//! * [`chrome`]: a header drawn in the desktop's style for programs that decorate their own windows.
//! * [`mem`]: fewer allocator arenas and `trim`, so a long-running program does not hold on to what it freed.
//! * [`theme`]: the look of the desktop (light or dark, accent, glass, window shape) the program follows live.
//! * [`i18n`]: translations; every text a widget shows goes through it (English is the key).
//! * [`input`]: the keys and modifiers the widgets understand.
//!
//! A program keeps a [`ui::State`] between frames, builds a [`ui::Ui`] over a [`draw::Draw`] each frame,
//! calls the widgets and asks [`ui::Ui::animating`] whether another frame is needed.

pub mod anim;
#[cfg(feature = "appmenu")]
pub mod appmenu;
#[cfg(feature = "app")]
pub mod app;
pub mod canvas;
#[cfg(feature = "app")]
pub mod chrome;
pub mod draw;
pub mod form;
pub mod glyphs;
pub mod i18n;
pub mod input;
pub mod lucide;
pub mod mem;
pub mod picture;
pub mod record;
pub mod retro;
pub mod soft;
pub mod spring;
pub mod style;
pub mod text;
pub mod theme;
pub mod ui;
