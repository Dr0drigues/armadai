//! `fake-claude` — deterministic stand-in for the `claude` CLI, used by the gaveldrop
//! e2e suite. The engine lives in the `armadai-fake` crate (built on `gaveldrop-fake`);
//! this binary is only its entry point. Built only under the `e2e-fake` feature so a
//! default release build never pulls the external gaveldrop deps.
fn main() {
    armadai_fake::run();
}
