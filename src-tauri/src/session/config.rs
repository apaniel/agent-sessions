use log::debug;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::model::ProjectLink;

// ---------------------------------------------------------------------------
// TTL Cache (same pattern as git.rs)
// ---------------------------------------------------------------------------

struct CacheEntry<T> {
    value: T,
    inserted_at: Instant,
}

struct TtlCache<T> {
    map: HashMap<String, CacheEntry<T>>,
    ttl: Duration,
}

impl<T: Clone> TtlCache<T> {
    fn new(ttl: Duration) -> Self {
        TtlCache {
            map: HashMap::new(),
            ttl,
        }
    }

    fn get(&self, key: &str) -> Option<T> {
        let entry = self.map.get(key)?;
        if entry.inserted_at.elapsed() > self.ttl {
            return None;
        }
        Some(entry.value.clone())
    }

    fn insert(&mut self, key: String, value: T) {
        self.map.insert(
            key,
            CacheEntry {
                value,
                inserted_at: Instant::now(),
            },
        );
    }

    fn invalidate(&mut self, key: &str) {
        self.map.remove(key);
    }
}

// ---------------------------------------------------------------------------
// Static cache
// ---------------------------------------------------------------------------

static CONFIG_CACHE: Lazy<Mutex<TtlCache<ProjectConfig>>> =
    Lazy::new(|| Mutex::new(TtlCache::new(Duration::from_secs(60))));

// ---------------------------------------------------------------------------
// Config file schema
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    #[serde(default)]
    pub links: Vec<ProjectLink>,
    #[serde(default)]
    pub session_links: HashMap<String, Vec<ProjectLink>>,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        ProjectConfig {
            links: Vec::new(),
            session_links: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Read full project config from `.agent-sessions.json` in the project root.
/// Returns default config on missing file or parse errors. Cached for 60s.
pub fn get_config(project_path: &str) -> ProjectConfig {
    {
        let cache = CONFIG_CACHE.lock().unwrap();
        if let Some(cached) = cache.get(project_path) {
            return cached;
        }
    }

    let result = read_config(project_path);

    let mut cache = CONFIG_CACHE.lock().unwrap();
    cache.insert(project_path.to_string(), result.clone());
    result
}

/// Read project links from `.agent-sessions.json` in the project root.
/// Returns an empty vec on missing file or parse errors. Cached for 60s.
pub fn get_project_links(project_path: &str) -> Vec<ProjectLink> {
    get_config(project_path).links
}

/// Read session-specific links from `.agent-sessions.json`.
pub fn get_session_links(project_path: &str, session_id: &str) -> Vec<ProjectLink> {
    get_config(project_path)
        .session_links
        .get(session_id)
        .cloned()
        .unwrap_or_default()
}

/// Write project links to `.agent-sessions.json` (read-modify-write). Invalidates cache.
pub fn set_project_links(project_path: &str, links: Vec<ProjectLink>) -> Result<(), String> {
    let mut cache = CONFIG_CACHE.lock().unwrap();
    let mut config = read_config(project_path);
    config.links = links;
    write_config(project_path, &config)?;
    cache.invalidate(project_path);
    Ok(())
}

/// Write session links to `.agent-sessions.json` (read-modify-write). Invalidates cache.
pub fn set_session_links(
    project_path: &str,
    session_id: &str,
    links: Vec<ProjectLink>,
) -> Result<(), String> {
    let mut cache = CONFIG_CACHE.lock().unwrap();
    let mut config = read_config(project_path);
    if links.is_empty() {
        config.session_links.remove(session_id);
    } else {
        config.session_links.insert(session_id.to_string(), links);
    }
    write_config(project_path, &config)?;
    cache.invalidate(project_path);
    Ok(())
}

/// Clean up cache entries for projects that are no longer active.
pub fn cleanup_links_cache(active_project_paths: &std::collections::HashSet<String>) {
    if let Ok(mut cache) = CONFIG_CACHE.lock() {
        cache.map.retain(|k, _| active_project_paths.contains(k));
    }
}

// ---------------------------------------------------------------------------
// Internal
// ---------------------------------------------------------------------------

fn read_config(project_path: &str) -> ProjectConfig {
    let config_path = std::path::Path::new(project_path).join(".agent-sessions.json");

    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return ProjectConfig::default(),
    };

    match serde_json::from_str::<ProjectConfig>(&content) {
        Ok(config) => {
            debug!(
                "Loaded {} project links from {:?}",
                config.links.len(),
                config_path
            );
            config
        }
        Err(e) => {
            debug!("Failed to parse {:?}: {}", config_path, e);
            ProjectConfig::default()
        }
    }
}

fn write_config(project_path: &str, config: &ProjectConfig) -> Result<(), String> {
    let config_path = std::path::Path::new(project_path).join(".agent-sessions.json");
    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(&config_path, json)
        .map_err(|e| format!("Failed to write {:?}: {}", config_path, e))?;
    debug!("Wrote config to {:?}", config_path);
    Ok(())
}
