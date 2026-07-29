fn main() {
    // skills/ and starters/ are embedded via include_dir! in skill.rs/starter.rs;
    // track them so edits trigger a rebuild (include_dir! doesn't on stable).
    println!("cargo::rerun-if-changed=skills");
    println!("cargo::rerun-if-changed=starters");
}
