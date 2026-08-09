//! Pure per-frame policy decisions for the render loop.
//!
//! `App::render()` mixes two concerns: deciding *what* to draw (which tabs,
//! whether a scrollbar earns its pixels, where the active pane sits in window
//! space) and mechanically *painting* it through the renderer. The most
//! failure-prone policy half (scrollbar gating, active-pane geometry, tab
//! titles) lives here so it can be unit-tested without a wgpu device, a window,
//! or a session — the exact failures this session hit (a full-height neon
//! scrollbar, an off-screen viewport) are geometry mistakes that only a pure,
//! tested function excludes. The paint loop still owns the pane iteration.

use zeroterm_render::TabInfo;

/// Window-space rect (pixels) of the active pane, including the floating-pane
/// transform. The scrollbar overlay is anchored to this.
///
/// `normal` is the pane's normalized `(x, y, w, h)` rect (unused in the
/// floating case, where the pane floats centered at 70% width).
pub fn active_pane_rect(
    floating: bool,
    win_w: f32,
    tab_h: f32,
    content_h: f32,
    normal: (f32, f32, f32, f32),
) -> (f32, f32, f32, f32) {
    if floating {
        let fw = win_w * 0.7;
        let fx = (win_w - fw) / 2.0;
        (fx, tab_h + content_h * 0.15, fw, content_h * 0.7)
    } else {
        let (nx, ny, nw, nh) = normal;
        (
            nx * win_w,
            ny * content_h + tab_h,
            nw * win_w,
            nh * content_h,
        )
    }
}

/// Scrollbar decision for a pane: `None` when scrollback is too shallow to
/// earn a scrollbar, otherwise `(scroll_fraction, thumb_fraction)`.
///
/// The thumb fraction is the viewport's share of the total content
/// (`visible / (visible + scrollback)`); a near-full thumb would read as a
/// plain colored strip on the right edge (the original bug painted it solid
/// accent blue at full height), so anything ≥ 85% is hidden instead.
pub fn scrollbar_policy(
    max_scroll: usize,
    scroll_offset: usize,
    visible_rows: usize,
) -> Option<(f32, f32)> {
    if max_scroll == 0 {
        return None;
    }
    let fraction = scroll_offset as f32 / max_scroll as f32;
    let thumb_fraction = visible_rows.max(1) as f32 / (visible_rows + max_scroll) as f32;
    if thumb_fraction < 0.85 {
        Some((fraction, thumb_fraction))
    } else {
        None
    }
}

/// Left-hand status-bar text: the active pane title plus the tab's position
/// (i/N) so the current tab is identifiable at a glance in a multi-tab
/// session. The tab count is clamped to ≥ 1 so an empty session can never
/// print "1/0".
pub fn status_left(title: &str, tab_index: usize, tab_count: usize) -> String {
    format!("{title} — tab {}/{}", tab_index + 1, tab_count.max(1))
}

/// Right-hand status-bar text: scroll position as a percent while there is
/// scrollback, empty otherwise.
pub fn status_right(max_scroll: usize, scroll_offset: usize) -> String {
    if max_scroll > 0 {
        format!(
            "[{}%]",
            (100 * scroll_offset)
                .checked_div(max_scroll)
                .unwrap_or(0)
        )
    } else {
        String::new()
    }
}

/// Tab-bar display title: the tab's title plus a split badge (" ▦N") when the
/// tab holds more than one pane, so split tabs are identifiable at a glance.
/// The tab-bar draw loop and the hit-testing (tab_at_point / tab_bar_hover)
/// must agree on the exact string — this is the single source for both.
pub fn tab_display_title(title: &str, pane_count: usize) -> String {
    if pane_count > 1 {
        format!("{title} ▦{pane_count}")
    } else {
        title.to_string()
    }
}

/// Tab strip content. While the line editor is open the active tab shows the
/// live buffer instead of the shell title.
pub fn tab_infos(
    ids: &[usize],
    active_pane: usize,
    titles: impl Fn(usize) -> String,
    edit_display: Option<&str>,
    hovered: Option<usize>,
    close_hovered: bool,
) -> Vec<TabInfo> {
    ids.iter()
        .map(|&id| TabInfo {
            title: match edit_display {
                Some(d) if id == active_pane => d.to_string(),
                _ => titles(id),
            },
            active: id == active_pane,
            hovered: hovered == Some(id),
            close_hovered,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_pane_rect_floating_centers_and_scales() {
        let (x, y, w, h) = active_pane_rect(true, 1000.0, 30.0, 700.0, (0.0, 0.0, 1.0, 1.0));
        assert_eq!(x, 150.0); // (1000 - 700) / 2
        assert_eq!(y, 30.0 + 105.0);
        assert_eq!(w, 700.0);
        assert_eq!(h, 490.0);
        // The normal rect must be ignored while floating (no drift).
        let (x2, _, _, _) = active_pane_rect(true, 1000.0, 30.0, 700.0, (0.9, 0.9, 0.1, 0.1));
        assert_eq!(x2, 150.0);
    }

    #[test]
    fn active_pane_rect_normal_maps_to_window_space() {
        let (x, y, w, h) = active_pane_rect(false, 1000.0, 30.0, 700.0, (0.25, 0.5, 0.5, 0.25));
        assert_eq!(x, 250.0);
        assert_eq!(y, 30.0 + 350.0);
        assert_eq!(w, 500.0);
        assert_eq!(h, 175.0);
    }

    #[test]
    fn scrollbar_policy_hides_trivial_scrollback() {
        // No scrollback at all → no scrollbar.
        assert_eq!(scrollbar_policy(0, 0, 24), None);
        // 5 lines of scrollback in a 49-row viewport: thumb 49/54 ≈ 0.907 ≥ 0.85.
        assert_eq!(scrollbar_policy(5, 0, 49), None);
        // Deep scrollback → bar with correct fractions.
        let (f, t) = scrollbar_policy(500, 100, 49).unwrap();
        assert!((f - 0.2).abs() < 1e-6, "fraction: {f}");
        assert!(t < 0.2, "thumb fraction: {t}");
        // Visible rows of 0 must not divide by zero (clamped to 1 row).
        assert_eq!(scrollbar_policy(500, 0, 0), Some((0.0, 0.002)));
    }

    #[test]
    fn status_left_shows_tab_position() {
        assert_eq!(status_left("bash", 0, 3), "bash — tab 1/3");
        assert_eq!(status_left("ssh host", 2, 3), "ssh host — tab 3/3");
        // Degenerate empty session cannot print a 0 denominator.
        assert_eq!(status_left("bash", 0, 0), "bash — tab 1/1");
    }

    #[test]
    fn status_right_percent_and_empty() {
        assert_eq!(status_right(0, 0), "");
        assert_eq!(status_right(100, 25), "[25%]");
        // Mirrors the original scroll-percent semantics (no clamp).
        assert_eq!(status_right(3, 7), "[233%]");
    }

    #[test]
    fn tab_infos_orders_and_flags() {
        let infos = tab_infos(
            &[1, 2, 3],
            2,
            |id| format!("title{id}"),
            None,
            Some(3),
            true,
        );
        assert_eq!(infos.len(), 3);
        assert_eq!(infos[0].title, "title1");
        assert!(infos[1].active);
        assert!(infos[2].hovered && infos[2].close_hovered);
        assert!(!infos[0].hovered);
    }

    #[test]
    fn tab_display_title_adds_split_badge_only_for_splits() {
        assert_eq!(tab_display_title("bash", 1), "bash");
        assert_eq!(tab_display_title("bash", 3), "bash ▦3");
        assert_eq!(tab_display_title("", 2), " ▦2");
        // The badge must survive truncation limits the same way as the title
        // (draw and hit-test both cap at 20 chars).
        let long = "a".repeat(18);
        let badged = tab_display_title(&long, 5);
        assert!(badged.starts_with(&long));
        assert!(badged.contains("▦5"));
    }

    #[test]
    fn tab_infos_edit_display_overrides_active_title() {
        let infos = tab_infos(&[1, 2], 2, |id| format!("title{id}"), Some("ls foo"), None, false);
        assert_eq!(infos[1].title, "ls foo", "active tab shows the editor buffer");
        assert_eq!(infos[0].title, "title1");
    }
}
