//! Shared numeric formatting helpers for the dashboard TUI.
//!
//! Centralizes cost and context-window formatting so every view (Costs,
//! History, Models list, Model detail, ...) displays the same precision and
//! unit conventions instead of ad-hoc `{:.N}` calls scattered across files.

/// Format a cost value in USD with a fixed 4-decimal precision, e.g. `$0.0000`.
pub fn format_cost(cost: f64) -> String {
    format!("${cost:.4}")
}

/// Format a token-count / context-window size with a `K`/`M` suffix.
///
/// - Values under 1,000 are shown as-is (e.g. `999`).
/// - Values under 1,000,000 are shown in thousands (e.g. `200000` -> `200K`).
/// - Values at or above 1,000,000 are shown in millions (e.g. `1500000` -> `1.5M`).
///
/// Fractional K/M values keep a single decimal place; whole values drop it.
pub fn format_context(value: u64) -> String {
    const THOUSAND: u64 = 1_000;
    const MILLION: u64 = 1_000_000;

    if value < THOUSAND {
        value.to_string()
    } else if value < MILLION {
        format_unit(value, THOUSAND, "K")
    } else {
        format_unit(value, MILLION, "M")
    }
}

fn format_unit(value: u64, divisor: u64, suffix: &str) -> String {
    let scaled = value as f64 / divisor as f64;
    if scaled.fract() == 0.0 {
        format!("{scaled:.0}{suffix}")
    } else {
        format!("{scaled:.1}{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_cost_zero() {
        assert_eq!(format_cost(0.0), "$0.0000");
    }

    #[test]
    fn format_cost_rounds_to_four_decimals() {
        assert_eq!(format_cost(0.00005), "$0.0001");
    }

    #[test]
    fn format_cost_typical_value() {
        assert_eq!(format_cost(1.5), "$1.5000");
    }

    #[test]
    fn format_context_below_thousand_is_raw() {
        assert_eq!(format_context(999), "999");
    }

    #[test]
    fn format_context_at_thousand_boundary() {
        assert_eq!(format_context(1000), "1K");
    }

    #[test]
    fn format_context_typical_k_value() {
        assert_eq!(format_context(200_000), "200K");
    }

    #[test]
    fn format_context_at_million_boundary() {
        assert_eq!(format_context(1_500_000), "1.5M");
    }

    #[test]
    fn format_context_fractional_million() {
        assert_eq!(format_context(1_000_000), "1M");
    }
}
