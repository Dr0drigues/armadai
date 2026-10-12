/// Known tool definitions for unified provider names: a pure table mapping a
/// user-friendly tool name to the API vendor whose model catalogue it speaks.
struct ToolDef {
    /// API vendor backing this tool, or `None` for a CLI-only tool.
    ///
    /// `Option` rather than a required string because three of these tools
    /// (`codex`, `copilot`, `opencode`) have no API ArmadAI can reach with an
    /// agent's plain-text exchange: their vendors expose the agent behind the
    /// CLI itself. Naming an arbitrary backend for them would send the user
    /// looking for the wrong thing (issue #369).
    api_backend: Option<&'static str>,
}

const KNOWN_TOOLS: &[(&str, ToolDef)] = &[
    (
        "claude",
        ToolDef {
            api_backend: Some("anthropic"),
        },
    ),
    (
        "gemini",
        ToolDef {
            api_backend: Some("google"),
        },
    ),
    (
        "gpt",
        ToolDef {
            api_backend: Some("openai"),
        },
    ),
    (
        "aider",
        ToolDef {
            api_backend: Some("openai"),
        },
    ),
    // CLI-only: no API backend ArmadAI can reach.
    ("codex", ToolDef { api_backend: None }),
    ("copilot", ToolDef { api_backend: None }),
    ("opencode", ToolDef { api_backend: None }),
];

/// API vendor names, which map to themselves.
pub const API_PROVIDER_NAMES: &[&str] = &["anthropic", "openai", "google", "proxy"];

fn find_tool(name: &str) -> Option<&'static ToolDef> {
    KNOWN_TOOLS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, def)| def)
}

/// Map a unified tool name to its API backend name.
/// Returns the backend directly for explicit API providers.
/// e.g. "claude" → "anthropic", "gemini" → "google", "anthropic" → "anthropic"
pub fn api_backend_for_tool(name: &str) -> Option<&'static str> {
    API_PROVIDER_NAMES
        .iter()
        .find(|n| **n == name)
        .copied()
        .or_else(|| find_tool(name).and_then(|t| t.api_backend))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_known_tools() {
        assert!(find_tool("claude").is_some());
        assert!(find_tool("gemini").is_some());
        assert!(find_tool("gpt").is_some());
        assert!(find_tool("aider").is_some());
        assert!(find_tool("unknown").is_none());
    }

    #[test]
    fn api_backend_for_tool_maps_tools_and_vendors() {
        for (name, expected) in [
            ("anthropic", Some("anthropic")),
            ("openai", Some("openai")),
            ("google", Some("google")),
            ("proxy", Some("proxy")),
            ("claude", Some("anthropic")),
            ("gemini", Some("google")),
            ("gpt", Some("openai")),
            ("aider", Some("openai")),
            ("cli", None),
            ("unknown-tool", None),
        ] {
            assert_eq!(api_backend_for_tool(name), expected, "{name}");
        }
    }

    /// CLI-only tools bill through their own subscription: they have no API
    /// backend (issue #369).
    #[test]
    fn cli_only_tools_have_no_api_backend() {
        for tool in ["codex", "copilot", "opencode"] {
            assert!(find_tool(tool).is_some(), "{tool}");
            assert_eq!(api_backend_for_tool(tool), None, "{tool}");
        }
    }
}
