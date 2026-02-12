use super::cache::IndexEntry;

/// Search entries by keywords (AND logic).
///
/// Scores entries by relevance: name match > description match > tag match.
/// Returns results sorted by descending score.
pub fn search(entries: &[IndexEntry], query: &str) -> Vec<SearchResult> {
    let keywords: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();

    if keywords.is_empty() {
        return Vec::new();
    }

    let mut results: Vec<SearchResult> = entries
        .iter()
        .filter_map(|entry| {
            let score = score_entry(entry, &keywords);
            if score > 0 {
                Some(SearchResult {
                    entry: entry.clone(),
                    score,
                })
            } else {
                None
            }
        })
        .collect();

    results.sort_by(|a, b| b.score.cmp(&a.score));
    results
}

/// Filter entries by category.
pub fn filter_by_category<'a>(entries: &'a [IndexEntry], category: &str) -> Vec<&'a IndexEntry> {
    entries
        .iter()
        .filter(|e| e.category.as_deref() == Some(category))
        .collect()
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entry: IndexEntry,
    pub score: u32,
}

/// Score an entry against keywords. Returns 0 if any keyword doesn't match.
fn score_entry(entry: &IndexEntry, keywords: &[String]) -> u32 {
    let mut total = 0;

    for kw in keywords {
        let mut kw_score = 0u32;

        // Name match (highest weight)
        if entry.name.to_lowercase().contains(kw) {
            kw_score += 10;
        }

        // Description match
        if let Some(ref desc) = entry.description
            && desc.to_lowercase().contains(kw)
        {
            kw_score += 5;
        }

        // Tag match
        if entry.tags.iter().any(|t| t.to_lowercase().contains(kw)) {
            kw_score += 3;
        }

        // Category match
        if let Some(ref cat) = entry.category
            && cat.to_lowercase().contains(kw)
        {
            kw_score += 1;
        }

        // AND logic: all keywords must match somewhere
        if kw_score == 0 {
            return 0;
        }
        total += kw_score;
    }

    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entry(
        name: &str,
        desc: Option<&str>,
        tags: &[&str],
        category: Option<&str>,
    ) -> IndexEntry {
        IndexEntry {
            path: format!("{name}.md"),
            name: name.to_string(),
            description: desc.map(String::from),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            category: category.map(String::from),
        }
    }

    #[test]
    fn test_search_single_keyword() {
        let entries = vec![
            make_entry(
                "security-scanner",
                Some("OWASP vulnerability scanner"),
                &["security"],
                None,
            ),
            make_entry(
                "code-reviewer",
                Some("General code review"),
                &["review"],
                None,
            ),
        ];

        let results = search(&entries, "security");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.name, "security-scanner");
    }

    #[test]
    fn test_search_multiple_keywords_and() {
        let entries = vec![
            make_entry(
                "security-scanner",
                Some("OWASP vulnerability scanner"),
                &["security"],
                None,
            ),
            make_entry(
                "security-reviewer",
                Some("Code security review"),
                &["security", "review"],
                None,
            ),
        ];

        let results = search(&entries, "security review");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.name, "security-reviewer");
    }

    #[test]
    fn test_search_no_match() {
        let entries = vec![make_entry(
            "code-reviewer",
            Some("General code review"),
            &["review"],
            None,
        )];

        let results = search(&entries, "nonexistent");
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_ranking() {
        let entries = vec![
            make_entry("test-writer", Some("Writes tests"), &["testing"], None),
            make_entry("reviewer", Some("Reviews test coverage"), &["test"], None),
        ];

        let results = search(&entries, "test");
        assert_eq!(results.len(), 2);
        // "test-writer" should rank higher (name match = 10 + desc match = 5)
        assert_eq!(results[0].entry.name, "test-writer");
    }

    #[test]
    fn test_search_empty_query() {
        let entries = vec![make_entry("test", None, &[], None)];
        let results = search(&entries, "");
        assert!(results.is_empty());
    }

    #[test]
    fn test_filter_by_category() {
        let entries = vec![
            make_entry("a", None, &[], Some("official")),
            make_entry("b", None, &[], Some("community")),
            make_entry("c", None, &[], Some("official")),
        ];

        let filtered = filter_by_category(&entries, "official");
        assert_eq!(filtered.len(), 2);
    }
}
