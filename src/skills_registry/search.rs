use super::cache::SkillIndexEntry;

/// Search skill entries by keywords (AND logic).
///
/// Scores entries by relevance: name > description > tags > source repo.
/// Returns results sorted by descending score.
pub fn search(entries: &[SkillIndexEntry], query: &str) -> Vec<SearchResult> {
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

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entry: SkillIndexEntry,
    pub score: u32,
}

/// Score an entry against keywords. Returns 0 if any keyword doesn't match.
fn score_entry(entry: &SkillIndexEntry, keywords: &[String]) -> u32 {
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

        // Source repo match
        if entry.source_repo.to_lowercase().contains(kw) {
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

    fn make_entry(name: &str, desc: Option<&str>, tags: &[&str], source: &str) -> SkillIndexEntry {
        SkillIndexEntry {
            name: name.to_string(),
            description: desc.map(String::from),
            source_repo: source.to_string(),
            path: format!("skills/{name}"),
            tags: tags.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn test_search_single_keyword() {
        let entries = vec![
            make_entry(
                "webapp-testing",
                Some("Test web applications"),
                &["playwright"],
                "anthropics/skills",
            ),
            make_entry(
                "docker-compose",
                Some("Manage Docker Compose"),
                &["docker"],
                "anthropics/skills",
            ),
        ];

        let results = search(&entries, "testing");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.name, "webapp-testing");
    }

    #[test]
    fn test_search_multiple_keywords_and() {
        let entries = vec![
            make_entry(
                "webapp-testing",
                Some("Test web applications with Playwright"),
                &["playwright", "testing"],
                "anthropics/skills",
            ),
            make_entry(
                "unit-testing",
                Some("Unit test framework"),
                &["jest"],
                "anthropics/skills",
            ),
        ];

        let results = search(&entries, "testing playwright");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.name, "webapp-testing");
    }

    #[test]
    fn test_search_no_match() {
        let entries = vec![make_entry(
            "docker-compose",
            Some("Docker management"),
            &["docker"],
            "anthropics/skills",
        )];

        let results = search(&entries, "nonexistent");
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_ranking() {
        let entries = vec![
            make_entry(
                "test-runner",
                Some("Run tests"),
                &["testing"],
                "anthropics/skills",
            ),
            make_entry(
                "linter",
                Some("Lint and test code"),
                &["test"],
                "anthropics/skills",
            ),
        ];

        let results = search(&entries, "test");
        assert_eq!(results.len(), 2);
        // "test-runner" should rank higher (name match = 10 + desc match = 5)
        assert_eq!(results[0].entry.name, "test-runner");
    }

    #[test]
    fn test_search_empty_query() {
        let entries = vec![make_entry("test", None, &[], "anthropics/skills")];
        let results = search(&entries, "");
        assert!(results.is_empty());
    }
}
