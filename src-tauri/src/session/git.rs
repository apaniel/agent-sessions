use log::{debug, info};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Pull request information from GitHub
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrInfo {
    pub url: String,
    pub number: u32,
    pub state: String,
    pub ci_status: Option<CiStatus>,
}

/// CI/pipeline check status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CiStatus {
    Success,
    Failure,
    Pending,
    Unknown,
}

// ---------------------------------------------------------------------------
// Generic TTL Cache
// ---------------------------------------------------------------------------

struct CacheEntry<T> {
    value: T,
    inserted_at: Instant,
}

struct TtlCache<T> {
    map: HashMap<String, CacheEntry<T>>,
    ttl: Option<Duration>, // None = permanent
}

impl<T: Clone> TtlCache<T> {
    fn new(ttl: Option<Duration>) -> Self {
        TtlCache {
            map: HashMap::new(),
            ttl,
        }
    }

    fn get(&self, key: &str) -> Option<T> {
        let entry = self.map.get(key)?;
        if let Some(ttl) = self.ttl {
            if entry.inserted_at.elapsed() > ttl {
                return None;
            }
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

    fn retain_keys(&mut self, active_keys: &std::collections::HashSet<String>) {
        self.map.retain(|k, _| active_keys.contains(k));
    }
}

// ---------------------------------------------------------------------------
// Static caches
// ---------------------------------------------------------------------------

static WORKTREE_CACHE: Lazy<Mutex<TtlCache<bool>>> =
    Lazy::new(|| Mutex::new(TtlCache::new(None)));

static GITHUB_URL_CACHE: Lazy<Mutex<TtlCache<Option<String>>>> =
    Lazy::new(|| Mutex::new(TtlCache::new(None)));

static PR_INFO_CACHE: Lazy<Mutex<TtlCache<Option<PrInfo>>>> =
    Lazy::new(|| Mutex::new(TtlCache::new(Some(Duration::from_secs(60)))));

static AHEAD_BEHIND_CACHE: Lazy<Mutex<TtlCache<Option<(u32, u32)>>>> =
    Lazy::new(|| Mutex::new(TtlCache::new(Some(Duration::from_secs(30)))));

/// Whether `gh` CLI is available (checked once at startup)
static GH_AVAILABLE: Lazy<bool> = Lazy::new(|| {
    let available = Command::new("gh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    info!("GitHub CLI (gh) available: {}", available);
    available
});

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Get GitHub URL from a project's git remote origin (cached permanently).
pub fn get_github_url(project_path: &str) -> Option<String> {
    {
        let cache = GITHUB_URL_CACHE.lock().unwrap();
        if let Some(cached) = cache.get(project_path) {
            return cached;
        }
    }

    let result = fetch_github_url(project_path);

    let mut cache = GITHUB_URL_CACHE.lock().unwrap();
    cache.insert(project_path.to_string(), result.clone());
    result
}

/// Derive "user/repo" from a GitHub URL like "https://github.com/user/repo".
pub fn get_repo_name(github_url: &Option<String>) -> Option<String> {
    let url = github_url.as_ref()?;
    let path = url.strip_prefix("https://github.com/")?;
    if path.contains('/') {
        Some(path.to_string())
    } else {
        None
    }
}

/// Check if a project path is inside a git worktree (cached permanently).
pub fn is_worktree(project_path: &str) -> bool {
    {
        let cache = WORKTREE_CACHE.lock().unwrap();
        if let Some(cached) = cache.get(project_path) {
            return cached;
        }
    }

    let result = check_is_worktree(project_path);
    debug!("Worktree check for {}: {}", project_path, result);

    let mut cache = WORKTREE_CACHE.lock().unwrap();
    cache.insert(project_path.to_string(), result);
    result
}

/// Get commits ahead/behind upstream (cached 30s).
/// Returns (ahead, behind) or None if not a git repo or no upstream.
pub fn get_ahead_behind(project_path: &str, branch: &str) -> Option<(u32, u32)> {
    let cache_key = format!("{}:{}", project_path, branch);

    {
        let cache = AHEAD_BEHIND_CACHE.lock().unwrap();
        if let Some(cached) = cache.get(&cache_key) {
            return cached;
        }
    }

    let result = fetch_ahead_behind(project_path, branch);

    let mut cache = AHEAD_BEHIND_CACHE.lock().unwrap();
    cache.insert(cache_key, result);
    result
}

/// Get PR info for a branch (cached 60s). Returns None if no PR or gh unavailable.
pub fn get_pr_info(project_path: &str, branch: &str) -> Option<PrInfo> {
    if !*GH_AVAILABLE {
        return None;
    }

    let cache_key = format!("{}:{}", project_path, branch);

    {
        let cache = PR_INFO_CACHE.lock().unwrap();
        if let Some(cached) = cache.get(&cache_key) {
            return cached;
        }
    }

    let result = fetch_pr_info(project_path, branch);

    let mut cache = PR_INFO_CACHE.lock().unwrap();
    cache.insert(cache_key, result.clone());
    result
}

/// Clean up cache entries for projects that are no longer active.
pub fn cleanup_git_caches(active_project_paths: &std::collections::HashSet<String>) {
    // Build a set of all cache keys that match active project paths
    // For path-only caches, the key IS the path
    // For path:branch caches, the key starts with the path

    if let Ok(mut cache) = WORKTREE_CACHE.lock() {
        cache.retain_keys(active_project_paths);
    }
    if let Ok(mut cache) = GITHUB_URL_CACHE.lock() {
        cache.retain_keys(active_project_paths);
    }

    // PR and ahead/behind caches use "path:branch" keys
    // Build a set of prefixes that match active paths
    if let Ok(mut cache) = PR_INFO_CACHE.lock() {
        cache
            .map
            .retain(|k, _| active_project_paths.iter().any(|p| k.starts_with(p)));
    }
    if let Ok(mut cache) = AHEAD_BEHIND_CACHE.lock() {
        cache
            .map
            .retain(|k, _| active_project_paths.iter().any(|p| k.starts_with(p)));
    }
}

// ---------------------------------------------------------------------------
// Internal implementations
// ---------------------------------------------------------------------------

fn fetch_github_url(project_path: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(project_path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let remote_url = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Convert SSH format: git@github.com:user/repo.git -> https://github.com/user/repo
    if remote_url.starts_with("git@github.com:") {
        let path = remote_url
            .strip_prefix("git@github.com:")?
            .strip_suffix(".git")
            .unwrap_or(&remote_url[15..]);
        return Some(format!("https://github.com/{}", path));
    }

    // Already HTTPS: https://github.com/user/repo.git -> https://github.com/user/repo
    if remote_url.starts_with("https://github.com/") {
        let url = remote_url.strip_suffix(".git").unwrap_or(&remote_url);
        return Some(url.to_string());
    }

    None
}

fn check_is_worktree(project_path: &str) -> bool {
    let git_dir = Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(project_path)
        .output()
        .ok();

    let git_common_dir = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(project_path)
        .output()
        .ok();

    match (git_dir, git_common_dir) {
        (Some(dir), Some(common)) if dir.status.success() && common.status.success() => {
            let dir_str = String::from_utf8_lossy(&dir.stdout).trim().to_string();
            let common_str = String::from_utf8_lossy(&common.stdout).trim().to_string();
            // In a worktree, --git-dir points to worktrees/<name> while
            // --git-common-dir points to the main repo's .git directory
            dir_str != common_str
        }
        _ => false,
    }
}

fn fetch_ahead_behind(project_path: &str, branch: &str) -> Option<(u32, u32)> {
    // Try upstream first
    let output = Command::new("git")
        .args(["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])
        .current_dir(project_path)
        .output()
        .ok();

    let result = if let Some(ref o) = output {
        if o.status.success() {
            Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
        } else {
            None
        }
    } else {
        None
    };

    // Fallback to origin/<branch>
    let result = result.or_else(|| {
        let output = Command::new("git")
            .args([
                "rev-list",
                "--left-right",
                "--count",
                &format!("HEAD...origin/{}", branch),
            ])
            .current_dir(project_path)
            .output()
            .ok()?;

        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            None
        }
    });

    result.and_then(|s| {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() == 2 {
            let ahead = parts[0].parse::<u32>().ok()?;
            let behind = parts[1].parse::<u32>().ok()?;
            Some((ahead, behind))
        } else {
            None
        }
    })
}

/// JSON structures for parsing `gh pr view` output
#[derive(Debug, Deserialize)]
struct GhPrResponse {
    url: Option<String>,
    number: Option<u32>,
    state: Option<String>,
    #[serde(rename = "statusCheckRollup")]
    status_check_rollup: Option<Vec<GhCheckRun>>,
}

/// Represents both CheckRun and StatusContext from GitHub's statusCheckRollup.
/// CheckRun has: status, conclusion
/// StatusContext has: state, context
#[derive(Debug, Deserialize)]
struct GhCheckRun {
    status: Option<String>,
    conclusion: Option<String>,
    // StatusContext fields
    state: Option<String>,
}

fn fetch_pr_info(project_path: &str, branch: &str) -> Option<PrInfo> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            branch,
            "--json",
            "url,number,state,statusCheckRollup",
        ])
        .current_dir(project_path)
        .output()
        .ok()?;

    if !output.status.success() {
        debug!(
            "gh pr view failed for {} branch {}: {}",
            project_path,
            branch,
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return None;
    }

    let response: GhPrResponse = serde_json::from_slice(&output.stdout).ok()?;

    let ci_status = response.status_check_rollup.as_ref().map(|checks| {
        if checks.is_empty() {
            return CiStatus::Unknown;
        }

        // Normalize each check to a simple status.
        // CheckRun uses conclusion (SUCCESS/FAILURE/...) + status (COMPLETED/IN_PROGRESS/...)
        // StatusContext uses state (SUCCESS/PENDING/FAILURE/ERROR)
        let statuses: Vec<&str> = checks.iter().filter_map(|c| {
            // CheckRun: use conclusion if present
            if let Some(ref conclusion) = c.conclusion {
                return Some(conclusion.as_str());
            }
            // StatusContext: use state
            if let Some(ref state) = c.state {
                return Some(state.as_str());
            }
            // CheckRun with no conclusion yet — check status
            if let Some(ref status) = c.status {
                return Some(status.as_str());
            }
            None
        }).collect();

        if statuses.is_empty() {
            return CiStatus::Unknown;
        }

        let has_failure = statuses.iter().any(|s|
            matches!(*s, "FAILURE" | "ERROR" | "TIMED_OUT")
        );
        if has_failure {
            return CiStatus::Failure;
        }

        let has_pending = statuses.iter().any(|s|
            matches!(*s, "IN_PROGRESS" | "QUEUED" | "PENDING" | "WAITING")
        );
        if has_pending {
            return CiStatus::Pending;
        }

        let all_success = statuses.iter().all(|s|
            matches!(*s, "SUCCESS" | "NEUTRAL" | "SKIPPED" | "CANCELLED" | "COMPLETED")
        );
        if all_success {
            CiStatus::Success
        } else {
            CiStatus::Unknown
        }
    });

    Some(PrInfo {
        url: response.url?,
        number: response.number?,
        state: response.state.unwrap_or_else(|| "UNKNOWN".to_string()),
        ci_status,
    })
}
