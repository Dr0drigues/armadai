//! What `migrate` did, dropped and wants the user to know.

use std::path::PathBuf;

/// A legacy field with no counterpart in the new format.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Dropped {
    pub subject: String,
    pub field: String,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct Report {
    /// Paths relative to the project root.
    pub written: Vec<PathBuf>,
    pub dropped: Vec<Dropped>,
    pub warnings: Vec<String>,
    /// Where the legacy project was copied (relative to the root), once it was.
    pub backup: Option<PathBuf>,
}

impl Report {
    pub fn drop(&mut self, subject: &str, field: &str, reason: impl Into<String>) {
        self.dropped.push(Dropped {
            subject: subject.to_string(),
            field: field.to_string(),
            reason: reason.into(),
        });
    }

    pub fn warn(&mut self, w: impl Into<String>) {
        self.warnings.push(w.into());
    }

    /// Sections `Written`, `Dropped`, `Warnings`, each sorted: same input, same text.
    pub fn render(&self) -> String {
        let mut written: Vec<String> = self
            .written
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        written.sort();
        // one line for the field every legacy agent has
        let providers = self
            .dropped
            .iter()
            .filter(|d| d.field == "provider")
            .count();
        let mut dropped: Vec<String> = self
            .dropped
            .iter()
            .filter(|d| d.field != "provider")
            .map(|d| format!("{}: {} ({})", d.subject, d.field, d.reason))
            .collect();
        if providers > 0 {
            let reason = (self.dropped.iter())
                .find(|d| d.field == "provider")
                .map_or("", |d| d.reason.as_str());
            let s = if providers > 1 { "s" } else { "" };
            dropped.push(format!(
                "provider: dropped for {providers} agent{s} ({reason})"
            ));
        }
        dropped.sort();
        dropped.dedup();
        let mut warnings = self.warnings.clone();
        warnings.sort();
        warnings.dedup();

        let mut out = String::new();
        for (title, lines) in [
            ("Written", written),
            ("Dropped", dropped),
            ("Warnings", warnings),
        ] {
            out.push_str(&format!("{title} ({}):\n", lines.len()));
            for l in lines {
                out.push_str(&format!("  {l}\n"));
            }
        }
        if let Some(b) = &self.backup {
            out.push_str(&format!(
                "Note: 'armadai link' reads the new format from step 2 of the v1 work; \
                 the previous project is in {}/.\n",
                b.display()
            ));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_is_sorted_and_deterministic() {
        let mut a = Report {
            written: vec!["b".into(), "a".into()],
            ..Default::default()
        };
        a.drop("z", "provider", "r");
        a.drop("a", "timeout", "r");
        a.warn("w2");
        a.warn("w1");
        let mut b = Report {
            written: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        b.drop("a", "timeout", "r");
        b.drop("z", "provider", "r");
        b.warn("w1");
        b.warn("w2");
        assert_eq!(a.render(), b.render());
        assert_eq!(
            a.render(),
            "Written (2):\n  a\n  b\nDropped (2):\n  a: timeout (r)\n  provider: dropped for 1 agent (r)\n\
             Warnings (2):\n  w1\n  w2\n"
        );
    }
}
