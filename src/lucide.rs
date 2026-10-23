//! Lucide icons (https://lucide.dev, ISC licence): embedded SVGs, rasterised with resvg.
//!
//! Lucide strokes use `currentColor`, so an icon is tinted by substituting the colour before rasterising.
//! To add an icon: drop its SVG into `assets/lucide/` and list it below (keep the atlas under 256 entries).

use resvg::{tiny_skia, usvg};

/// (name, svg source). The index in this list is the icon id used by the glyph shader.
pub const ICONS: &[(&str, &str)] = &[
    ("folder", include_str!("../assets/lucide/folder.svg")),
    ("captions", include_str!("../assets/lucide/captions.svg")),
    ("repeat", include_str!("../assets/lucide/repeat.svg")),
    ("gauge", include_str!("../assets/lucide/gauge.svg")),
    ("move", include_str!("../assets/lucide/move.svg")),
    ("panel-top", include_str!("../assets/lucide/panel-top.svg")),
    ("rotate-ccw", include_str!("../assets/lucide/rotate-ccw.svg")),
    ("equal", include_str!("../assets/lucide/equal.svg")),
    ("globe", include_str!("../assets/lucide/globe.svg")),
    ("plus", include_str!("../assets/lucide/plus.svg")),
    ("x", include_str!("../assets/lucide/x.svg")),
    ("terminal", include_str!("../assets/lucide/terminal.svg")),
    ("wifi", include_str!("../assets/lucide/wifi.svg")),
    ("bluetooth", include_str!("../assets/lucide/bluetooth.svg")),
    ("airplay", include_str!("../assets/lucide/airplay.svg")),
    ("radar", include_str!("../assets/lucide/radar.svg")),
    ("moon", include_str!("../assets/lucide/moon.svg")),
    ("sun", include_str!("../assets/lucide/sun.svg")),
    ("volume-2", include_str!("../assets/lucide/volume-2.svg")),
    ("search", include_str!("../assets/lucide/search.svg")),
    ("battery-full", include_str!("../assets/lucide/battery-full.svg")),
    ("battery-medium", include_str!("../assets/lucide/battery-medium.svg")),
    ("battery-low", include_str!("../assets/lucide/battery-low.svg")),
    ("battery-charging", include_str!("../assets/lucide/battery-charging.svg")),
    ("volume-1", include_str!("../assets/lucide/volume-1.svg")),
    ("volume-x", include_str!("../assets/lucide/volume-x.svg")),
    ("sliders-horizontal", include_str!("../assets/lucide/sliders-horizontal.svg")),
    ("play", include_str!("../assets/lucide/play.svg")),
    ("skip-back", include_str!("../assets/lucide/skip-back.svg")),
    ("skip-forward", include_str!("../assets/lucide/skip-forward.svg")),
    ("layout-panel-left", include_str!("../assets/lucide/layout-panel-left.svg")),
    ("screen-share", include_str!("../assets/lucide/screen-share.svg")),
    ("monitor", include_str!("../assets/lucide/monitor.svg")),
    ("app-window", include_str!("../assets/lucide/app-window.svg")),
    ("settings", include_str!("../assets/lucide/settings.svg")),
    ("image", include_str!("../assets/lucide/image.svg")),
    ("file-text", include_str!("../assets/lucide/file-text.svg")),
    ("folder-open", include_str!("../assets/lucide/folder-open.svg")),
    ("user", include_str!("../assets/lucide/user.svg")),
    ("power", include_str!("../assets/lucide/power.svg")),
    ("bell", include_str!("../assets/lucide/bell.svg")),
    ("chevron-down", include_str!("../assets/lucide/chevron-down.svg")),
    ("chevron-up", include_str!("../assets/lucide/chevron-up.svg")),
    ("copy", include_str!("../assets/lucide/copy.svg")),
    ("square", include_str!("../assets/lucide/square.svg")),
    ("minus", include_str!("../assets/lucide/minus.svg")),
    ("contrast", include_str!("../assets/lucide/contrast.svg")),
    ("camera", include_str!("../assets/lucide/camera.svg")),
    ("laptop", include_str!("../assets/lucide/laptop.svg")),
    ("user-round", include_str!("../assets/lucide/user-round.svg")),
    ("house", include_str!("../assets/lucide/house.svg")),
    ("lock", include_str!("../assets/lucide/lock.svg")),
    ("sticky-note", include_str!("../assets/lucide/sticky-note.svg")),
    ("audio-lines", include_str!("../assets/lucide/audio-lines.svg")),
    ("layers", include_str!("../assets/lucide/layers.svg")),
    ("wrench", include_str!("../assets/lucide/wrench.svg")),
    ("mic", include_str!("../assets/lucide/mic.svg")),
    ("accessibility", include_str!("../assets/lucide/accessibility.svg")),
    ("ear", include_str!("../assets/lucide/ear.svg")),
    ("layout-grid", include_str!("../assets/lucide/layout-grid.svg")),
    ("clock", include_str!("../assets/lucide/clock.svg")),
    ("lightbulb", include_str!("../assets/lucide/lightbulb.svg")),
    ("alarm-clock", include_str!("../assets/lucide/alarm-clock.svg")),
    ("timer", include_str!("../assets/lucide/timer.svg")),
    ("columns-2", include_str!("../assets/lucide/columns-2.svg")),
    ("monitor-play", include_str!("../assets/lucide/monitor-play.svg")),
    ("wifi-off", include_str!("../assets/lucide/wifi-off.svg")),
    ("bluetooth-off", include_str!("../assets/lucide/bluetooth-off.svg")),
    ("audio-waveform", include_str!("../assets/lucide/audio-waveform.svg")),
    ("notebook-pen", include_str!("../assets/lucide/notebook-pen.svg")),
    ("hourglass", include_str!("../assets/lucide/hourglass.svg")),
    ("arrow-left", include_str!("../assets/lucide/arrow-left.svg")),
    ("arrow-right", include_str!("../assets/lucide/arrow-right.svg")),
    ("arrow-up", include_str!("../assets/lucide/arrow-up.svg")),
    ("refresh-cw", include_str!("../assets/lucide/refresh-cw.svg")),
    ("chevron-right", include_str!("../assets/lucide/chevron-right.svg")),
    ("chevron-left", include_str!("../assets/lucide/chevron-left.svg")),
    ("file", include_str!("../assets/lucide/file.svg")),
    ("file-image", include_str!("../assets/lucide/file-image.svg")),
    ("file-music", include_str!("../assets/lucide/file-music.svg")),
    ("file-archive", include_str!("../assets/lucide/file-archive.svg")),
    ("file-code", include_str!("../assets/lucide/file-code.svg")),
    ("download", include_str!("../assets/lucide/download.svg")),
    ("trash-2", include_str!("../assets/lucide/trash-2.svg")),
    ("pencil", include_str!("../assets/lucide/pencil.svg")),
    ("scissors", include_str!("../assets/lucide/scissors.svg")),
    ("clipboard-paste", include_str!("../assets/lucide/clipboard-paste.svg")),
    ("undo-2", include_str!("../assets/lucide/undo-2.svg")),
    ("list", include_str!("../assets/lucide/list.svg")),
    ("panel-left", include_str!("../assets/lucide/panel-left.svg")),
    ("calendar", include_str!("../assets/lucide/calendar.svg")),
    ("arrow-down-a-z", include_str!("../assets/lucide/arrow-down-a-z.svg")),
    ("ruler", include_str!("../assets/lucide/ruler.svg")),
    ("shapes", include_str!("../assets/lucide/shapes.svg")),
    ("minimize-2", include_str!("../assets/lucide/minimize-2.svg")),
    ("maximize-2", include_str!("../assets/lucide/maximize-2.svg")),
    ("eraser", include_str!("../assets/lucide/eraser.svg")),
    ("pipette", include_str!("../assets/lucide/pipette.svg")),
    ("paint-bucket", include_str!("../assets/lucide/paint-bucket.svg")),
    ("brush", include_str!("../assets/lucide/brush.svg")),
    ("circle", include_str!("../assets/lucide/circle.svg")),
    ("triangle", include_str!("../assets/lucide/triangle.svg")),
    ("redo-2", include_str!("../assets/lucide/redo-2.svg")),
    ("save", include_str!("../assets/lucide/save.svg")),
    ("line-diag", include_str!("../assets/lucide/line-diag.svg")),
    ("rounded-square", include_str!("../assets/lucide/rounded-square.svg")),
    ("zoom-in", include_str!("../assets/lucide/zoom-in.svg")),
    ("zoom-out", include_str!("../assets/lucide/zoom-out.svg")),
    ("square-dashed", include_str!("../assets/lucide/square-dashed.svg")),
    ("type", include_str!("../assets/lucide/type.svg")),
    ("crop", include_str!("../assets/lucide/crop.svg")),
    ("rotate-cw", include_str!("../assets/lucide/rotate-cw.svg")),
    ("pause", include_str!("../assets/lucide/pause.svg")),
    ("music-2", include_str!("../assets/lucide/music-2.svg")),
    ("eye", include_str!("../assets/lucide/eye.svg")),
    ("eye-off", include_str!("../assets/lucide/eye-off.svg")),
    ("bell-off", include_str!("../assets/lucide/bell-off.svg")),
    ("pin", include_str!("../assets/lucide/pin.svg")),
    ("shield", include_str!("../assets/lucide/shield.svg")),
    ("shield-check", include_str!("../assets/lucide/shield-check.svg")),
    ("keyboard", include_str!("../assets/lucide/keyboard.svg")),
    ("server", include_str!("../assets/lucide/server.svg")),
    ("hard-drive", include_str!("../assets/lucide/hard-drive.svg")),
    ("star", include_str!("../assets/lucide/star.svg")),
    ("folder-plus", include_str!("../assets/lucide/folder-plus.svg")),
    ("file-plus", include_str!("../assets/lucide/file-plus.svg")),
    ("info", include_str!("../assets/lucide/info.svg")),
    ("check", include_str!("../assets/lucide/check.svg")),
    ("arrow-up-down", include_str!("../assets/lucide/arrow-up-down.svg")),
    ("external-link", include_str!("../assets/lucide/external-link.svg")),
    ("music", include_str!("../assets/lucide/music.svg")),
    ("video", include_str!("../assets/lucide/video.svg")),
    ("folder-closed", include_str!("../assets/lucide/folder-closed.svg")),
    ("monitor-cog", include_str!("../assets/lucide/monitor-cog.svg")),
    ("layers-2", include_str!("../assets/lucide/layers-2.svg")),
    ("layers-3", include_str!("../assets/lucide/layers-3.svg")),
    ("flip-horizontal", include_str!("../assets/lucide/flip-horizontal.svg")),
    ("flip-vertical", include_str!("../assets/lucide/flip-vertical.svg")),
    ("flip-horizontal-2", include_str!("../assets/lucide/flip-horizontal-2.svg")),
    ("flip-vertical-2", include_str!("../assets/lucide/flip-vertical-2.svg")),
    ("rotate-cw-square", include_str!("../assets/lucide/rotate-cw-square.svg")),
    ("rotate-ccw-square", include_str!("../assets/lucide/rotate-ccw-square.svg")),
    ("align-center-horizontal", include_str!("../assets/lucide/align-center-horizontal.svg")),
    ("align-center-vertical", include_str!("../assets/lucide/align-center-vertical.svg")),
    ("align-start-horizontal", include_str!("../assets/lucide/align-start-horizontal.svg")),
    ("align-end-horizontal", include_str!("../assets/lucide/align-end-horizontal.svg")),
    ("align-start-vertical", include_str!("../assets/lucide/align-start-vertical.svg")),
    ("align-end-vertical", include_str!("../assets/lucide/align-end-vertical.svg")),
    ("align-horizontal-justify-center", include_str!("../assets/lucide/align-horizontal-justify-center.svg")),
    ("group", include_str!("../assets/lucide/group.svg")),
    ("ungroup", include_str!("../assets/lucide/ungroup.svg")),
    ("trash", include_str!("../assets/lucide/trash.svg")),
    ("upload", include_str!("../assets/lucide/upload.svg")),
    ("images", include_str!("../assets/lucide/images.svg")),
    ("palette", include_str!("../assets/lucide/palette.svg")),
    ("droplet", include_str!("../assets/lucide/droplet.svg")),
    ("droplets", include_str!("../assets/lucide/droplets.svg")),
    ("sparkles", include_str!("../assets/lucide/sparkles.svg")),
    ("sparkle", include_str!("../assets/lucide/sparkle.svg")),
    ("sun-moon", include_str!("../assets/lucide/sun-moon.svg")),
    ("blend", include_str!("../assets/lucide/blend.svg")),
    ("scan", include_str!("../assets/lucide/scan.svg")),
    ("scaling", include_str!("../assets/lucide/scaling.svg")),
    ("maximize", include_str!("../assets/lucide/maximize.svg")),
    ("minimize", include_str!("../assets/lucide/minimize.svg")),
    ("move-diagonal", include_str!("../assets/lucide/move-diagonal.svg")),
    ("square-round-corner", include_str!("../assets/lucide/square-round-corner.svg")),
    ("circle-dashed", include_str!("../assets/lucide/circle-dashed.svg")),
    ("wand-sparkles", include_str!("../assets/lucide/wand-sparkles.svg")),
    ("wand", include_str!("../assets/lucide/wand.svg")),
    ("sliders-vertical", include_str!("../assets/lucide/sliders-vertical.svg")),
    ("settings-2", include_str!("../assets/lucide/settings-2.svg")),
    ("grid-2x2", include_str!("../assets/lucide/grid-2x2.svg")),
    ("panel-right", include_str!("../assets/lucide/panel-right.svg")),
    ("panel-left-close", include_str!("../assets/lucide/panel-left-close.svg")),
    ("panel-right-close", include_str!("../assets/lucide/panel-right-close.svg")),
    ("sidebar", include_str!("../assets/lucide/sidebar.svg")),
    ("chevrons-up-down", include_str!("../assets/lucide/chevrons-up-down.svg")),
    ("arrow-down", include_str!("../assets/lucide/arrow-down.svg")),
    ("history", include_str!("../assets/lucide/history.svg")),
    ("file-down", include_str!("../assets/lucide/file-down.svg")),
    ("file-up", include_str!("../assets/lucide/file-up.svg")),
    ("package", include_str!("../assets/lucide/package.svg")),
    ("hammer", include_str!("../assets/lucide/hammer.svg")),
    ("smartphone", include_str!("../assets/lucide/smartphone.svg")),
    ("tablet", include_str!("../assets/lucide/tablet.svg")),
    ("mouse-pointer-2", include_str!("../assets/lucide/mouse-pointer-2.svg")),
    ("mouse-pointer", include_str!("../assets/lucide/mouse-pointer.svg")),
    ("hand", include_str!("../assets/lucide/hand.svg")),
    ("grab", include_str!("../assets/lucide/grab.svg")),
    ("crosshair", include_str!("../assets/lucide/crosshair.svg")),
    ("lock-open", include_str!("../assets/lucide/lock-open.svg")),
    ("link", include_str!("../assets/lucide/link.svg")),
    ("unlink", include_str!("../assets/lucide/unlink.svg")),
    ("bookmark", include_str!("../assets/lucide/bookmark.svg")),
    ("tag", include_str!("../assets/lucide/tag.svg")),
    ("clipboard", include_str!("../assets/lucide/clipboard.svg")),
    ("spline", include_str!("../assets/lucide/spline.svg")),
    ("pen-tool", include_str!("../assets/lucide/pen-tool.svg")),
    ("vector-square", include_str!("../assets/lucide/vector-square.svg")),
    ("hexagon", include_str!("../assets/lucide/hexagon.svg")),
    ("pentagon", include_str!("../assets/lucide/pentagon.svg")),
    ("octagon", include_str!("../assets/lucide/octagon.svg")),
    ("diamond", include_str!("../assets/lucide/diamond.svg")),
    ("heart", include_str!("../assets/lucide/heart.svg")),
    ("cloud", include_str!("../assets/lucide/cloud.svg")),
    ("zap", include_str!("../assets/lucide/zap.svg")),
    ("flame", include_str!("../assets/lucide/flame.svg")),
    ("leaf", include_str!("../assets/lucide/leaf.svg")),
    ("mountain", include_str!("../assets/lucide/mountain.svg")),
    ("film", include_str!("../assets/lucide/film.svg")),
    ("headphones", include_str!("../assets/lucide/headphones.svg")),
    ("cpu", include_str!("../assets/lucide/cpu.svg")),
    ("database", include_str!("../assets/lucide/database.svg")),
    ("code", include_str!("../assets/lucide/code.svg")),
    ("braces", include_str!("../assets/lucide/braces.svg")),
    ("bug", include_str!("../assets/lucide/bug.svg")),
    ("rocket", include_str!("../assets/lucide/rocket.svg")),
    ("users", include_str!("../assets/lucide/users.svg")),
    ("mail", include_str!("../assets/lucide/mail.svg")),
    ("message-square", include_str!("../assets/lucide/message-square.svg")),
    ("map-pin", include_str!("../assets/lucide/map-pin.svg")),
    ("compass", include_str!("../assets/lucide/compass.svg")),
    ("filter", include_str!("../assets/lucide/filter.svg")),
    ("menu", include_str!("../assets/lucide/menu.svg")),
    ("ellipsis", include_str!("../assets/lucide/ellipsis.svg")),
    ("ellipsis-vertical", include_str!("../assets/lucide/ellipsis-vertical.svg")),
    ("check-check", include_str!("../assets/lucide/check-check.svg")),
    ("circle-check", include_str!("../assets/lucide/circle-check.svg")),
    ("circle-x", include_str!("../assets/lucide/circle-x.svg")),
    ("triangle-alert", include_str!("../assets/lucide/triangle-alert.svg")),
    ("circle-dot", include_str!("../assets/lucide/circle-dot.svg")),
    ("circle-plus", include_str!("../assets/lucide/circle-plus.svg")),
    ("circle-minus", include_str!("../assets/lucide/circle-minus.svg")),
    ("move-3d", include_str!("../assets/lucide/move-3d.svg")),
    ("box", include_str!("../assets/lucide/box.svg")),
    ("boxes", include_str!("../assets/lucide/boxes.svg")),
    ("cuboid", include_str!("../assets/lucide/cuboid.svg")),
    ("cylinder", include_str!("../assets/lucide/cylinder.svg")),
    ("orbit", include_str!("../assets/lucide/orbit.svg")),
    ("globe-2", include_str!("../assets/lucide/globe-2.svg")),
    ("aperture", include_str!("../assets/lucide/aperture.svg")),
    ("focus", include_str!("../assets/lucide/focus.svg")),
    ("frame", include_str!("../assets/lucide/frame.svg")),
    ("scan-line", include_str!("../assets/lucide/scan-line.svg")),
    ("magnet", include_str!("../assets/lucide/magnet.svg")),
    ("fullscreen", include_str!("../assets/lucide/fullscreen.svg")),
    ("shrink", include_str!("../assets/lucide/shrink.svg")),
    ("expand", include_str!("../assets/lucide/expand.svg")),
    ("glasses", include_str!("../assets/lucide/glasses.svg")),
    ("eye-closed", include_str!("../assets/lucide/eye-closed.svg")),
    ("pencil-line", include_str!("../assets/lucide/pencil-line.svg")),
    ("brush-cleaning", include_str!("../assets/lucide/brush-cleaning.svg")),
    ("highlighter", include_str!("../assets/lucide/highlighter.svg")),
    ("stamp", include_str!("../assets/lucide/stamp.svg")),
    ("swatch-book", include_str!("../assets/lucide/swatch-book.svg")),
];

/// Atlas layout used by the shader: `ATLAS_COLS` x `ATLAS_COLS` cells of `CELL` px, with `MIPS` mip levels.
pub const ATLAS_COLS: u32 = 16;
pub const CELL: u32 = 64;
pub const MIPS: u32 = 4;

/// Icon id for a name (panics on a typo, which is a programming error).
pub fn id(name: &str) -> u32 {
    ICONS.iter().position(|(n, _)| *n == name).unwrap_or_else(|| panic!("unknown lucide icon `{name}`")) as u32
}

fn svg_source(name: &str) -> &'static str {
    ICONS[id(name) as usize].1
}

/// Rasterise to a `px` x `px` premultiplied RGBA8 bitmap tinted with `color`.
pub fn rasterize(name: &str, px: u32, color: [u8; 3]) -> Vec<u8> {
    let hex = format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]);
    let svg = svg_source(name).replace("currentColor", &hex);
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).expect("valid lucide svg");
    let mut pixmap = tiny_skia::Pixmap::new(px.max(1), px.max(1)).unwrap();
    let scale = px as f32 / tree.size().width();
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.take()
}

/// All icons as white-on-transparent in one atlas; every mip level is rasterised at its own size
/// (not downsampled) so thin strokes stay crisp when drawn small. Returns the mips concatenated.
pub fn build_atlas() -> Vec<u8> {
    let mut out = Vec::new();
    for mip in 0..MIPS {
        let cell = CELL >> mip;
        let side = (ATLAS_COLS * cell) as usize;
        let mut level = vec![0u8; side * side * 4];
        for (i, (name, _)) in ICONS.iter().enumerate() {
            let bitmap = rasterize(name, cell, [255, 255, 255]);
            let (cx, cy) = ((i as u32 % ATLAS_COLS) * cell, (i as u32 / ATLAS_COLS) * cell);
            for row in 0..cell as usize {
                let dst = ((cy as usize + row) * side + cx as usize) * 4;
                let src = row * cell as usize * 4;
                level[dst..dst + cell as usize * 4].copy_from_slice(&bitmap[src..src + cell as usize * 4]);
            }
        }
        out.extend(level);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_parses_and_fits_the_atlas() {
        assert!(ICONS.len() <= (ATLAS_COLS * ATLAS_COLS) as usize, "{} icons do not fit", ICONS.len());
        for (name, _) in ICONS {
            let bmp = rasterize(name, 16, [255, 255, 255]);
            assert!(bmp.iter().any(|b| *b != 0), "{name} is empty");
        }
        let mut names: Vec<&str> = ICONS.iter().map(|i| i.0).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), ICONS.len(), "an icon is listed twice");
    }
}
