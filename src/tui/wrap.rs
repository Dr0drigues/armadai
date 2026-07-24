//! Approximate word-wrap line counting, used only to bound detail-view
//! scrolling (Up/Down/PageUp/PageDown must never scroll past the start or
//! end of the rendered content — see `App::detail_scroll_max`).
//!
//! ratatui's own `Paragraph::line_count` would compute this exactly, but it
//! is gated behind the `unstable-rendered-line-info` cargo feature (no
//! stability guarantee — <https://github.com/ratatui/ratatui/issues/293>),
//! which we don't enable. This reimplements just the greedy word-wrap
//! ratatui uses for `Wrap { trim: false }`: whitespace-separated words are
//! packed onto a line until the next word would overflow `width`, then a
//! new line starts. It doesn't need to match ratatui's wrapping character
//! for character, only closely enough that the computed bound never lets
//! the user scroll past real content.

use unicode_width::UnicodeWidthStr;

/// Number of display lines `text` occupies when wrapped to `width` columns
/// (mirrors `Wrap { trim: false }`). Returns 0 for `width == 0` (nothing
/// can be rendered in a zero-width area).
pub(crate) fn wrapped_line_count(text: &str, width: u16) -> usize {
    if width == 0 {
        return 0;
    }
    let width = width as usize;
    text.split('\n')
        .map(|line| wrapped_line_count_single(line, width))
        .sum()
}

fn wrapped_line_count_single(line: &str, width: usize) -> usize {
    let mut lines = 1usize;
    let mut current = 0usize;
    let mut has_content = false;

    for word in line.split(' ') {
        if word.is_empty() {
            // Collapsed/repeated whitespace: consume a column if there's
            // room. Only cosmetic for our bound-checking purpose.
            if has_content && current < width {
                current += 1;
            }
            continue;
        }

        let word_width = word.width();
        if word_width > width {
            // A single word wider than the line will be hard-broken by the
            // real renderer. We don't reproduce the exact split point, only
            // the resulting line count (see module doc).
            if has_content {
                lines += 1;
            }
            lines += word_width.div_ceil(width).saturating_sub(1);
            current = 0;
            has_content = false;
            continue;
        }

        let sep = usize::from(has_content);
        if current + sep + word_width > width {
            lines += 1;
            current = word_width;
        } else {
            current += sep + word_width;
        }
        has_content = true;
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_fits_one_line() {
        assert_eq!(wrapped_line_count("hello world", 40), 1);
    }

    #[test]
    fn two_words_wrap_when_they_dont_both_fit() {
        // "aaaaaaaaaa" (10) + space + "bbbbbbbbbb" (10) can't share a
        // width-10 line, so each word gets its own line.
        assert_eq!(
            wrapped_line_count("aaaaaaaaaa bbbbbbbbbb", 10),
            2,
            "greedy wrap must break at the word boundary, not mid-word"
        );
    }

    #[test]
    fn respects_explicit_newlines() {
        assert_eq!(wrapped_line_count("line one\nline two\nline three", 40), 3);
    }

    #[test]
    fn empty_text_is_one_line() {
        assert_eq!(wrapped_line_count("", 40), 1);
    }

    #[test]
    fn zero_width_has_no_lines() {
        assert_eq!(wrapped_line_count("anything", 0), 0);
    }

    #[test]
    fn word_wider_than_width_is_hard_broken_across_lines() {
        // A 25-char unbroken token in a 10-col line needs 3 hard-wrapped
        // lines (10 + 10 + 5).
        assert_eq!(wrapped_line_count("aaaaaaaaaaaaaaaaaaaaaaaaa", 10), 3);
    }

    #[test]
    fn moderate_word_width_does_not_undercount_lines() {
        // Regression guard: a naive `ceil(total_display_width / width)`
        // estimate undercounts here (predicts 2 lines for 20 display
        // columns of content in width 10), but greedy word-wrap actually
        // needs 3 lines because "aaaaaa bbbbbb" (13 cols) doesn't fit on
        // one width-10 line — undercounting would hide real content.
        assert_eq!(wrapped_line_count("aaaaaa bbbbbb cccccc", 10), 3);
    }
}
