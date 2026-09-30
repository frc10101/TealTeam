//! Compile `static/` into the binary (P5).
//!
//! Writes `$OUT_DIR/static_assets.rs`: one entry per file under `static/`,
//! with its path, an ETag, and its bytes by `include_bytes!`. `src/assets.rs`
//! serves them. Editing a file there rebuilds the binary, so `cargo run`
//! always serves the current CSS.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let root = Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap()).join("static");
    println!("cargo:rerun-if-changed=static");

    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();

    let mut out = String::from("pub static ASSETS: &[Asset] = &[\n");
    for file in &files {
        let path = file
            .strip_prefix(&root)
            .unwrap()
            .to_str()
            .expect("static file names are UTF-8")
            .replace('\\', "/");
        let bytes = fs::read(file).unwrap();
        writeln!(
            out,
            "    Asset {{ path: {path:?}, etag: \"\\\"{:016x}\\\"\", bytes: include_bytes!({:?}) }},",
            fnv1a(&bytes),
            file.display().to_string(),
        )
        .unwrap();
    }
    out.push_str("];\n");

    let dest = Path::new(&env::var("OUT_DIR").unwrap()).join("static_assets.rs");
    fs::write(dest, out).unwrap();
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, files);
        } else {
            files.push(path);
        }
    }
}

/// Only has to change when the bytes do; not a security boundary.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}
