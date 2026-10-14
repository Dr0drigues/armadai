//! FROZEN copies of the few items the audit engine used from the `armadai`
//! binary crate, which this standalone crate cannot depend on (it has no
//! `lib.rs`), and from the deleted `armadai-providers` crate. Each file names
//! its origin. They are not kept in sync with the originals: the whole crate
//! is frozen until the audit is rebuilt (post-v1).

pub mod linker;
pub mod model_registry;
pub mod style;
pub mod transcript;
