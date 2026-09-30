use std::path::Path;

use crate::chunker::{chunk_text, parse_front_matter};
use crate::config::{CHUNK_OVERLAP, CHUNK_SIZE};
use crate::store::{DocEntry, Store};

pub const ACCEPTED: &[&str] = &["md", "markdown", "txt", "text"];

fn stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| name.to_string())
}

pub fn accepts(name: &str) -> bool {
    Path::new(name)
        .extension()
        .map(|e| ACCEPTED.contains(&e.to_string_lossy().to_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn ingest(store: &mut Store, name: &str, content: &str) -> Result<DocEntry, String> {
    let parsed = parse_front_matter(content);
    let doc_id = parsed
        .meta
        .get("id")
        .cloned()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| stem(name));
    let title = parsed
        .meta
        .get("title")
        .cloned()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| stem(name));
    let category = parsed.meta.get("category").cloned().unwrap_or_default();

    let chunks = chunk_text(&parsed.body, CHUNK_SIZE, CHUNK_OVERLAP);
    if chunks.is_empty() {
        return Err(format!("{name} holds no readable text"));
    }
    let n = store
        .put_document(&doc_id, &title, &category, &chunks)
        .map_err(|e| e.to_string())?;

    Ok(DocEntry {
        doc_id,
        title,
        category,
        chunks: n as i64,
    })
}

pub fn ingest_folder(store: &mut Store, dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut files: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && accepts(&p.to_string_lossy()))
        .collect();
    files.sort();

    files
        .iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(p).ok()?;
            let name = p.file_name()?.to_string_lossy().to_string();
            ingest(store, &name, &text).ok()
        })
        .count()
}
