//! Where to put the popover relative to the menu bar item, across displays
//! with different pixel densities.
//!
//! macOS reports the menu bar item and each display in "physical" pixels,
//! meaning points multiplied by that display's own scale factor. Windows,
//! however, are positioned in points, a single space shared by all displays.
//! Mixing the two puts the popover on the wrong display whenever a Retina
//! screen sits next to a non-Retina one, so everything is converted to points
//! here before any arithmetic.

/// A display as the windowing layer reports it: origin and size in physical
/// pixels (points times `scale`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

impl Screen {
    /// Bounds in points: (left, top, right, bottom).
    fn bounds(&self) -> (f64, f64, f64, f64) {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        let (l, t) = (self.x / s, self.y / s);
        (l, t, l + self.width / s, t + self.height / s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Top-left corner of the popover, in points.
    pub x: f64,
    pub y: f64,
    /// Index into the `screens` slice of the display that holds the icon.
    pub screen: Option<usize>,
}

const GAP: f64 = 4.0;
/// Menu bar height in points: 24 on regular displays, up to about 40 on
/// MacBooks with a notch.
const MENU_BAR_MIN: f64 = 20.0;
const MENU_BAR_MAX: f64 = 45.0;
const MARGIN: f64 = 8.0;

/// Top-left corner (in points) for a `win_w` x `win_h` point popover centred
/// under a menu bar item whose rectangle is given in physical pixels.
pub fn popover_origin(icon_x: f64, icon_y: f64, icon_w: f64, icon_h: f64, screens: &[Screen], win_w: f64, win_h: f64) -> Placement {
    // The icon's rectangle was scaled by the factor of the display it is on,
    // which we are not told. Undo each display's scale in turn and keep the
    // displays where the result lands inside their own bounds.
    //
    // With mixed densities one pixel position can land inside two displays
    // (pixel 2000 is point 2000 at 1x and point 1000 at 2x), so candidates are
    // ranked by two things only the right one gets right: the item is as tall
    // as a menu bar in points, and it sits at the very top of the display.
    let centre = |scale: f64| ((icon_x + icon_w / 2.0) / scale, (icon_y + icon_h / 2.0) / scale);
    let found = screens
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let scale = if s.scale > 0.0 { s.scale } else { 1.0 };
            let (cx, cy) = centre(scale);
            let (l, t, r, b) = s.bounds();
            if !(cx >= l && cx < r && cy >= t && cy < b) {
                return None;
            }
            let bar_height = icon_h / scale;
            let plausible_height = (MENU_BAR_MIN..=MENU_BAR_MAX).contains(&bar_height);
            let at_top = (icon_y / scale - t).abs() <= 2.0;
            Some((plausible_height as u8 + at_top as u8, i, s))
        })
        // Highest score wins; on a tie the earlier display does.
        .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)))
        .map(|(_, i, s)| (i, s));

    let scale = found.map(|(_, s)| s.scale).or(screens.first().map(|s| s.scale)).filter(|s| *s > 0.0).unwrap_or(1.0);
    let (cx, _) = centre(scale);
    let mut x = cx - win_w / 2.0;
    let mut y = (icon_y + icon_h) / scale + GAP;

    if let Some((_, s)) = found {
        let (l, t, r, b) = s.bounds();
        x = x.clamp(l + MARGIN, (r - win_w - MARGIN).max(l + MARGIN));
        y = y.clamp(t, (b - win_h - MARGIN).max(t));
    }
    Placement { x, y, screen: found.map(|(i, _)| i) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f64 = 384.0;
    const H: f64 = 640.0;

    /// 1512x982 pt Retina laptop panel at the origin.
    fn laptop() -> Screen {
        Screen { x: 0.0, y: 0.0, width: 3024.0, height: 1964.0, scale: 2.0 }
    }

    /// Icon rectangle in physical pixels for an item at `x_pt`, `y_pt` points
    /// on a display with `scale`.
    fn icon(x_pt: f64, y_pt: f64, scale: f64) -> (f64, f64, f64, f64) {
        (x_pt * scale, y_pt * scale, 90.0 * scale, 24.0 * scale)
    }

    #[test]
    fn single_retina_display() {
        let (x, y, w, h) = icon(400.0, 0.0, 2.0);
        let p = popover_origin(x, y, w, h, &[laptop()], W, H);
        assert_eq!(p.screen, Some(0));
        assert_eq!(p.x, 400.0 + 45.0 - W / 2.0);
        assert_eq!(p.y, 28.0);
    }

    #[test]
    fn external_non_retina_display_to_the_right() {
        // 1920x1080 at 1x, placed right of the laptop: starts at x = 1512 pt.
        let external = Screen { x: 1512.0, y: 0.0, width: 1920.0, height: 1080.0, scale: 1.0 };
        let screens = [laptop(), external];
        let (x, y, w, h) = icon(1512.0 + 600.0, 0.0, 1.0);
        let p = popover_origin(x, y, w, h, &screens, W, H);
        assert_eq!(p.screen, Some(1), "must stay on the external display");
        assert_eq!(p.x, 2112.0 + 45.0 - W / 2.0);
        assert!(p.x >= 1512.0 && p.x + W <= 1512.0 + 1920.0);
        assert_eq!(p.y, 28.0);

        // The same item on the laptop still resolves to the laptop, including
        // at a pixel position that would also fit inside the external display.
        for x_pt in [600.0, 900.0, 1300.0] {
            let (x, y, w, h) = icon(x_pt, 0.0, 2.0);
            let p = popover_origin(x, y, w, h, &screens, W, H);
            assert_eq!(p.screen, Some(0), "item at {x_pt} pt on the laptop");
            assert!(p.x + W <= 1512.0);
        }
        // And every position along the external display's menu bar stays there.
        for x_pt in [1512.0, 1800.0, 2400.0, 3300.0] {
            let (x, y, w, h) = icon(x_pt, 0.0, 1.0);
            assert_eq!(popover_origin(x, y, w, h, &screens, W, H).screen, Some(1), "item at {x_pt} pt on the external display");
        }
    }

    #[test]
    fn external_display_to_the_left_has_negative_coordinates() {
        let external = Screen { x: -2560.0, y: 0.0, width: 2560.0, height: 1440.0, scale: 1.0 };
        let (x, y, w, h) = icon(-700.0, 0.0, 1.0);
        let p = popover_origin(x, y, w, h, &[laptop(), external], W, H);
        assert_eq!(p.screen, Some(1));
        assert!(p.x >= -2560.0 && p.x + W <= 0.0, "{p:?}");
    }

    #[test]
    fn retina_external_above_the_laptop() {
        // 4K at 2x (1920x1080 pt) above the laptop: y from -1080 pt to 0.
        let external = Screen { x: -408.0, y: -2160.0, width: 3840.0, height: 2160.0, scale: 2.0 };
        let (x, y, w, h) = icon(300.0, -1080.0, 2.0);
        let p = popover_origin(x, y, w, h, &[laptop(), external], W, H);
        assert_eq!(p.screen, Some(1));
        assert_eq!(p.y, -1080.0 + 24.0 + 4.0);
    }

    #[test]
    fn notch_laptop_next_to_a_lower_external_display() {
        // MacBook with a 37 pt menu bar, external 1x display below-right.
        let external = Screen { x: 1512.0, y: 300.0, width: 1920.0, height: 1080.0, scale: 1.0 };
        let screens = [laptop(), external];
        // On the laptop: 37 pt tall bar at 2x.
        let p = popover_origin(700.0 * 2.0, 0.0, 180.0, 74.0, &screens, W, H);
        assert_eq!(p.screen, Some(0));
        assert_eq!(p.y, 41.0);
        // On the external display: 24 pt bar at 1x, top at y = 300.
        let p = popover_origin(2000.0, 300.0, 90.0, 24.0, &screens, W, H);
        assert_eq!(p.screen, Some(1));
        assert_eq!(p.y, 328.0);
    }

    #[test]
    fn stays_inside_the_screen_edges() {
        // Item at the far right of the laptop's menu bar.
        let (x, y, w, h) = icon(1420.0, 0.0, 2.0);
        let p = popover_origin(x, y, w, h, &[laptop()], W, H);
        assert_eq!(p.x, 1512.0 - W - 8.0);
        // Item at the far left.
        let (x, y, w, h) = icon(0.0, 0.0, 2.0);
        assert_eq!(popover_origin(x, y, w, h, &[laptop()], W, H).x, 8.0);
    }

    #[test]
    fn unknown_display_falls_back_without_clamping() {
        let p = popover_origin(5000.0, 0.0, 100.0, 48.0, &[], W, H);
        assert_eq!(p.screen, None);
        assert_eq!(p.x, 5050.0 - W / 2.0);
    }
}
