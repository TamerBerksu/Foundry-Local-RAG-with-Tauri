use std::sync::{Arc, Mutex};

use foundry_local_sdk::{ChatCompletionRequestMessage, FoundryLocalConfig, FoundryLocalManager, Model};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::config::{self, APP_NAME, EXCERPT_CHARS, HISTORY_LIMIT};
use crate::prompts::{SYSTEM_PROMPT, SYSTEM_PROMPT_COMPACT};
use crate::store::Hit;
use crate::AppState;

#[derive(Serialize, Clone, Default)]
pub struct Status {
    pub phase: String,
    pub message: String,
    pub progress: Option<f64>,
    pub model: Option<String>,
}

#[derive(Default)]
pub struct Engine {
    pub model: Mutex<Option<Arc<Model>>>,
    pub status: Mutex<Status>,
}

#[derive(Deserialize)]
pub struct Turn {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub doc_id: String,
    pub title: String,
    pub category: String,
    pub score: f32,
    pub excerpt: String,
}

impl From<&Hit> for Source {
    fn from(hit: &Hit) -> Self {
        let mut excerpt: String = hit.content.chars().take(EXCERPT_CHARS).collect();
        if hit.content.chars().count() > EXCERPT_CHARS {
            excerpt.push('…');
        }
        Source {
            doc_id: hit.doc_id.clone(),
            title: hit.title.clone(),
            category: hit.category.clone(),
            score: (hit.score * 100.0).round() / 100.0,
            excerpt,
        }
    }
}

fn core_library() -> &'static str {
    if cfg!(target_os = "windows") {
        "Microsoft.AI.Foundry.Local.Core.dll"
    } else if cfg!(target_os = "macos") {
        "Microsoft.AI.Foundry.Local.Core.dylib"
    } else {
        "Microsoft.AI.Foundry.Local.Core.so"
    }
}

pub fn report(app: &AppHandle, phase: &str, message: impl Into<String>, progress: Option<f64>) {
    let state = app.state::<AppState>();
    let model = state
        .engine
        .model
        .lock()
        .ok()
        .and_then(|m| m.as_ref().map(|m| m.id().to_string()));
    let status = Status {
        phase: phase.into(),
        message: message.into(),
        progress,
        model,
    };
    if let Ok(mut current) = state.engine.status.lock() {
        *current = status.clone();
    }
    let _ = app.emit("status", status);
}

pub async fn boot(app: AppHandle) {
    if let Err(e) = start(&app).await {
        report(&app, "error", e, None);
    }
}

async fn start(app: &AppHandle) -> Result<(), String> {
    let alias = config::model_alias();
    report(app, "waking", "Waking the local runtime", None);

    let bundled = app
        .path()
        .resource_dir()
        .ok()
        .map(|d| d.join("native"))
        .filter(|d| d.join(core_library()).exists());

    let manager = tauri::async_runtime::spawn_blocking(move || {
        let mut cfg = FoundryLocalConfig::new(APP_NAME);
        if let Some(dir) = bundled {
            cfg = cfg.library_path(dir.to_string_lossy().to_string());
        }
        FoundryLocalManager::create(cfg).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())??;

    report(app, "seeking", format!("Looking for {alias}"), None);
    let catalog = manager.catalog();
    let model = match catalog.get_model(&alias).await {
        Ok(m) => m,
        Err(first) => catalog
            .get_cached_models()
            .await
            .ok()
            .and_then(|cached| cached.into_iter().find(|m| m.alias() == alias))
            .ok_or_else(|| first.to_string())?,
    };

    if !model.is_cached().await.map_err(|e| e.to_string())? {
        report(app, "fetching", format!("Fetching {alias}, first run only"), Some(0.0));
        let handle = app.clone();
        let label = alias.clone();
        let mut last = -1.0f64;
        model
            .download(Some(move |p: f64| {
                if p - last >= 0.5 || p >= 100.0 {
                    last = p;
                    report(&handle, "fetching", format!("Fetching {label}, first run only"), Some(p));
                }
            }))
            .await
            .map_err(|e| e.to_string())?;
    }

    report(app, "loading", format!("Loading {alias} into memory"), None);
    model.load().await.map_err(|e| e.to_string())?;

    let state = app.state::<AppState>();
    if let Ok(mut slot) = state.engine.model.lock() {
        *slot = Some(model.clone());
    }
    report(app, "ready", format!("{} is listening", model.alias()), None);
    Ok(())
}

fn context_block(hits: &[Hit]) -> String {
    if hits.is_empty() {
        return "No passages matched in the local library.".into();
    }
    hits.iter()
        .enumerate()
        .map(|(i, h)| {
            let category = if h.category.is_empty() {
                String::new()
            } else {
                format!(" [{}]", h.category)
            };
            format!("--- Passage {}: {}{} ---\n{}", i + 1, h.title, category, h.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn build_messages(
    compact: bool,
    hits: &[Hit],
    history: &[Turn],
    question: &str,
) -> Result<Vec<ChatCompletionRequestMessage>, String> {
    let prompt = if compact { SYSTEM_PROMPT_COMPACT } else { SYSTEM_PROMPT };
    let system = format!(
        "{prompt}\n\nPassages from the local library:\n\n{}",
        context_block(hits)
    );

    let mut raw = vec![json!({ "role": "system", "content": system })];
    let recent = history
        .iter()
        .filter(|t| (t.role == "user" || t.role == "assistant") && !t.content.trim().is_empty())
        .collect::<Vec<_>>();
    let skip = recent.len().saturating_sub(HISTORY_LIMIT);
    for turn in recent.into_iter().skip(skip) {
        raw.push(json!({ "role": turn.role, "content": turn.content }));
    }
    raw.push(json!({ "role": "user", "content": question }));

    raw.into_iter()
        .map(|m| serde_json::from_value(m).map_err(|e| e.to_string()))
        .collect()
}
