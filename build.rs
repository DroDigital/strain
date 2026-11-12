//! Embeds everything under assets/ into the binary, so the game stays a single
//! self-contained executable (`cargo run --release` needs no files at runtime).
use std::path::Path;

fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, root, out);
        } else if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
            if matches!(ext, "png" | "glb" | "wav") {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.push((rel, p.canonicalize().unwrap().to_string_lossy().replace('\\', "/")));
            }
        }
    }
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    println!("cargo:rerun-if-changed=assets");
    let mut files = Vec::new();
    if root.exists() {
        walk(&root, &root, &mut files);
    }
    let mut src = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for (rel, abs) in &files {
        src.push_str(&format!("    ({rel:?}, include_bytes!({abs:?})),\n"));
    }
    src.push_str("];\n");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("assets_table.rs");
    std::fs::write(out, src).unwrap();
}
