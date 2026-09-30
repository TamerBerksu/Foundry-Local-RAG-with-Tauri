mod chunker;
mod config;
mod engine;
mod library;
mod prompts;
mod store;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{Manager, State};
use tokio_stream::StreamExt;

use config::{MAX_TOKENS, MAX_TOKENS_COMPACT, TEMPERATURE, TOP_K, TOP_K_COMPACT};
use engine::{Engine, Source, Status, Turn};
use store::{DocEntry, Store};

pub struct AppState {
    store: Mutex<Store>,
    engine: Engine,
    compact: AtomicBool,
    ticket: AtomicU64,
    seed_dir: Option<PathBuf>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "kind", content = "data", rename_all = "lowercase")]
pub enum Piece {
    Sources(Vec<Source>),
    Text(String),
    Error(String),
    Done,
}

fn text_err<E: ToString>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
fn status(state: State<'_, AppState>) -> Status {
    state
        .engine
        .status
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}

#[tauri::command]
fn documents(state: State<'_, AppState>) -> Result<Vec<DocEntry>, String> {
    state.store.lock().map_err(text_err)?.documents().map_err(text_err)
}

#[tauri::command]
fn add_document(state: State<'_, AppState>, name: String, content: String) -> Result<DocEntry, String> {
    if !library::accepts(&name) {
        return Err(format!("{name} is not a .txt or .md file"));
    }
    let mut store = state.store.lock().map_err(text_err)?;
    library::ingest(&mut store, &name, &content)
}

#[tauri::command]
fn remove_document(state: State<'_, AppState>, doc_id: String) -> Result<usize, String> {
    state
        .store
        .lock()
        .map_err(text_err)?
        .remove_document(&doc_id)
        .map_err(text_err)
}

#[tauri::command]
fn replant(state: State<'_, AppState>) -> Result<usize, String> {
    let dir = state.seed_dir.clone().ok_or("no seed texts were bundled")?;
    let mut store = state.store.lock().map_err(text_err)?;
    Ok(library::ingest_folder(&mut store, &dir))
}

#[tauri::command]
fn set_compact(state: State<'_, AppState>, on: bool) -> bool {
    state.compact.store(on, Ordering::Relaxed);
    on
}

#[tauri::command]
fn stop(state: State<'_, AppState>) {
    state.ticket.fetch_add(1, Ordering::SeqCst);
}

#[tauri::command]
async fn ask(
    state: State<'_, AppState>,
    question: String,
    history: Vec<Turn>,
    channel: Channel<Piece>,
) -> Result<(), String> {
    let question = question.trim().to_string();
    if question.is_empty() {
        return Err("ask something first".into());
    }

    let model = state
        .engine
        .model
        .lock()
        .map_err(text_err)?
        .clone()
        .ok_or("the model is still waking")?;

    let compact = state.compact.load(Ordering::Relaxed);
    let top_k = if compact { TOP_K_COMPACT } else { TOP_K };
    let hits = {
        let mut store = state.store.lock().map_err(text_err)?;
        store.search(&question, top_k).map_err(text_err)?
    };
    let _ = channel.send(Piece::Sources(hits.iter().map(Source::from).collect()));

    let messages = engine::build_messages(compact, &hits, &history, &question)?;
    let ticket = state.ticket.fetch_add(1, Ordering::SeqCst) + 1;

    let client = model
        .create_chat_client()
        .temperature(TEMPERATURE)
        .max_tokens(if compact { MAX_TOKENS_COMPACT } else { MAX_TOKENS });

    let mut stream = match client.complete_streaming_chat(&messages, None).await {
        Ok(s) => s,
        Err(e) => {
            let _ = channel.send(Piece::Error(e.to_string()));
            let _ = channel.send(Piece::Done);
            return Ok(());
        }
    };

    while let Some(item) = stream.next().await {
        if state.ticket.load(Ordering::SeqCst) != ticket {
            break;
        }
        match item {
            Ok(chunk) => {
                if let Some(text) = chunk.choices.first().and_then(|c| c.delta.content.clone()) {
                    if !text.is_empty() {
                        let _ = channel.send(Piece::Text(text));
                    }
                }
            }
            Err(e) => {
                let _ = channel.send(Piece::Error(e.to_string()));
                break;
            }
        }
    }

    let _ = channel.send(Piece::Done);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let mut store = Store::open(&data.join(config::DB_FILE))?;

            let seed_dir = app
                .path()
                .resource_dir()
                .ok()
                .map(|d| d.join("seed"))
                .filter(|d| d.is_dir());
            if store.count()? == 0 {
                if let Some(dir) = &seed_dir {
                    library::ingest_folder(&mut store, dir);
                }
            }

            app.manage(AppState {
                store: Mutex::new(store),
                engine: Engine::default(),
                compact: AtomicBool::new(false),
                ticket: AtomicU64::new(0),
                seed_dir,
            });

            tauri::async_runtime::spawn(engine::boot(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            status,
            documents,
            add_document,
            remove_document,
            replant,
            set_compact,
            stop,
            ask
        ])
        .run(tauri::generate_context!())
        .expect("culm failed to start");
}
