//! `armadai graph`: one flow, every flow, or the team's delegations, as Mermaid, DOT or JSON.
//!
//! stdout carries the drawing only (pipe it into a file or a renderer); every
//! diagnostic and error goes to stderr. Several flows are several diagrams: one
//! block each, opened by `%% flow: <id>` (`// flow: <id>` in DOT) and separated by a
//! blank line (render them one at a time). In JSON they are one `{"flows": [...]}`.

use anyhow::bail;

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
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

pub fn execute(flow: Option<String>, format: GraphFormat, view: GraphView) -> anyhow::Result<()> {
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
