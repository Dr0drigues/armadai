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
//! the user scroll past real content — so this counts leading/indentation
//! whitespace toward line width (ratatui's `trim: false` keeps it, it isn't
//! dropped) and, after a word gets hard-broken across multiple lines,
//! carries the remaining width of its last physical chunk forward so
//! following text can share that line, exactly as ratatui's own
//! `WordWrapper` does. Both behaviors were verified against real ratatui
//! 0.30.2 rendering (`Paragraph` + `Wrap { trim: false }` into a
//! `TestBackend` buffer) — see the tests below.

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
            // room. This also covers *leading* whitespace (indentation),
            // which `Wrap { trim: false }` renders and counts toward line
            // width instead of dropping — a `split(' ')` run of N leading
            // spaces yields N empty tokens before the first real word, so
            // counting them unconditionally reproduces that width exactly.
            //
            // Deliberately not setting `has_content = true` here: a run of
            // leading spaces has no preceding word to separate from, so the
            // first real word that follows must not get a phantom extra
            // separator column added on top of the spaces already counted.
            if current < width {
                current += 1;
            } else {
                // The line is already full of whitespace; ratatui starts a
                // fresh (empty) line rather than carrying anything over.
                lines += 1;
                current = 0;
            }
            continue;
        }

        let word_width = word.width();
        if word_width > width {
            // A single word wider than the line will be hard-broken by the
            // real renderer. We don't reproduce the exact split point, only
            // the resulting line count (see module doc) — except for the
            // *last* hard-wrapped chunk, which matters for what follows: it
            // isn't a fresh closed line, it's the still-open current line.
            // Ratatui reuses whatever width remains on it for the text that
            // comes next, instead of starting that text on a brand new
            // line. Losing this remaining-width state is what caused the
            // original undercount here.
            if has_content {
                lines += 1;
            }
            lines += word_width.div_ceil(width).saturating_sub(1);
            let remainder = word_width % width;
            current = if remainder == 0 { width } else { remainder };
            has_content = true;
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

    #[test]
    fn leading_indentation_counts_toward_line_width() {
        // Regression guard for a confirmed undercount: `Wrap { trim: false
        // }` renders leading whitespace (e.g. code-block/nested-list
        // indentation) as real columns instead of dropping it, so "    let
        // x = 5;" (4 leading spaces + 10 columns of text = 14 total) at
        // width 10 needs 2 physical lines, not 1. Verified against real
        // ratatui 0.30.2 (`Paragraph::new(text).wrap(Wrap { trim: false
        // })` rendered into a `TestBackend`, counting non-blank rows):
        // ratatui renders exactly 2 rows for this input at width 10. The
        // pre-fix implementation returned 1 here (dropped the 4 leading
        // spaces while `has_content` was still false), stranding scroll
        // content.
        assert!(
            wrapped_line_count("    let x = 5;", 10) >= 2,
            "must not undercount indented content (ratatui renders 2 lines here)"
        );
    }

    #[test]
    fn hard_break_remainder_shares_line_with_following_text() {
        // Regression guard for a confirmed undercount: once a word too wide
        // for the line gets hard-broken, ratatui's `WordWrapper` reuses the
        // remaining width of that word's last physical chunk for whatever
        // text follows, rather than starting that text on a fresh line.
        // Verified against real ratatui 0.30.2 the same way as above: this
        // 87-column unbreakable "URL" followed by " more text after", at
        // width 20, renders exactly 6 rows (the 87-wide token hard-breaks
        // into 4 full 20-wide rows plus a 7-wide remainder, and that
        // remainder row absorbs " more" before "text after" wraps onward).
        // The pre-fix implementation reset the running column count to 0
        // after the hard break (as if a brand new empty line had started),
        // which returned 5 here instead of 6 — an undercount that would
        // strand the last visible line of content.
        assert!(
            wrapped_line_count(
                "https://example.com/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa more text after",
                20
            ) >= 6,
            "must not undercount text following a hard-broken word (ratatui renders 6 lines here)"
        );
    }
}
