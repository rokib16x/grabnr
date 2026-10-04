//! Where the menu bar / tray popover goes: next to the tray icon, on the screen it was clicked on.

/// A rectangle in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Top-left corner for a popover of `size` (width, height) opened from `icon` on `screen`.
///
/// It is centred on the icon and kept inside the screen. The menu bar on macOS sits at the top, so the popover
/// opens below the icon; a Windows taskbar sits at the bottom, so it opens above the icon. Whichever half of the
/// screen the icon is in decides.
pub fn position(icon: Rect, size: (f64, f64), screen: Rect, margin: f64) -> (f64, f64) {
    let mut x = icon.x + icon.w / 2.0 - size.0 / 2.0;
    x = x.clamp(screen.x + margin, (screen.x + screen.w - size.0 - margin).max(screen.x + margin));
    let icon_in_bottom_half = icon.y + icon.h / 2.0 > screen.y + screen.h / 2.0;
    let y = if icon_in_bottom_half { icon.y - size.1 - margin } else { icon.y + icon.h + margin };
    (x, y.clamp(screen.y, (screen.y + screen.h - size.1).max(screen.y)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 };

    #[test]
    fn opens_below_a_menu_bar_icon_and_centred() {
        let icon = Rect { x: 1500.0, y: 0.0, w: 24.0, h: 24.0 };
        assert_eq!(position(icon, (340.0, 420.0), SCREEN, 6.0), (1342.0, 30.0));
    }

    #[test]
    fn opens_above_a_taskbar_icon() {
        let icon = Rect { x: 1800.0, y: 1050.0, w: 24.0, h: 24.0 };
        let (x, y) = position(icon, (340.0, 420.0), SCREEN, 6.0);
        assert_eq!(y, 1050.0 - 420.0 - 6.0);
        assert!(x + 340.0 <= 1920.0 - 6.0, "kept inside the screen: {x}");
    }

    #[test]
    fn stays_on_the_screen_it_was_clicked_on() {
        let right = Rect { x: 1920.0, y: 0.0, w: 1920.0, h: 1080.0 };
        let icon = Rect { x: 1925.0, y: 0.0, w: 24.0, h: 24.0 };
        let (x, _) = position(icon, (340.0, 420.0), right, 8.0);
        assert!(x >= 1920.0 + 8.0, "not pushed onto the neighbouring screen: {x}");
    }

    #[test]
    fn never_leaves_a_small_screen() {
        let tiny = Rect { x: 0.0, y: 0.0, w: 300.0, h: 300.0 };
        let (x, y) = position(Rect { x: 150.0, y: 280.0, w: 20.0, h: 20.0 }, (340.0, 420.0), tiny, 6.0);
        assert!(x >= 0.0 && y >= 0.0);
    }
}
