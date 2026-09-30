pub const APP_NAME: &str = "culm";
pub const MODEL_ALIAS: &str = "phi-3.5-mini";
pub const MODEL_ENV: &str = "CULM_MODEL";
pub const DB_FILE: &str = "culm.db";

pub const CHUNK_SIZE: usize = 200;
pub const CHUNK_OVERLAP: usize = 25;
pub const TOP_K: usize = 3;
pub const TOP_K_COMPACT: usize = 2;

pub const TEMPERATURE: f64 = 0.1;
pub const MAX_TOKENS: u32 = 1024;
pub const MAX_TOKENS_COMPACT: u32 = 512;
pub const HISTORY_LIMIT: usize = 6;
pub const EXCERPT_CHARS: usize = 220;

pub fn model_alias() -> String {
    std::env::var(MODEL_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| MODEL_ALIAS.to_string())
}
