//! `armadai graph`: one flow, every flow, or the team's delegations, as Mermaid, DOT, JSON or ASCII.
//!
//! stdout carries the drawing only (pipe it into a file or a renderer); every
//! diagnostic and error goes to stderr. Several flows are several diagrams: one
//! block each, opened by `%% flow: <id>` (`// flow: <id>` in DOT) and separated by a
//! blank line (render them one at a time). In JSON they are one `{"flows": [...]}`.

use anyhow::bail;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum GraphFormat {
    #[default]
    Mermaid,
    Dot,
    Json,
    Ascii,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum GraphView {
    #[default]
    Flows,
    Team,
}

/// `https://mermaid.live/edit#pako:<payload>`: the editor's own serde, a zlib stream
/// (level 9) of the minimal state, in URL-safe base64 without padding.
fn mermaid_live_url(code: &str) -> String {
    use base64::Engine as _;
    let state =
        serde_json::json!({"code": code, "mermaid": "{}", "updateDiagram": true, "rough": false});
    let zipped = miniz_oxide::deflate::compress_to_vec_zlib(state.to_string().as_bytes(), 9);
    format!(
        "https://mermaid.live/edit#pako:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(zipped)
    )
}

/// The platform's opener, waited for. Failure carries the URL so the user can open it.
fn open_url(url: &str) -> anyhow::Result<()> {
    let (cmd, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(windows) {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    let why = match std::process::Command::new(cmd).args(&args).status() {
        Ok(s) if s.success() => return Ok(()),
        Ok(s) => s.to_string(),
        Err(e) => e.to_string(),
    };
    bail!("could not open the browser ({cmd}: {why}); open this URL yourself:\n{url}")
}

pub fn execute(
    flow: Option<String>,
    format: GraphFormat,
    view: GraphView,
    open: bool,
) -> anyhow::Result<()> {
    if open && format != GraphFormat::Mermaid {
        use clap::ValueEnum as _;
        let name = format
            .to_possible_value()
            .map_or_else(String::new, |v| v.get_name().to_string());
        bail!("--open draws Mermaid only; drop --format {name}");
    }
    if let (GraphView::Team, Some(id)) = (view, &flow) {
        bail!("--view team draws the whole team: drop the flow id {id}");
    }
    let root = crate::compile::find_project(&std::env::current_dir()?)?;
    let project = match armadai_model::load_checked(&root) {
        Ok((p, _)) => p,
        Err(ds) => {
            for d in &ds {
                eprintln!("{}", d.render());
            }
            bail!("the project is invalid; run `armadai validate`")
        }
    };
    if view == GraphView::Team {
        let g = armadai_model::team_graph(&project);
        if open {
            return open_url(&mermaid_live_url(&armadai_model::team_to_mermaid(&g)));
        }
        print!(
            "{}",
            match format {
                GraphFormat::Mermaid => armadai_model::team_to_mermaid(&g),
                GraphFormat::Dot => armadai_model::team_to_dot(&g),
                GraphFormat::Json => format!("{}\n", serde_json::to_string_pretty(&g)?),
                GraphFormat::Ascii => crate::ascii_graph::team_to_ascii(&g),
            }
        );
        return Ok(());
    }
    let graphs: Vec<armadai_model::FlowGraph> = match flow {
        Some(id) => match project.flows.get(&id) {
            Some(f) => vec![armadai_model::flow_graph(&id, f)],
            None if project.flows.is_empty() => {
                bail!("unknown flow `{id}`: the project has no flow")
            }
            None => bail!(
                "unknown flow `{id}`; flows: {}",
                project.flows.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        },
        None => (project.flows.iter())
            .map(|(id, f)| armadai_model::flow_graph(id, f))
            .collect(),
    };
    if graphs.is_empty() {
        bail!("the project has no flow (.armadai/flows/ is empty)");
    }
    if open {
        if let [g] = graphs.as_slice() {
            return open_url(&mermaid_live_url(&armadai_model::flow_graph_to_mermaid(g)));
        }
        bail!(
            "--open draws one diagram: name a flow (flows: {})",
            project.flows.keys().cloned().collect::<Vec<_>>().join(", ")
        );
    }
    match format {
        GraphFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "flows": graphs }))?
        ),
        _ => {
            let blocks: Vec<String> = graphs
                .iter()
                .map(|g| match format {
                    GraphFormat::Dot => armadai_model::flow_graph_to_dot(g),
                    GraphFormat::Ascii => crate::ascii_graph::flow_to_ascii(g),
                    _ => armadai_model::flow_graph_to_mermaid(g),
                })
                .collect();
            print!("{}", blocks.join("\n"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    #[test]
    fn the_url_decodes_to_the_state() {
        // four lengths, so one of them needs base64 padding
        for pad in ["", "a", "ab", "abc"] {
            check(&format!("flowchart TD\n  a --> b?>~{pad}\n"));
        }
    }

    fn check(code: &str) {
        let url = mermaid_live_url(code);
        let payload = url.strip_prefix("https://mermaid.live/edit#pako:").unwrap();
        assert!(!payload.contains(['=', '+', '/']), "{payload}");
        let zipped = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload)
            .unwrap();
        let json = miniz_oxide::inflate::decompress_to_vec_zlib(&zipped).unwrap();
        let state: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(
            state,
            serde_json::json!({"code": code, "mermaid": "{}", "updateDiagram": true, "rough": false})
        );
    }
}
