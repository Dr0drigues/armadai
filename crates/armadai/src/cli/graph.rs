//! `armadai graph`: the Mermaid of one flow, or of every flow.
//!
//! stdout carries the Mermaid only (pipe it into a file or a renderer); every
//! diagnostic and error goes to stderr. Several flows are several diagrams: one
//! `flowchart` block each, opened by `%% flow: <id>` and separated by a blank line
//! (render them one at a time: Mermaid reads one diagram per document).

use anyhow::bail;

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum GraphFormat {
    #[default]
    Mermaid,
    /// Markdown: a heading and a mermaid block per flow
    Md,
}

fn markdown_block(id: &str, mermaid: &str) -> String {
    format!("## {id}\n\n```mermaid\n{mermaid}```\n")
}

pub fn execute(flow: Option<String>, format: GraphFormat) -> anyhow::Result<()> {
    let root = crate::compile::find_project(&std::env::current_dir()?)?;
    let view = crate::pack::effective::open(&root, crate::pack::effective::Vars::Literal)?;
    let project = match view.load_checked() {
        Ok((p, _)) => p,
        Err(ds) => {
            for d in &ds {
                eprintln!("{}", d.render());
            }
            bail!("the project is invalid; run `armadai validate`")
        }
    };
    let pairs: Vec<(String, String)> = match flow {
        Some(id) => match project.flows.get(&id) {
            Some(f) => vec![(id.clone(), armadai_model::flow_to_mermaid(&id, f))],
            None if project.flows.is_empty() => {
                bail!("unknown flow `{id}`: the project has no flow")
            }
            None => bail!(
                "unknown flow `{id}`; flows: {}",
                project.flows.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        },
        None => (project.flows.iter())
            .map(|(id, f)| (id.clone(), armadai_model::flow_to_mermaid(id, f)))
            .collect(),
    };
    let blocks: Vec<String> = (pairs.iter())
        .map(|(id, m)| match format {
            GraphFormat::Mermaid => m.clone(),
            GraphFormat::Md => markdown_block(id, m),
        })
        .collect();
    if blocks.is_empty() {
        bail!("the project has no flow (.armadai/flows/ is empty)");
    }
    print!("{}", blocks.join("\n"));
    Ok(())
}
