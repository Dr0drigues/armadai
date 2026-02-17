use std::path::Path;

use dialoguer::{Confirm, Input, Select};

use crate::core::config::AppPaths;
use crate::providers::factory::api_backend_for_tool;

pub async fn execute(
    name: Option<String>,
    template: String,
    stack: Option<String>,
    description: Option<String>,
    interactive: bool,
) -> anyhow::Result<()> {
    if interactive {
        return interactive_create().await;
    }

    let name =
        name.ok_or_else(|| anyhow::anyhow!("Agent name is required (or use --interactive)"))?;
    template_create(&name, &template, stack.as_deref(), description.as_deref())
}

/// Classic template-based creation (non-interactive)
fn template_create(
    name: &str,
    template: &str,
    stack: Option<&str>,
    description: Option<&str>,
) -> anyhow::Result<()> {
    let paths = AppPaths::resolve();
    let templates_dir = &paths.templates_dir;
    let agents_dir = &paths.agents_dir;

    // Load template
    let template_path = templates_dir.join(format!("{template}.md"));
    if !template_path.exists() {
        eprintln!("Template '{template}' not found.\n");
        list_templates(templates_dir)?;
        anyhow::bail!("Use --template <name> with one of the templates above");
    }

    // Check for duplicate
    let output_path = agents_dir.join(format!("{name}.md"));
    if output_path.exists() {
        anyhow::bail!(
            "Agent '{}' already exists at {}",
            name,
            output_path.display()
        );
    }

    // Read and replace placeholders
    let mut content = std::fs::read_to_string(&template_path)?;

    let title = slug_to_title(name);
    content = content.replace("{{name}}", &title);

    if let Some(desc) = description {
        content = content.replace("{{description}}", desc);
    }
    if let Some(s) = stack {
        content = content.replace("{{stack}}", s);
    }

    // Check for remaining placeholders
    let remaining: Vec<&str> = content
        .match_indices("{{")
        .filter_map(|(start, _)| {
            content[start..]
                .find("}}")
                .map(|end| &content[start..start + end + 2])
        })
        .collect();

    // Ensure agents directory exists
    std::fs::create_dir_all(agents_dir)?;

    // Write the agent file
    std::fs::write(&output_path, &content)?;

    println!("Agent created: {}", output_path.display());
    println!("  Template: {template}");
    println!("  Name: {title}");

    if !remaining.is_empty() {
        let unique: Vec<&str> = {
            let mut seen = std::collections::HashSet::new();
            remaining.into_iter().filter(|p| seen.insert(*p)).collect()
        };
        println!("\n  Note: the following placeholders need to be filled in manually:");
        for p in &unique {
            println!("    - {p}");
        }
    }

    Ok(())
}

/// Interactive agent creation wizard
async fn interactive_create() -> anyhow::Result<()> {
    let paths = AppPaths::resolve();
    let templates_dir = &paths.templates_dir;
    let agents_dir = &paths.agents_dir;

    println!("🧙 ArmadAI Agent Creation Wizard\n");

    // 1. Agent name
    let name: String = Input::new()
        .with_prompt("Agent name (slug, e.g. my-reviewer)")
        .validate_with(|input: &String| -> Result<(), String> {
            if input.is_empty() {
                return Err("Name cannot be empty".into());
            }
            if !input
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err("Use only letters, digits, hyphens, underscores".into());
            }
            Ok(())
        })
        .interact_text()?;

    // Check for duplicate
    let output_path = agents_dir.join(format!("{name}.md"));
    if output_path.exists() {
        anyhow::bail!("Agent '{name}' already exists at {}", output_path.display());
    }

    // 2. Template selection
    let mut template_names = collect_template_names(templates_dir);
    template_names.insert(0, "from scratch".to_string());

    let template_idx = Select::new()
        .with_prompt("Template")
        .items(&template_names)
        .default(0)
        .interact()?;

    let from_scratch = template_idx == 0;
    let chosen_template = if from_scratch {
        None
    } else {
        Some(&template_names[template_idx])
    };

    // 3. Provider
    let providers = [
        "claude",
        "gemini",
        "gpt",
        "aider",
        "anthropic",
        "openai",
        "google",
        "cli",
        "proxy",
    ];
    let provider_idx = Select::new()
        .with_prompt("Provider")
        .items(providers)
        .default(0)
        .interact()?;
    let provider = providers[provider_idx];

    // 4. Model (filtered by provider)
    let model = prompt_model(provider).await?;

    // 5. Temperature
    let temp_presets = [
        "Focused (0.2)",
        "Balanced (0.5)",
        "Creative (0.7)",
        "Custom",
    ];
    let temp_idx = Select::new()
        .with_prompt("Temperature")
        .items(temp_presets)
        .default(1)
        .interact()?;
    let temperature: f32 = match temp_idx {
        0 => 0.2,
        1 => 0.5,
        2 => 0.7,
        _ => Input::new()
            .with_prompt("Temperature (0.0 - 2.0)")
            .default(0.5)
            .validate_with(|input: &f32| {
                if (0.0..=2.0).contains(input) {
                    Ok(())
                } else {
                    Err("Must be between 0.0 and 2.0")
                }
            })
            .interact_text()?,
    };

    // 6. Max tokens
    let set_max_tokens = Confirm::new()
        .with_prompt("Set max tokens?")
        .default(false)
        .interact()?;
    let max_tokens: Option<u32> = if set_max_tokens {
        Some(
            Input::new()
                .with_prompt("Max tokens")
                .default(4096u32)
                .interact_text()?,
        )
    } else {
        None
    };

    // 7. Tags
    let tags_input: String = Input::new()
        .with_prompt("Tags (comma-separated, e.g. dev,review)")
        .default("general".to_string())
        .interact_text()?;
    let tags = parse_comma_list(&tags_input);

    // 8. Stacks
    let stacks_input: String = Input::new()
        .with_prompt("Stacks (comma-separated, e.g. rust,python)")
        .allow_empty(true)
        .interact_text()?;
    let stacks = parse_comma_list(&stacks_input);

    // 9. System prompt
    let system_prompt: String = Input::new()
        .with_prompt("System prompt")
        .default("You are a helpful assistant.".to_string())
        .interact_text()?;

    // 10. Instructions (optional)
    let add_instructions = Confirm::new()
        .with_prompt("Add instructions?")
        .default(false)
        .interact()?;
    let instructions = if add_instructions {
        let text: String = Input::new().with_prompt("Instructions").interact_text()?;
        Some(text)
    } else {
        None
    };

    // 11. Output format (optional)
    let add_output_format = Confirm::new()
        .with_prompt("Add output format?")
        .default(false)
        .interact()?;
    let output_format = if add_output_format {
        let text: String = Input::new().with_prompt("Output format").interact_text()?;
        Some(text)
    } else {
        None
    };

    // Generate content
    let content = if let Some(tpl_name) = chosen_template {
        let template_path = templates_dir.join(format!("{tpl_name}.md"));
        let mut content = std::fs::read_to_string(&template_path)?;
        let title = slug_to_title(&name);
        content = content.replace("{{name}}", &title);
        content = content.replace("{{description}}", &system_prompt);
        // Replace stacks placeholder
        if !stacks.is_empty() {
            content = content.replace("{{stack}}", &stacks[0]);
        }
        // Overwrite metadata section with user choices
        overwrite_metadata(
            &content,
            provider,
            model.as_deref(),
            temperature,
            max_tokens,
            &tags,
            &stacks,
        )
    } else {
        generate_from_scratch(&AgentParams {
            name: &name,
            provider,
            model: model.as_deref(),
            temperature,
            max_tokens,
            tags: &tags,
            stacks: &stacks,
            system_prompt: &system_prompt,
            instructions: instructions.as_deref(),
            output_format: output_format.as_deref(),
        })
    };

    // Write
    std::fs::create_dir_all(agents_dir)?;
    std::fs::write(&output_path, &content)?;

    println!("\nAgent created: {}", output_path.display());
    println!("  Name: {}", slug_to_title(&name));
    println!("  Provider: {provider}");
    if let Some(ref m) = model {
        println!("  Model: {m}");
    }

    Ok(())
}

/// Prompt for model based on provider.
/// Tries models.dev registry first (with cache), falls back to providers.yaml.
async fn prompt_model(provider: &str) -> anyhow::Result<Option<String>> {
    // CLI provider doesn't need a model
    if provider == "cli" {
        return Ok(None);
    }

    let backend = api_backend_for_tool(provider).unwrap_or(provider);

    // Try models.dev registry (online fetch with cache)
    #[cfg(feature = "providers-api")]
    if let Some(entries) = crate::model_registry::fetch::load_models_online(backend).await
        && !entries.is_empty()
    {
        return prompt_from_registry_entries(&entries);
    }

    // Try models.dev cache only (no network)
    #[cfg(not(feature = "providers-api"))]
    if let Some(entries) = crate::model_registry::fetch::load_models(backend)
        && !entries.is_empty()
    {
        return prompt_from_registry_entries(&entries);
    }

    // Fallback to providers.yaml
    let models = load_provider_models(backend);
    if models.is_empty() {
        let model: String = Input::new()
            .with_prompt("Model name")
            .allow_empty(true)
            .interact_text()?;
        return Ok(if model.is_empty() { None } else { Some(model) });
    }

    let mut items: Vec<String> = models;
    items.push("(custom)".to_string());

    let idx = Select::new()
        .with_prompt("Model")
        .items(&items)
        .default(0)
        .interact()?;

    if idx == items.len() - 1 {
        let model: String = Input::new()
            .with_prompt("Custom model name")
            .interact_text()?;
        Ok(Some(model))
    } else {
        Ok(Some(items[idx].clone()))
    }
}

/// Display models from the registry with enriched labels (context window, cost).
fn prompt_from_registry_entries(
    entries: &[crate::model_registry::ModelEntry],
) -> anyhow::Result<Option<String>> {
    let labels: Vec<String> = entries.iter().map(|e| e.display_label()).collect();
    let mut items = labels;
    items.push("(custom)".to_string());

    let idx = Select::new()
        .with_prompt("Model")
        .items(&items)
        .default(0)
        .interact()?;

    if idx == items.len() - 1 {
        let model: String = Input::new()
            .with_prompt("Custom model name")
            .interact_text()?;
        Ok(Some(model))
    } else {
        Ok(Some(entries[idx].id.clone()))
    }
}

/// Load model list for a given API backend from providers config.
fn load_provider_models(backend: &str) -> Vec<String> {
    let cfg = crate::core::config::load_providers_config();
    cfg.providers
        .get(backend)
        .map(|p| p.models.clone())
        .unwrap_or_default()
}

/// Overwrite the ## Metadata section in a template with user choices.
fn overwrite_metadata(
    content: &str,
    provider: &str,
    model: Option<&str>,
    temperature: f32,
    max_tokens: Option<u32>,
    tags: &[String],
    stacks: &[String],
) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut in_metadata = false;
    let mut metadata_written = false;

    for line in content.lines() {
        if line.starts_with("## Metadata") {
            in_metadata = true;
            lines.push(line.to_string());
            // Write new metadata
            lines.push(format!("- provider: {provider}"));
            if let Some(m) = model {
                lines.push(format!("- model: {m}"));
            }
            lines.push(format!("- temperature: {temperature}"));
            if let Some(mt) = max_tokens {
                lines.push(format!("- max_tokens: {mt}"));
            }
            if !tags.is_empty() {
                lines.push(format!("- tags: [{}]", tags.join(", ")));
            }
            if !stacks.is_empty() {
                lines.push(format!("- stacks: [{}]", stacks.join(", ")));
            }
            metadata_written = true;
            continue;
        }

        if in_metadata {
            // Skip old metadata lines until next section or blank line before section
            if line.starts_with("## ") {
                in_metadata = false;
                lines.push(String::new());
                lines.push(line.to_string());
            }
            // Skip metadata lines (starting with -)
            continue;
        }

        lines.push(line.to_string());
    }

    if !metadata_written {
        // Fallback: just return original
        return content.to_string();
    }

    lines.join("\n") + "\n"
}

/// Parameters for generating an agent from scratch.
pub struct AgentParams<'a> {
    pub name: &'a str,
    pub provider: &'a str,
    pub model: Option<&'a str>,
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub tags: &'a [String],
    pub stacks: &'a [String],
    pub system_prompt: &'a str,
    pub instructions: Option<&'a str>,
    pub output_format: Option<&'a str>,
}

/// Generate a complete agent markdown file from scratch.
pub fn generate_from_scratch(params: &AgentParams<'_>) -> String {
    let title = slug_to_title(params.name);
    let mut md = format!("# {title}\n\n## Metadata\n");

    md.push_str(&format!("- provider: {}\n", params.provider));
    if let Some(m) = params.model {
        md.push_str(&format!("- model: {m}\n"));
    }
    md.push_str(&format!("- temperature: {}\n", params.temperature));
    if let Some(mt) = params.max_tokens {
        md.push_str(&format!("- max_tokens: {mt}\n"));
    }
    if !params.tags.is_empty() {
        md.push_str(&format!("- tags: [{}]\n", params.tags.join(", ")));
    }
    if !params.stacks.is_empty() {
        md.push_str(&format!("- stacks: [{}]\n", params.stacks.join(", ")));
    }

    md.push_str(&format!("\n## System Prompt\n\n{}\n", params.system_prompt));

    if let Some(inst) = params.instructions {
        md.push_str(&format!("\n## Instructions\n\n{inst}\n"));
    }

    if let Some(fmt) = params.output_format {
        md.push_str(&format!("\n## Output Format\n\n{fmt}\n"));
    }

    md
}

/// Convert a slug like "my-agent" to title case "My Agent".
pub fn slug_to_title(slug: &str) -> String {
    slug.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse a comma-separated string into a list of trimmed, non-empty strings.
pub fn parse_comma_list(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Clap value parser that provides completion for available template names.
pub fn template_value_parser() -> clap::builder::PossibleValuesParser {
    let paths = crate::core::config::AppPaths::resolve();
    let names = collect_template_names(&paths.templates_dir);
    // Leak strings to get 'static references needed by clap's PossibleValuesParser.
    // This is called once at startup, so the leak is negligible.
    let leaked: Vec<&'static str> = names
        .into_iter()
        .map(|s| &*Box::leak(s.into_boxed_str()))
        .collect();
    clap::builder::PossibleValuesParser::new(leaked)
}

/// Collect template names (stems) from the templates directory.
fn collect_template_names(templates_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(templates_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .filter_map(|e| {
            e.path()
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
        })
        .collect();
    names.sort();
    names
}

fn list_templates(templates_dir: &Path) -> anyhow::Result<()> {
    println!("Available templates:");
    if !templates_dir.exists() {
        println!("  (no templates directory found)");
        return Ok(());
    }

    let mut found = false;
    for entry in std::fs::read_dir(templates_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "md")
            && let Some(stem) = path.file_stem()
        {
            println!("  - {}", stem.to_string_lossy());
            found = true;
        }
    }

    if !found {
        println!("  (no templates found)");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn new_basic_agent() {
        let dir = tempfile::tempdir().unwrap();
        let templates = dir.path().join("templates");
        let agents = dir.path().join("agents");
        fs::create_dir_all(&templates).unwrap();

        fs::write(
            templates.join("basic.md"),
            "# {{name}}\n\n## Metadata\n- provider: anthropic\n\n## System Prompt\n\n{{description}}\n",
        )
        .unwrap();

        // We need to work from the temp dir, so test the logic directly
        let template_path = templates.join("basic.md");
        let output_path = agents.join("my-agent.md");
        let mut content = fs::read_to_string(&template_path).unwrap();
        content = content.replace("{{name}}", "My Agent");
        content = content.replace("{{description}}", "A test agent");
        fs::create_dir_all(&agents).unwrap();
        fs::write(&output_path, &content).unwrap();

        let result = fs::read_to_string(&output_path).unwrap();
        assert!(result.contains("# My Agent"));
        assert!(result.contains("A test agent"));
        assert!(!result.contains("{{name}}"));
    }

    #[test]
    fn test_generate_from_scratch() {
        let tags = vec!["dev".to_string(), "test".to_string()];
        let stacks = vec!["rust".to_string()];
        let md = generate_from_scratch(&AgentParams {
            name: "test-agent",
            provider: "claude",
            model: Some("claude-sonnet-4-5-20250929"),
            temperature: 0.5,
            max_tokens: Some(4096),
            tags: &tags,
            stacks: &stacks,
            system_prompt: "You are a test assistant.",
            instructions: Some("1. Read code\n2. Write tests"),
            output_format: Some("Markdown with code blocks"),
        });

        assert!(md.contains("# Test Agent"));
        assert!(md.contains("- provider: claude"));
        assert!(md.contains("- model: claude-sonnet-4-5-20250929"));
        assert!(md.contains("- temperature: 0.5"));
        assert!(md.contains("- max_tokens: 4096"));
        assert!(md.contains("- tags: [dev, test]"));
        assert!(md.contains("- stacks: [rust]"));
        assert!(md.contains("## System Prompt"));
        assert!(md.contains("You are a test assistant."));
        assert!(md.contains("## Instructions"));
        assert!(md.contains("## Output Format"));

        // Verify parseable by the parser
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test-agent.md");
        std::fs::write(&path, &md).unwrap();
        let agent = crate::parser::parse_agent_file(&path).unwrap();
        assert_eq!(agent.name, "Test Agent");
        assert_eq!(agent.metadata.provider, "claude");
    }

    #[test]
    fn test_parse_comma_list() {
        assert_eq!(parse_comma_list("a, b, c"), vec!["a", "b", "c"]);
        assert_eq!(parse_comma_list("single"), vec!["single"]);
        assert_eq!(parse_comma_list(""), Vec::<String>::new());
        assert_eq!(parse_comma_list(" a , , b "), vec!["a", "b"]);
    }

    #[test]
    fn test_slug_to_title() {
        assert_eq!(slug_to_title("my-agent"), "My Agent");
        assert_eq!(slug_to_title("code-reviewer"), "Code Reviewer");
        assert_eq!(slug_to_title("simple"), "Simple");
    }
}
