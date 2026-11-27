//! Draws the menu bar meter: two bars, 5-hour on top and weekly below.
//!
//! The image is a macOS "template" (black with alpha), so the system tints it
//! for light and dark menu bars.

pub const WIDTH: u32 = 40;
pub const HEIGHT: u32 = 36;

const TRACK_ALPHA: u8 = 120;
const FILL_ALPHA: u8 = 255;
const BAR_X: u32 = 2;
const BAR_W: u32 = 36;
const BAR_H: u32 = 11;
const BAR_TOPS: [u32; 2] = [5, 20];
const RADIUS: i32 = 3;

/// RGBA pixels, `WIDTH * HEIGHT * 4` bytes. A value of `None` draws an empty
/// track (no data yet).
pub fn render_meter(five_hour: Option<f64>, seven_day: Option<f64>) -> Vec<u8> {
    let mut px = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    for (top, value) in BAR_TOPS.into_iter().zip([five_hour, seven_day]) {
        let filled = value.map(|v| ((v.clamp(0.0, 100.0) / 100.0) * BAR_W as f64).round() as u32).unwrap_or(0);
        // Any non-zero usage shows at least a sliver.
        let filled = if value.is_some_and(|v| v > 0.0) { filled.max(2) } else { filled };
        for y in 0..BAR_H {
            for x in 0..BAR_W {
                if !inside_rounded(x as i32, y as i32) {
                    continue;
                }
                let alpha = if x < filled { FILL_ALPHA } else { TRACK_ALPHA };
                let i = (((top + y) * WIDTH + BAR_X + x) * 4) as usize;
                px[i + 3] = alpha;
            }
        }
    }
    px
}

fn inside_rounded(x: i32, y: i32) -> bool {
    let (w, h, r) = (BAR_W as i32, BAR_H as i32, RADIUS);
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
    fn unknown_values_draw_an_empty_track() {
        let px = render_meter(None, Some(0.0));
        let mid = BAR_TOPS[0] + BAR_H / 2;
        assert_eq!(alpha(&px, BAR_X + 3, mid), TRACK_ALPHA);
        assert_eq!(alpha(&px, BAR_X + 3, BAR_TOPS[1] + BAR_H / 2), TRACK_ALPHA);
    }
}
