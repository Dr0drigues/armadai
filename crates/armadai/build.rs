fn main() {
    // include_dir! embeds at compile time but doesn't track changes on stable.
    println!("cargo::rerun-if-changed=web/ui/dist");
}
