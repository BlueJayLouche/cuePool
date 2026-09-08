#[path = "build_support/identity.rs"]
mod identity;

fn main() {
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.parent().unwrap().parent().unwrap();
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    // Deliberately absent: Cargo must check Git on EVERY invocation, including
    // index-only edits, new untracked files, tag moves, worktrees and checkouts
    // that change no Rust files. Watching .git/HEAD alone misses these cases.
    println!(
        "cargo:rerun-if-changed={}",
        out.join("always-check-git").display()
    );
    println!("cargo:rerun-if-env-changed=CUEPOOL_BUILD_ID");
    let metadata = identity::collect(
        root,
        &std::env::var("CARGO_PKG_VERSION").unwrap(),
        std::env::var("CUEPOOL_BUILD_ID").ok().as_deref(),
    );
    let generated = metadata.rust_source();
    let path = out.join("build_identity.rs");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(&generated) {
        std::fs::write(path, generated).unwrap();
    }
}
