//! Draws the menu bar meter: two bars, 5-hour on top and weekly below.
//!
//! On macOS the image is a "template" (black with alpha), so the system tints
//! it for light and dark menu bars. Windows does not tint tray icons and
//! shows them square, so it gets a coloured square variant.

pub const WIDTH: u32 = 40;
pub const HEIGHT: u32 = 36;
/// Side of the square tray icon used on Windows.
pub const SQUARE: u32 = 32;

const TRACK_ALPHA: u8 = 120;
const FILL_ALPHA: u8 = 255;
const BAR_X: u32 = 2;
const BAR_W: u32 = 36;
const BAR_H: u32 = 11;
const BAR_TOPS: [u32; 2] = [5, 20];
const RADIUS: i32 = 3;

/// Grey that reads on both light and dark taskbars.
const TRACK_RGBA: [u8; 4] = [128, 128, 128, 150];

struct Layout {
    width: u32,
    bar_x: u32,
    bar_w: u32,
    bar_h: u32,
    tops: [u32; 2],
}

const TEMPLATE: Layout = Layout { width: WIDTH, bar_x: BAR_X, bar_w: BAR_W, bar_h: BAR_H, tops: BAR_TOPS };
const SQUARE_LAYOUT: Layout = Layout { width: SQUARE, bar_x: 2, bar_w: 28, bar_h: 12, tops: [3, 17] };

/// RGBA pixels, `WIDTH * HEIGHT * 4` bytes. A value of `None` draws an empty
/// track (no data yet).
pub fn render_meter(five_hour: Option<f64>, seven_day: Option<f64>) -> Vec<u8> {
    let mut px = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    draw(&mut px, &TEMPLATE, [five_hour, seven_day], |_| [0, 0, 0, FILL_ALPHA], [0, 0, 0, TRACK_ALPHA]);
    px
}

/// RGBA pixels, `SQUARE * SQUARE * 4` bytes, with fills coloured by level:
/// green, amber from 60%, red from 85% (the popover uses the same steps).
pub fn render_meter_color(five_hour: Option<f64>, seven_day: Option<f64>) -> Vec<u8> {
    let mut px = vec![0u8; (SQUARE * SQUARE * 4) as usize];
    draw(&mut px, &SQUARE_LAYOUT, [five_hour, seven_day], level_color, TRACK_RGBA);
    px
}

fn level_color(pct: f64) -> [u8; 4] {
    if pct >= 85.0 {
        [242, 103, 95, 255]
    } else if pct >= 60.0 {
        [240, 162, 59, 255]
    } else {
        [76, 195, 138, 255]
    }
}

fn draw(px: &mut [u8], l: &Layout, values: [Option<f64>; 2], fill: impl Fn(f64) -> [u8; 4], track: [u8; 4]) {
    for (top, value) in l.tops.into_iter().zip(values) {
        let filled = value.map(|v| ((v.clamp(0.0, 100.0) / 100.0) * l.bar_w as f64).round() as u32).unwrap_or(0);
        // Any non-zero usage shows at least a sliver.
        let filled = if value.is_some_and(|v| v > 0.0) { filled.max(2) } else { filled };
        let fill_rgba = fill(value.unwrap_or(0.0));
        for y in 0..l.bar_h {
            for x in 0..l.bar_w {
                if !inside_rounded(x as i32, y as i32, l.bar_w as i32, l.bar_h as i32) {
                    continue;
                }
                let rgba = if x < filled { fill_rgba } else { track };
                let i = (((top + y) * l.width + l.bar_x + x) * 4) as usize;
                px[i..i + 4].copy_from_slice(&rgba);
            }
        }
    }
}

fn inside_rounded(x: i32, y: i32, w: i32, h: i32) -> bool {
    let r = RADIUS;
    let cx = if x < r { r } else if x >= w - r { w - r - 1 } else { x };
    let cy = if y < r { r } else if y >= h - r { h - r - 1 } else { y };
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= r * r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(px: &[u8], x: u32, y: u32) -> u8 {
        px[((y * WIDTH + x) * 4 + 3) as usize]
    }

    #[test]
    fn bars_fill_proportionally() {
        let px = render_meter(Some(50.0), Some(100.0));
        assert_eq!(px.len(), (WIDTH * HEIGHT * 4) as usize);
        let mid = BAR_TOPS[0] + BAR_H / 2;
        assert_eq!(alpha(&px, BAR_X + 5, mid), FILL_ALPHA);
        assert_eq!(alpha(&px, BAR_X + BAR_W - 5, mid), TRACK_ALPHA);
        let mid2 = BAR_TOPS[1] + BAR_H / 2;
        assert_eq!(alpha(&px, BAR_X + BAR_W - 5, mid2), FILL_ALPHA);
        // Gap between the bars and the corners stay transparent.
        assert_eq!(alpha(&px, 20, 18), 0);
        assert_eq!(alpha(&px, BAR_X, BAR_TOPS[0]), 0);
        // Template image: colour channels stay black.
        assert!(px.chunks(4).all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0));
    }

    #[test]
    fn square_variant_is_coloured_by_level() {
        let px = render_meter_color(Some(30.0), Some(90.0));
        assert_eq!(px.len(), (SQUARE * SQUARE * 4) as usize);
        let at = |x: u32, y: u32| &px[((y * SQUARE + x) * 4) as usize..((y * SQUARE + x) * 4 + 4) as usize];
        let (top, bottom) = (SQUARE_LAYOUT.tops[0] + 6, SQUARE_LAYOUT.tops[1] + 6);
        assert_eq!(at(4, top), level_color(30.0), "green fill");
        assert_eq!(at(28, top), TRACK_RGBA, "track past 30%");
        assert_eq!(at(24, bottom), level_color(90.0), "red fill at 90%");
        assert_ne!(level_color(30.0), level_color(70.0));
        assert_eq!(at(16, 0)[3], 0, "transparent margin");
    }

    #[test]
    fn unknown_values_draw_an_empty_track() {
        let px = render_meter(None, Some(0.0));
        let mid = BAR_TOPS[0] + BAR_H / 2;
        assert_eq!(alpha(&px, BAR_X + 3, mid), TRACK_ALPHA);
        assert_eq!(alpha(&px, BAR_X + 3, BAR_TOPS[1] + BAR_H / 2), TRACK_ALPHA);
    }
}
