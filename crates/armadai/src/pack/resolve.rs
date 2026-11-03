//! Version ranges of `pack.version`, resolved against the cached versions of a pack.

use super::cache::Available;

#[derive(Debug, Clone)]
pub enum Range {
    Latest,
    Exact(semver::Version),
    Req(semver::VersionReq),
}

/// `latest`; a string that parses as a whole version is exact (`1.2.3` means `=1.2.3`,
/// never `^1.2.3` as `VersionReq` would read it); anything else a `VersionReq` (`^1.3`, `1`).
pub fn parse_range(s: &str) -> Result<Range, String> {
    let s = s.trim();
    if s == "latest" {
        return Ok(Range::Latest);
    }
    if let Ok(v) = semver::Version::parse(s) {
        return Ok(Range::Exact(v));
    }
    semver::VersionReq::parse(s)
        .map(Range::Req)
        .map_err(|e| format!("`{s}` is not `latest`, a version or a version range: {e}"))
}

/// The highest version `range` accepts; a pre-release only for an exact range.
pub fn best<'a>(range: &Range, available: &'a [Available]) -> Option<&'a Available> {
    available
        .iter()
        .filter_map(|a| semver::Version::parse(&a.version).ok().map(|v| (v, a)))
        .filter(|(v, _)| match range {
            Range::Exact(want) => v == want,
            Range::Latest => v.pre.is_empty(),
            Range::Req(req) => v.pre.is_empty() && req.matches(v),
        })
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, a)| a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn avail(vs: &[&str]) -> Vec<Available> {
        vs.iter()
            .map(|v| Available {
                version: v.to_string(),
                tag: None,
                commit: Some(format!("c-{v}")),
                digest: None,
            })
            .collect()
    }

    fn pick(range: &str, vs: &[Available]) -> Option<String> {
        best(&parse_range(range).unwrap(), vs).map(|a| a.version.clone())
    }

    #[test]
    fn exact_means_exact() {
        let vs = avail(&["1.2.3", "1.2.9"]);
        assert_eq!(pick("1.2.3", &vs).as_deref(), Some("1.2.3"));
    }

    #[test]
    fn ranges_and_latest() {
        let vs = avail(&["1.2.0", "1.2.5", "1.3.1", "1.9.0", "2.0.0", "2.1.0-rc.1"]);
        assert_eq!(pick("^1.3", &vs).as_deref(), Some("1.9.0"));
        assert_eq!(pick("~1.2", &vs).as_deref(), Some("1.2.5"));
        assert_eq!(pick("1", &vs).as_deref(), Some("1.9.0"));
        assert_eq!(pick("latest", &vs).as_deref(), Some("2.0.0"));
    }

    #[test]
    fn a_pre_release_is_picked_only_when_exact() {
        let vs = avail(&["2.0.0", "2.1.0-rc.1"]);
        assert_eq!(pick("2.1.0-rc.1", &vs).as_deref(), Some("2.1.0-rc.1"));
    }

    #[test]
    fn no_match_is_none() {
        let vs = avail(&["1.2.0", "2.0.0"]);
        assert_eq!(pick("^3", &vs), None);
        assert!(parse_range("not a range").is_err());
    }
}
