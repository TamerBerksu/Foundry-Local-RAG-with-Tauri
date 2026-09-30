use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::chunker::term_frequency;

struct Row {
    doc_id: String,
    title: String,
    category: String,
    content: String,
    tf: HashMap<String, f32>,
    norm: f32,
}

struct Index {
    rows: Vec<Row>,
    postings: HashMap<String, Vec<usize>>,
    idf: HashMap<String, f32>,
}

#[derive(Serialize, Clone)]
pub struct Hit {
    pub doc_id: String,
    pub title: String,
    pub category: String,
    pub content: String,
    pub score: f32,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DocEntry {
    pub doc_id: String,
    pub title: String,
    pub category: String,
    pub chunks: i64,
}

pub struct Store {
    conn: Connection,
    index: Option<Index>,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        Self::prepare(conn)
    }

    #[cfg(test)]
    pub fn memory() -> rusqlite::Result<Self> {
        Self::prepare(Connection::open_in_memory()?)
    }

    fn prepare(conn: Connection) -> rusqlite::Result<Self> {
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS chunks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                doc_id TEXT NOT NULL,
                title TEXT,
                category TEXT,
                chunk_index INTEGER NOT NULL,
                content TEXT NOT NULL,
                tf_json TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_doc_id ON chunks(doc_id);",
        )?;
        Ok(Self { conn, index: None })
    }

    pub fn put_document(
        &mut self,
        doc_id: &str,
        title: &str,
        category: &str,
        chunks: &[String],
    ) -> rusqlite::Result<usize> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM chunks WHERE doc_id = ?1", params![doc_id])?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO chunks (doc_id, title, category, chunk_index, content, tf_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for (i, chunk) in chunks.iter().enumerate() {
                let tf = serde_json::to_string(&term_frequency(chunk)).unwrap_or_else(|_| "{}".into());
                insert.execute(params![doc_id, title, category, i as i64, chunk, tf])?;
            }
        }
        tx.commit()?;
        self.index = None;
        Ok(chunks.len())
    }

    pub fn remove_document(&mut self, doc_id: &str) -> rusqlite::Result<usize> {
        let n = self
            .conn
            .execute("DELETE FROM chunks WHERE doc_id = ?1", params![doc_id])?;
        self.index = None;
        Ok(n)
    }

    pub fn count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
    }

    pub fn documents(&self) -> rusqlite::Result<Vec<DocEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT doc_id, MAX(title), MAX(category), COUNT(*) FROM chunks
             GROUP BY doc_id ORDER BY MAX(title) COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(DocEntry {
                doc_id: r.get(0)?,
                title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                category: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                chunks: r.get(3)?,
            })
        })?;
        rows.collect()
    }

    fn ensure_index(&mut self) -> rusqlite::Result<&Index> {
        if self.index.is_none() {
            self.index = Some(self.build_index()?);
        }
        Ok(self.index.as_ref().expect("index present"))
    }

    fn build_index(&self) -> rusqlite::Result<Index> {
        let mut stmt = self
            .conn
            .prepare("SELECT doc_id, title, category, content, tf_json FROM chunks ORDER BY id")?;
        let mut rows: Vec<Row> = stmt
            .query_map([], |r| {
                let tf_json: String = r.get(4)?;
                Ok(Row {
                    doc_id: r.get(0)?,
                    title: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    category: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    content: r.get(3)?,
                    tf: serde_json::from_str(&tf_json).unwrap_or_default(),
                    norm: 0.0,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mut postings: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, row) in rows.iter().enumerate() {
            for term in row.tf.keys() {
                postings.entry(term.clone()).or_default().push(i);
            }
        }

        let n = rows.len() as f32;
        let idf: HashMap<String, f32> = postings
            .iter()
            .map(|(t, ids)| (t.clone(), ((1.0 + n) / (1.0 + ids.len() as f32)).ln() + 1.0))
            .collect();

        for row in rows.iter_mut() {
            row.norm = row
                .tf
                .iter()
                .map(|(t, f)| {
                    let w = f * idf.get(t).copied().unwrap_or(1.0);
                    w * w
                })
                .sum::<f32>()
                .sqrt();
        }

        Ok(Index {
            rows,
            postings,
            idf,
        })
    }

    pub fn search(&mut self, query: &str, top_k: usize) -> rusqlite::Result<Vec<Hit>> {
        let query_tf = term_frequency(query);
        let index = self.ensure_index()?;

        let mut query_norm = 0.0f32;
        let mut dots: HashMap<usize, f32> = HashMap::new();
        for (term, qf) in &query_tf {
            let Some(idf) = index.idf.get(term) else {
                continue;
            };
            let qw = qf * idf;
            query_norm += qw * qw;
            for &i in &index.postings[term] {
                let dw = index.rows[i].tf.get(term).copied().unwrap_or(0.0) * idf;
                *dots.entry(i).or_insert(0.0) += qw * dw;
            }
        }
        if query_norm == 0.0 {
            return Ok(Vec::new());
        }
        let query_norm = query_norm.sqrt();

        let mut scored: Vec<(usize, f32)> = dots
            .into_iter()
            .filter_map(|(i, dot)| {
                let norm = index.rows[i].norm;
                (norm > 0.0).then(|| (i, dot / (norm * query_norm)))
            })
            .filter(|(_, s)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.truncate(top_k);

        Ok(scored
            .into_iter()
            .map(|(i, score)| {
                let row = &index.rows[i];
                Hit {
                    doc_id: row.doc_id.clone(),
                    title: row.title.clone(),
                    category: row.category.clone(),
                    content: row.content.clone(),
                    score,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_passage_wins() {
        let mut store = Store::memory().unwrap();
        store
            .put_document("a", "Rhizomes", "Growth", &["running rhizomes spread under barriers".into()])
            .unwrap();
        store
            .put_document("b", "Harvest", "Craft", &["cure culms after harvest in winter".into()])
            .unwrap();
        let hits = store.search("how do rhizomes spread", 3).unwrap();
        assert_eq!(hits[0].doc_id, "a");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn replace_and_remove() {
        let mut store = Store::memory().unwrap();
        store.put_document("a", "A", "", &["one".into(), "two".into()]).unwrap();
        store.put_document("a", "A", "", &["three".into()]).unwrap();
        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(store.documents().unwrap()[0].chunks, 1);
        store.remove_document("a").unwrap();
        assert_eq!(store.count().unwrap(), 0);
        assert!(store.search("three", 3).unwrap().is_empty());
    }
}
