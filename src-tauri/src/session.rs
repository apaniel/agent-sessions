use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use crate::process::{find_claude_processes, ClaudeProcess};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub project_name: String,
    pub project_path: String,
    pub git_branch: Option<String>,
    pub status: SessionStatus,
    pub last_message: Option<String>,
    pub last_message_role: Option<String>,
    pub last_activity_at: String,
    pub pid: u32,
    pub cpu_usage: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Waiting,
    Processing,
    Thinking,
    Idle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionsResponse {
    pub sessions: Vec<Session>,
    pub total_count: usize,
    pub waiting_count: usize,
}

#[derive(Debug, Deserialize)]
struct JsonlMessage {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "type")]
    msg_type: Option<String>,
    message: Option<MessageContent>,
}

#[derive(Debug, Deserialize)]
struct MessageContent {
    role: Option<String>,
    content: Option<serde_json::Value>,
}

/// Check if message content contains a tool_use block
fn has_tool_use(content: &serde_json::Value) -> bool {
    if let serde_json::Value::Array(arr) = content {
        arr.iter().any(|item| {
            item.get("type")
                .and_then(|t| t.as_str())
                .map(|t| t == "tool_use")
                .unwrap_or(false)
        })
    } else {
        false
    }
}

/// Check if message content contains a tool_result block
fn has_tool_result(content: &serde_json::Value) -> bool {
    if let serde_json::Value::Array(arr) = content {
        arr.iter().any(|item| {
            item.get("type")
                .and_then(|t| t.as_str())
                .map(|t| t == "tool_result")
                .unwrap_or(false)
        })
    } else {
        false
    }
}

/// Check if message content is a local slash command that doesn't trigger Claude response
/// These commands are handled locally by Claude Code and don't require thinking
fn is_local_slash_command(content: &serde_json::Value) -> bool {
    let text = match content {
        serde_json::Value::String(s) => s.as_str(),
        serde_json::Value::Array(arr) => {
            // Find first text block
            arr.iter().find_map(|v| {
                v.get("text").and_then(|t| t.as_str())
            }).unwrap_or("")
        }
        _ => return false,
    };

    let trimmed = text.trim();

    // Local commands that don't trigger Claude to think
    // These are handled by the CLI itself
    let local_commands = [
        "/clear",
        "/compact",
        "/help",
        "/config",
        "/cost",
        "/doctor",
        "/init",
        "/login",
        "/logout",
        "/memory",
        "/model",
        "/permissions",
        "/pr-comments",
        "/review",
        "/status",
        "/terminal-setup",
        "/vim",
    ];

    local_commands.iter().any(|cmd| {
        trimmed == *cmd || trimmed.starts_with(&format!("{} ", cmd))
    })
}

/// Convert a directory name like "-Users-ozan-Projects-ai-image-dashboard" back to a path
/// The challenge is that both path separators AND project names can contain dashes
/// We handle this by recognizing that the path structure is predictable:
/// /Users/<username>/Projects/<project-name> or /Users/<username>/.../<project-name>
fn convert_dir_name_to_path(dir_name: &str) -> String {
    // Remove leading dash if present
    let name = dir_name.strip_prefix('-').unwrap_or(dir_name);

    // Split by dash
    let parts: Vec<&str> = name.split('-').collect();

    if parts.is_empty() {
        return String::new();
    }

    // Find "Projects" or "UnityProjects" index - everything after that is the project name
    let projects_idx = parts.iter().position(|&p| p == "Projects" || p == "UnityProjects");

    if let Some(idx) = projects_idx {
        // Path components are before and including "Projects"
        let path_parts = &parts[..=idx];
        // Project name is everything after "Projects", joined with dashes
        let project_parts = &parts[idx + 1..];

        let mut path = String::from("/");
        path.push_str(&path_parts.join("/"));

        if !project_parts.is_empty() {
            path.push('/');
            path.push_str(&project_parts.join("-"));
        }

        path
    } else {
        // Fallback: just replace dashes with slashes (old behavior)
        format!("/{}", name.replace('-', "/"))
    }
}

pub fn get_sessions() -> SessionsResponse {
    let claude_processes = find_claude_processes();
    let mut sessions = Vec::new();

    // Build a map of cwd -> list of processes (multiple sessions can run in same folder)
    let mut cwd_to_processes: HashMap<String, Vec<&ClaudeProcess>> = HashMap::new();
    for process in &claude_processes {
        if let Some(cwd) = &process.cwd {
            let cwd_str = cwd.to_string_lossy().to_string();
            cwd_to_processes.entry(cwd_str).or_default().push(process);
        }
    }

    // Scan ~/.claude/projects for session files
    let claude_dir = dirs::home_dir()
        .map(|h| h.join(".claude").join("projects"))
        .unwrap_or_default();

    if !claude_dir.exists() {
        return SessionsResponse {
            sessions: vec![],
            total_count: 0,
            waiting_count: 0,
        };
    }

    // For each project directory
    if let Ok(entries) = fs::read_dir(&claude_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            // Convert directory name back to path
            let dir_name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            let project_path = convert_dir_name_to_path(dir_name);

            // Check if this project has active Claude processes
            let processes = match cwd_to_processes.get(&project_path) {
                Some(p) => p,
                None => continue, // Skip projects without active processes
            };

            // Find all JSONL files that were recently modified (within last 30 seconds)
            // These are likely the active sessions
            let jsonl_files = get_recently_active_jsonl_files(&path, processes.len());

            // Match processes to JSONL files
            for (index, process) in processes.iter().enumerate() {
                if let Some(session) = find_session_for_process(&jsonl_files, &project_path, process, index) {
                    sessions.push(session);
                }
            }
        }
    }

    // Sort by status priority first, then by most recent activity within same priority
    // Priority: Waiting (needs attention) > Thinking/Processing (active) > Idle
    // Within same priority, sort by most recent activity
    sessions.sort_by(|a, b| {
        let priority_a = status_sort_priority(&a.status);
        let priority_b = status_sort_priority(&b.status);

        if priority_a != priority_b {
            priority_a.cmp(&priority_b)
        } else {
            b.last_activity_at.cmp(&a.last_activity_at)
        }
    });

    let waiting_count = sessions.iter()
        .filter(|s| matches!(s.status, SessionStatus::Waiting))
        .count();
    let total_count = sessions.len();

    SessionsResponse {
        sessions,
        total_count,
        waiting_count,
    }
}

/// Get JSONL files that are likely active sessions
/// Takes the expected count of active processes and returns the most recently modified files
fn get_recently_active_jsonl_files(project_dir: &PathBuf, expected_count: usize) -> Vec<PathBuf> {
    use std::time::{Duration, SystemTime};

    let now = SystemTime::now();
    let recent_threshold = Duration::from_secs(60); // Consider files modified in last 60 seconds as potentially active

    let mut jsonl_files: Vec<_> = fs::read_dir(project_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.path().extension()
                .map(|ext| ext == "jsonl")
                .unwrap_or(false)
        })
        .filter_map(|e| {
            let path = e.path();
            let modified = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((path, modified))
        })
        .collect();

    // Sort by modification time (newest first)
    jsonl_files.sort_by(|a, b| b.1.cmp(&a.1));

    // First, try to get recently modified files (within threshold)
    let recent_files: Vec<PathBuf> = jsonl_files
        .iter()
        .filter(|(_, modified)| {
            now.duration_since(*modified)
                .map(|d| d < recent_threshold)
                .unwrap_or(false)
        })
        .map(|(path, _)| path.clone())
        .collect();

    // If we have enough recent files, use those
    if recent_files.len() >= expected_count {
        return recent_files.into_iter().take(expected_count).collect();
    }

    // Otherwise, just take the N most recently modified files
    jsonl_files
        .into_iter()
        .take(expected_count)
        .map(|(path, _)| path)
        .collect()
}

/// Find a session for a specific process from available JSONL files
/// Takes the index to pick different files for different processes
fn find_session_for_process(jsonl_files: &[PathBuf], project_path: &str, process: &ClaudeProcess, index: usize) -> Option<Session> {
    // Get the JSONL file at the given index (they're sorted by most recent first)
    let jsonl_path = jsonl_files.get(index)?;
    parse_session_file(jsonl_path, project_path, process)
}

fn parse_session_file(jsonl_path: &PathBuf, project_path: &str, process: &ClaudeProcess) -> Option<Session> {
    use std::time::{Duration, SystemTime};

    // Check if the file was modified very recently (indicates active processing)
    let file_recently_modified = jsonl_path
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .map(|d| d < Duration::from_secs(3))
        .unwrap_or(false);

    // Parse the JSONL file to get session info
    let file = File::open(jsonl_path).ok()?;
    let reader = BufReader::new(file);

    let mut session_id = None;
    let mut git_branch = None;
    let mut last_timestamp = None;
    let mut last_message = None;
    let mut last_role = None;
    let mut last_msg_type = None;
    let mut last_has_tool_use = false;
    let mut last_has_tool_result = false;
    let mut last_is_local_command = false;
    let mut found_status_info = false;

    // Read last N lines for efficiency
    let lines: Vec<_> = reader.lines().flatten().collect();
    let recent_lines: Vec<_> = lines.iter().rev().take(100).collect();

    for line in &recent_lines {
        if let Ok(msg) = serde_json::from_str::<JsonlMessage>(line) {
            if session_id.is_none() {
                session_id = msg.session_id;
            }
            if git_branch.is_none() {
                git_branch = msg.git_branch;
            }
            if last_timestamp.is_none() {
                last_timestamp = msg.timestamp;
            }

            // For status detection, we need to find the most recent message that has CONTENT
            if !found_status_info {
                if let Some(content) = &msg.message {
                    if let Some(c) = &content.content {
                        let has_content = match c {
                            serde_json::Value::String(s) => !s.is_empty(),
                            serde_json::Value::Array(arr) => !arr.is_empty(),
                            _ => false,
                        };

                        if has_content {
                            last_msg_type = msg.msg_type.clone();
                            last_role = content.role.clone();
                            last_has_tool_use = has_tool_use(c);
                            last_has_tool_result = has_tool_result(c);
                            last_is_local_command = is_local_slash_command(c);
                            found_status_info = true;
                        }
                    }
                }
            }

            if session_id.is_some() && found_status_info {
                break;
            }
        }
    }

    // Now find the last meaningful text message (keep looking even after finding status)
    for line in &recent_lines {
        if let Ok(msg) = serde_json::from_str::<JsonlMessage>(line) {
            if let Some(content) = &msg.message {
                if let Some(c) = &content.content {
                    let text = match c {
                        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
                        serde_json::Value::Array(arr) => {
                            arr.iter().find_map(|v| {
                                v.get("text").and_then(|t| t.as_str())
                                    .filter(|s| !s.is_empty())
                                    .map(String::from)
                            })
                        }
                        _ => None,
                    };

                    if text.is_some() {
                        last_message = text;
                        break;
                    }
                }
            }
        }
    }

    let session_id = session_id?;

    // Determine status based on message type, content, and file activity
    let status = determine_status(
        last_msg_type.as_deref(),
        last_has_tool_use,
        last_has_tool_result,
        last_is_local_command,
        file_recently_modified,
    );

    // Extract project name from path
    let project_name = project_path
        .split('/')
        .filter(|s| !s.is_empty())
        .last()
        .unwrap_or("Unknown")
        .to_string();

    // Truncate message for preview
    let last_message = last_message.map(|m| {
        if m.len() > 100 {
            format!("{}...", &m[..100])
        } else {
            m
        }
    });

    Some(Session {
        id: session_id,
        project_name,
        project_path: project_path.to_string(),
        git_branch,
        status,
        last_message,
        last_message_role: last_role,
        last_activity_at: last_timestamp.unwrap_or_else(|| "Unknown".to_string()),
        pid: process.pid,
        cpu_usage: process.cpu_usage,
    })
}

/// Returns sort priority for status (lower = higher priority in list)
/// Active sessions (thinking/processing) appear first, then waiting, then idle
fn status_sort_priority(status: &SessionStatus) -> u8 {
    match status {
        SessionStatus::Thinking => 0,   // Active - Claude is working - show first
        SessionStatus::Processing => 0, // Active - tool is running - show first
        SessionStatus::Waiting => 1,    // Needs attention - show second
        SessionStatus::Idle => 2,       // Inactive - show last
    }
}

fn determine_status(
    last_msg_type: Option<&str>,
    has_tool_use: bool,
    has_tool_result: bool,
    is_local_command: bool,
    file_recently_modified: bool,
) -> SessionStatus {
    // Determine status based on the last message in the conversation:
    // - If file is being actively modified (within last 3s) -> active state (Thinking or Processing)
    // - If last message is user with tool_result -> Processing (tool just ran, Claude processing result)
    // - If last message is from assistant with tool_use -> Processing (tool is being executed)
    // - If last message is from assistant with only text -> Waiting (Claude finished, waiting for user)
    // - If last message is from user -> Thinking (Claude is generating a response)
    // - If last message is a local slash command (/clear, /help, etc.) -> Waiting (these don't trigger Claude)

    // Key insight: if the file was modified very recently, Claude is actively working
    // and we should NOT show "Waiting" even if the last written message was assistant text

    match last_msg_type {
        Some("assistant") => {
            if has_tool_use {
                // Assistant sent a tool_use, tool is executing
                SessionStatus::Processing
            } else if file_recently_modified {
                // File is being actively modified but last message is text
                // Claude is likely still streaming or about to send more
                SessionStatus::Thinking
            } else {
                // Assistant sent a text response and file is stable - waiting for user
                SessionStatus::Waiting
            }
        }
        Some("user") => {
            if is_local_command {
                // Local slash commands like /clear, /help, /compact don't trigger Claude
                // Session is waiting for actual user input
                SessionStatus::Waiting
            } else if has_tool_result {
                // User message contains tool_result - tool execution complete,
                // Claude is processing the result
                if file_recently_modified {
                    SessionStatus::Thinking
                } else {
                    // Tool result was sent but file not recently modified
                    // This can happen if tool result was recent - show Processing
                    SessionStatus::Processing
                }
            } else {
                // Regular user input, Claude is thinking/generating response
                SessionStatus::Thinking
            }
        }
        _ => {
            // No recognized message type - check if file is active
            if file_recently_modified {
                SessionStatus::Thinking
            } else {
                SessionStatus::Idle
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_dir_name_to_path() {
        // Test basic project path
        assert_eq!(
            convert_dir_name_to_path("-Users-ozan-Projects-ai-image-dashboard"),
            "/Users/ozan/Projects/ai-image-dashboard"
        );

        // Test project with multiple dashes
        assert_eq!(
            convert_dir_name_to_path("-Users-ozan-Projects-backend-service-generator-ai"),
            "/Users/ozan/Projects/backend-service-generator-ai"
        );

        // Test UnityProjects
        assert_eq!(
            convert_dir_name_to_path("-Users-ozan-UnityProjects-my-game"),
            "/Users/ozan/UnityProjects/my-game"
        );

        // Test worktree paths (with double dashes)
        assert_eq!(
            convert_dir_name_to_path("-Users-ozan-Projects-ai-image-dashboard--rsworktree-feature"),
            "/Users/ozan/Projects/ai-image-dashboard--rsworktree-feature"
        );

        // Test just Projects folder
        assert_eq!(
            convert_dir_name_to_path("-Users-ozan-Projects"),
            "/Users/ozan/Projects"
        );
    }
}
