//! `/static/`, served from inside the binary (P5).
//!
//! The retired binary found `static/` by walking up from its own location and
//! the working directory. That worked under `cargo run` and from
//! `target/release/`, but a binary copied to the Pi without the folder beside
//! it served a page with no CSS and no scripts, and said nothing. Migrations,
//! the season, and the templates are already compiled in; now the assets are
//! too, so the binary is the whole deploy (REBUILD_SPEC.md 10).
//!
//! `build.rs` writes the table. Each file carries an ETag of its bytes and is
//! sent `Cache-Control: no-cache`: a browser keeps its copy but asks each time,
//! and the answer is a bodiless 304 until a new binary changes the file.

use axum::extract::Path;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

pub struct Asset {
    /// Relative to `static/`, with `/` separators: `css/site.css`.
    pub path: &'static str,
    /// Quoted, as the header wants it.
    pub etag: &'static str,
    pub bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/static_assets.rs"));

pub fn find(path: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|a| a.path == path)
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        // The service worker's module (C5). Streaming compilation insists.
        Some("wasm") => "application/wasm",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// `GET /static/{*path}`. A missing file is a bare 404, as it was from disk.
pub async fn serve(Path(path): Path<String>, headers: HeaderMap) -> Response {
    let Some(asset) = find(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let cache = [(ETAG, asset.etag), (CACHE_CONTROL, "no-cache")];
    let fresh = headers
        .get_all(IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|tag| tag.trim() == asset.etag || tag.trim() == "*");
    if fresh {
        return (StatusCode::NOT_MODIFIED, cache).into_response();
    }
    (
        [(CONTENT_TYPE, content_type(asset.path))],
        cache,
        asset.bytes,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_under_static_is_compiled_in() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("static");
        let mut stack = vec![root.clone()];
        let mut seen = 0;
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let key = path.strip_prefix(&root).unwrap().to_str().unwrap();
                let asset = find(key).unwrap_or_else(|| panic!("{key} is not embedded"));
                assert_eq!(asset.bytes, std::fs::read(&path).unwrap(), "{key}");
                seen += 1;
            }
        }
        assert_eq!(seen, ASSETS.len());
        assert!(find("css/site.css").is_some());
    }

    #[test]
    fn types_are_what_browsers_insist_on() {
        // A stylesheet or script served as anything else is refused under
        // `X-Content-Type-Options: nosniff`.
        assert_eq!(content_type("css/site.css"), "text/css; charset=utf-8");
        assert_eq!(content_type("js/live.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("README"), "application/octet-stream");
    }

    /// Width and height from a PNG's header.
    fn png_size(bytes: &[u8]) -> (u32, u32) {
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "a PNG");
        let be = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
        (be(16), be(20))
    }

    #[test]
    fn the_manifest_has_what_install_needs_and_every_icon_it_names() {
        // C2. What Chrome checks before it offers to install: a name, a
        // start_url inside the scope, a standalone display, and PNG icons at
        // 192 and 512. Every icon must be embedded, at the size it claims.
        let manifest: serde_json::Value =
            serde_json::from_slice(find("manifest.webmanifest").expect("embedded").bytes)
                .expect("valid JSON");
        assert_eq!(manifest["short_name"], "TealTeam");
        assert!(manifest["name"].as_str().is_some_and(|n| !n.is_empty()));
        assert_eq!(manifest["display"], "standalone");
        let (start, scope) = (
            manifest["start_url"].as_str().unwrap(),
            manifest["scope"].as_str().unwrap(),
        );
        assert!(start.starts_with(scope));

        let mut png_sizes = Vec::new();
        let mut maskable = false;
        for icon in manifest["icons"].as_array().unwrap() {
            let src = icon["src"].as_str().unwrap();
            let asset = find(src.strip_prefix("/static/").expect("under /static/"))
                .unwrap_or_else(|| panic!("{src} is not embedded"));
            assert_eq!(
                content_type(asset.path),
                icon["type"].as_str().unwrap(),
                "{src}"
            );
            maskable |= icon["purpose"] == "maskable";
            if icon["type"] == "image/png" {
                let (w, h) = png_size(asset.bytes);
                assert_eq!(icon["sizes"], format!("{w}x{h}"), "{src}");
                png_sizes.push(w);
            }
        }
        assert!(png_sizes.contains(&192) && png_sizes.contains(&512));
        assert!(maskable, "Android crops icons to its own shape");
        assert_eq!(
            png_size(find("icons/apple-touch-icon.png").unwrap().bytes),
            (180, 180),
            "what iOS puts on the Home Screen"
        );
        assert_eq!(
            content_type("manifest.webmanifest"),
            "application/manifest+json"
        );
    }

    #[test]
    fn etags_differ_between_files() {
        let mut tags: Vec<_> = ASSETS.iter().map(|a| a.etag).collect();
        tags.sort();
        tags.dedup();
        assert_eq!(tags.len(), ASSETS.len());
    }
}
