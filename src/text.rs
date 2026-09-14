//! Minimal text measuring helpers.
//!
//! Terminals lay text out in cells, so every string we print has to be measured
//! in cells and not in `char`s. This is a small, pragmatic subset of the Unicode
//! East Asian Width tables: enough to keep emoji and CJK names aligned.

/// Width of a single `char` in terminal cells.
pub fn char_width(c: char) -> usize {
    let cp = c as u32;
    match cp {
        // C0/C1 controls never reach the screen.
        0x0000..=0x001F | 0x007F..=0x009F => 0,
        // Combining marks and the emoji variation selector take no space.
        0x0300..=0x036F | 0x200D | 0xFE00..=0xFE0F => 0,
        // Ranges that terminals render double width.
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F000..=0x1FAFF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

/// Width of a string in terminal cells.
pub fn width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// Truncate `s` to at most `cells`, appending `…` when anything was dropped.
pub fn truncate(s: &str, cells: usize) -> String {
    if width(s) <= cells {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = char_width(c);
        // Leave room for the ellipsis.
        if used + cw > cells.saturating_sub(1) {
            break;
        }
        out.push(c);
        used += cw;
    }
    if used < cells {
        out.push('…');
    }
    out
}

/// Truncate from the left instead, keeping the tail visible (used for paths).
pub fn truncate_left(s: &str, cells: usize) -> String {
    if width(s) <= cells {
        return s.to_string();
    }
    let budget = cells.saturating_sub(1);
    let mut tail: Vec<char> = Vec::new();
    let mut used = 0;
    for c in s.chars().rev() {
        let cw = char_width(c);
        if used + cw > budget {
            break;
        }
        tail.push(c);
        used += cw;
    }
    let mut out = String::from("…");
    out.extend(tail.iter().rev());
    out
}

/// Pad (or truncate) `s` so it occupies exactly `cells` terminal cells.
pub fn fit(s: &str, cells: usize) -> String {
    let mut out = truncate(s, cells);
    let mut used = width(&out);
    while used < cells {
        out.push(' ');
        used += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_and_cjk_are_double_width() {
        assert_eq!(width("📁"), 2);
        assert_eq!(width("📁 src"), 6);
        assert_eq!(width("日本語"), 6);
        assert_eq!(width("ascii"), 5);
    }

    #[test]
    fn fit_always_lands_on_the_requested_width() {
        for s in ["", "short", "📁 a folder with a long name", "日本語のフォルダー"] {
            for cells in [1, 2, 3, 7, 12, 40] {
                assert_eq!(width(&fit(s, cells)), cells, "{s:?} at {cells}");
            }
        }
    }

    #[test]
    fn truncation_marks_what_it_drops() {
        assert_eq!(truncate("abcdef", 6), "abcdef");
        assert_eq!(truncate("abcdef", 4), "abc…");
        assert_eq!(truncate_left(r"C:\a\b\c", 6), r"…a\b\c");
        assert!(truncate_left(r"C:\very\long\path\indeed", 10).ends_with("indeed"));
        assert_eq!(width(&truncate_left(r"C:\very\long\path\indeed", 10)), 10);
    }
}
