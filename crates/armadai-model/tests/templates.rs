//! Flow templates: each one yields a flow the validator accepts without a
//! single diagnostic, and refuses a wrong list of agents with a clear error.

use armadai_model::*;

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Diagnostics of a project holding `agents` and the flow `f` (as written by `render_flow`).
fn validate_flow(agents: &[&str], f: &Flow) -> Vec<Diagnostic> {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join(".armadai");
    std::fs::create_dir_all(a.join("agents")).unwrap();
    std::fs::create_dir_all(a.join("flows")).unwrap();
    std::fs::write(a.join("project.yaml"), "schema_version: 1\n").unwrap();
    for id in agents {
        std::fs::write(
            a.join(format!("agents/{id}.md")),
            "---\ndescription: d\n---\nBody\n",
        )
        .unwrap();
    }
    std::fs::write(a.join("flows/f.yaml"), render_flow(f)).unwrap();
    validate(dir.path())
}

fn link(from: &str, to: &str, when: Option<&str>) -> Link {
    Link {
        from: from.into(),
        to: to.into(),
        when: when.map(Into::into),
    }
}

#[test]
fn names_round_trip() {
    for t in FLOW_TEMPLATES {
        assert_eq!(t.as_str().parse::<FlowTemplate>().unwrap(), t);
    }
    assert_eq!(
        FLOW_TEMPLATES.map(|t| t.as_str()),
        ["pipeline", "review-loop", "plan-build-verify", "fan-out"]
    );
    let e = "loop".parse::<FlowTemplate>().unwrap_err();
    assert!(
        e.to_string()
            .contains("pipeline, review-loop, plan-build-verify"),
        "{e}"
    );
}

#[test]
fn every_template_validates_without_a_diagnostic() {
    let cases: [(FlowTemplate, &[&str]); 7] = [
        (FlowTemplate::FanOut, &["lead", "a"]),
        (FlowTemplate::FanOut, &["lead", "a", "b", "c", "d"]),
        (FlowTemplate::Pipeline, &["a", "b"]),
        (FlowTemplate::Pipeline, &["a", "b", "c", "d"]),
        (FlowTemplate::ReviewLoop, &["dev", "rev"]),
        (FlowTemplate::ReviewLoop, &["dev", "rev", "ops"]),
        (FlowTemplate::PlanBuildVerify, &["plan", "build", "check"]),
    ];
    for (t, agents) in cases {
        let f = flow_from_template(t, &ids(agents), Some("d".into())).unwrap();
        let ds = validate_flow(agents, &f);
        assert!(ds.is_empty(), "{t:?} {agents:?}: {ds:?}");
        assert_eq!(f.description.as_deref(), Some("d"));
    }
}

#[test]
fn pipeline_chains_the_agents_in_the_given_order() {
    let f = flow_from_template(FlowTemplate::Pipeline, &ids(&["c", "a", "b"]), None).unwrap();
    assert_eq!(f.links, [link("c", "a", None), link("a", "b", None)]);
    assert_eq!(f.steps["a"].agent, "a");
    assert_eq!(f.description, None);
}

#[test]
fn review_loop_sends_rejections_back_and_approvals_on() {
    let f =
        flow_from_template(FlowTemplate::ReviewLoop, &ids(&["dev", "rev", "ops"]), None).unwrap();
    assert_eq!(f.steps["rev"].states, ["approved", "rejected"]);
    assert!(f.steps["dev"].states.is_empty());
    assert_eq!(
        f.links,
        [
            link("dev", "rev", None),
            link("rev", "dev", Some("rejected")),
            link("rev", "ops", Some("approved")),
        ]
    );
    // without a shipper, approval ends the flow
    let f = flow_from_template(FlowTemplate::ReviewLoop, &ids(&["dev", "rev"]), None).unwrap();
    assert_eq!(
        f.links,
        [
            link("dev", "rev", None),
            link("rev", "dev", Some("rejected"))
        ]
    );
    assert_eq!(entry_steps(&f), ["dev"]);
}

#[test]
fn plan_build_verify_retries_the_build_on_failure() {
    let f = flow_from_template(
        FlowTemplate::PlanBuildVerify,
        &ids(&["plan", "build", "check"]),
        None,
    )
    .unwrap();
    assert_eq!(
        f.links,
        [link("plan", "build", None), link("build", "check", None)]
    );
    assert_eq!(f.steps["check"].on_failure.as_deref(), Some("build"));
    assert_eq!(f.steps["build"].on_failure, None);
    assert_eq!(entry_steps(&f), ["plan"]);
}

#[test]
fn fan_out_sends_the_lead_to_every_agent() {
    let f = flow_from_template(FlowTemplate::FanOut, &ids(&["lead", "b", "a"]), None).unwrap();
    assert_eq!(f.links, [link("lead", "b", None), link("lead", "a", None)]);
    assert!(
        f.steps
            .values()
            .all(|s| s.states.is_empty() && s.on_failure.is_none())
    );
    assert_eq!(entry_steps(&f), ["lead"]);
    let e = flow_from_template(FlowTemplate::FanOut, &ids(&["lead"]), None).unwrap_err();
    assert!(
        e.to_string().contains("fan-out") && e.to_string().contains("at least 2"),
        "{e}"
    );
}

#[test]
fn the_number_of_agents_is_checked() {
    use TemplateError::*;
    let e = |t, a: &[&str]| flow_from_template(t, &ids(a), None).unwrap_err();
    assert!(matches!(
        e(FlowTemplate::Pipeline, &["a"]),
        TooFewAgents { min: 2, .. }
    ));
    assert!(matches!(
        e(FlowTemplate::Pipeline, &[]),
        TooFewAgents { min: 2, .. }
    ));
    assert!(matches!(
        e(FlowTemplate::ReviewLoop, &["a"]),
        TooFewAgents { min: 2, .. }
    ));
    assert!(matches!(
        e(FlowTemplate::ReviewLoop, &["a", "b", "c", "d"]),
        TooManyAgents { max: 3, .. }
    ));
    assert!(matches!(
        e(FlowTemplate::PlanBuildVerify, &["a", "b"]),
        TooFewAgents { min: 3, .. }
    ));
    assert!(matches!(
        e(FlowTemplate::PlanBuildVerify, &["a", "b", "c", "d"]),
        TooManyAgents { max: 3, .. }
    ));
    let msg = e(FlowTemplate::ReviewLoop, &["a", "b", "c", "d"]).to_string();
    assert!(
        msg.contains("review-loop") && msg.contains("at most 3"),
        "{msg}"
    );
    let msg = e(FlowTemplate::Pipeline, &["a"]).to_string();
    assert!(
        msg.contains("pipeline") && msg.contains("at least 2"),
        "{msg}"
    );
}

#[test]
fn duplicate_and_invalid_agents_are_refused() {
    let e = |a: &[&str]| flow_from_template(FlowTemplate::Pipeline, &ids(a), None).unwrap_err();
    assert_eq!(
        e(&["a", "b", "a"]),
        TemplateError::DuplicateAgent("a".into())
    );
    for bad in ["../x", "a/b", "Bad", "", "é"] {
        assert_eq!(
            e(&["ok", bad]),
            TemplateError::InvalidAgentId(bad.into()),
            "{bad}"
        );
    }
}
