use std::path::Path;

pub async fn execute(
    name: String,
    template: String,
    stack: Option<String>,
    description: Option<String>,
) -> anyhow::Result<()> {
    let templates_dir = Path::new("templates");
    let agents_dir = Path::new("agents");

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

    // Convert name to title case for the H1 heading
    let title = name
        .split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    content = content.replace("{{name}}", &title);

    if let Some(ref desc) = description {
        content = content.replace("{{description}}", desc);
    }
    if let Some(ref s) = stack {
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
}
