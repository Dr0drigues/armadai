//! The `[<flow>/<step>#<lot>]` marker a coordinator puts at the start of an
//! `Agent` call's `description`, so the run a delegation belongs to is read
//! from what the coordinator wrote, not guessed from the order of events.

/// Longest `lot` accepted, in characters.
pub const MAX_LOT: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub flow: String,
    pub step: String,
    pub lot: Option<String>,
}

/// A lot is a free name: 1 to `MAX_LOT` ASCII letters (either case), digits, `.` or `-`.
/// `Y2` and `y2` are two lots (no normalization).
pub fn is_valid_lot(l: &str) -> bool {
    (1..=MAX_LOT).contains(&l.len())
        && l.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))
}

/// `[<flow>/<step>]` or `[<flow>/<step>#<lot>]` at the very start of `description`;
/// flow and step are ids (`is_valid_id`), the lot is `is_valid_lot`.
pub fn parse(description: &str) -> Option<Marker> {
    let inner = description.strip_prefix('[')?.split_once(']')?.0;
    let (flow, rest) = inner.split_once('/')?;
    let (step, lot) = match rest.split_once('#') {
        Some((s, l)) => (s, Some(l)),
        None => (rest, None),
    };
    let id = armadai_model::is_valid_id;
    let lot_ok = lot.is_none_or(is_valid_lot);
    (id(flow) && id(step) && lot_ok).then(|| Marker {
        flow: flow.into(),
        step: step.into(),
        lot: lot.map(Into::into),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(flow: &str, step: &str, lot: Option<&str>) -> Option<Marker> {
        Some(Marker {
            flow: flow.into(),
            step: step.into(),
            lot: lot.map(Into::into),
        })
    }

    #[test]
    fn a_marker_with_and_without_a_lot_parses() {
        assert_eq!(
            parse("[evolution/review#menu] relire"),
            m("evolution", "review", Some("menu"))
        );
        assert_eq!(parse("[task/build-cli] go"), m("task", "build-cli", None));
        assert_eq!(parse("[a/b]"), m("a", "b", None));
    }

    #[test]
    fn a_lot_is_a_free_name_with_case() {
        for l in ["Y2", "GA1", "v1.8.0", "cc1-sm1", "y2"] {
            assert_eq!(
                parse(&format!("[mr/revue-mr#{l}] x")),
                m("mr", "revue-mr", Some(l)),
                "{l}"
            );
        }
    }

    #[test]
    fn a_lot_of_40_characters_passes_and_41_does_not() {
        let l40 = "a".repeat(40);
        let l41 = "a".repeat(41);
        assert_eq!(
            parse(&format!("[f/s#{l40}] x")),
            m("f", "s", Some(l40.as_str()))
        );
        assert_eq!(parse(&format!("[f/s#{l41}] x")), None);
    }

    #[test]
    fn anything_but_an_id_or_the_exact_shape_is_no_marker() {
        for bad in [
            "[Flow/step]",
            "[flow/Step]",
            "[fl_ow/step]",
            "[flow/st_ep]",
            "[flow/step#a_b]",
            "[flow/step#a+b]",
            " [flow/step]",
            "x [flow/step]",
            "[flow/step",
            "[flow]",
            "[/step]",
            "[flow/]",
            "[flow/step#]",
            "[flow/step/extra]",
            "",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }
}
