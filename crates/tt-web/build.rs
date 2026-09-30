//! Compile `static/` into the binary (P5).
//!
//! Writes `$OUT_DIR/static_assets.rs`: one entry per file under `static/`,
//! with its path, an ETag, and its bytes by `include_bytes!`. `src/assets.rs`
//! serves them. Editing a file there rebuilds the binary, so `cargo run`
//! always serves the current CSS.
//!
//! Also `BUILD_VERSION` (C1): a hash of everything the offline shell is made
//! of -- the static files, the templates, and the service worker's source.
//! It names the service worker's cache, so a binary that changes any of them
//! replaces the shell on every device, and one that changes none leaves it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("static");
    let templates = manifest.join("../tt-templates/templates");
    let worker = manifest.join("src/sw.js");
    println!("cargo:rerun-if-changed=static");
    println!("cargo:rerun-if-changed={}", templates.display());
    println!("cargo:rerun-if-changed={}", worker.display());
    let mut build = fnv1a_from(
        FNV_OFFSET,
        env::var("CARGO_PKG_VERSION").unwrap().as_bytes(),
    );

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
        build = fnv1a_from(fnv1a_from(build, path.as_bytes()), &bytes);
        writeln!(
            out,
            "    Asset {{ path: {path:?}, etag: \"\\\"{:016x}\\\"\", bytes: include_bytes!({:?}) }},",
            fnv1a(&bytes),
            file.display().to_string(),
        )
        .unwrap();
    }
    out.push_str("];\n");

    let mut shell = Vec::new();
    collect(&templates, &mut shell);
    shell.sort();
    shell.push(worker);
    for file in &shell {
        build = fnv1a_from(build, &fs::read(file).unwrap());
    }
    writeln!(out, "pub const BUILD_VERSION: &str = \"{build:016x}\";").unwrap();

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
    fnv1a_from(FNV_OFFSET, bytes)
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

fn fnv1a_from(start: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(start, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}
