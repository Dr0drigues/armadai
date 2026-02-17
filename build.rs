fn main() {
    // Force cargo to re-run when embedded asset directories change.
    // include_dir! is a proc-macro that embeds files at compile time but
    // doesn't track changes on stable Rust (needs nightly `track_path`).
    println!("cargo::rerun-if-changed=skills");
    println!("cargo::rerun-if-changed=starters");
    println!("cargo::rerun-if-changed=templates");
}
