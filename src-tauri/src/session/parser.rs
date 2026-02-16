use log::{debug, info, trace, warn};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Mutex;
use once_cell::sync::Lazy;

use crate::agent::AgentProcess;
use crate::terminal::detect_terminal_for_pid;
use super::model::{AgentType, Session, SessionStatus, SessionsResponse, JsonlMessage, TerminalApp};
use super::git;
use super::status::{determine_status, has_tool_use, has_tool_result, is_local_slash_command, is_interrupted_request, is_thinking_only, status_sort_priority};

/// Track previous status for each session to detect transitions
static PREVIOUS_STATUS: Lazy<Mutex<HashMap<String, SessionStatus>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Clean up PREVIOUS_STATUS entries for sessions that no longer exist.
/// Call this after all agent detectors have run to prevent unbounded memory growth.
pub fn cleanup_stale_status_entries(active_session_ids: &std::collections::HashSet<String>) {
    let mut prev_status_map = PREVIOUS_STATUS.lock().unwrap();
    let before_count = prev_status_map.len();
    prev_status_map.retain(|id, _| active_session_ids.contains(id));
    let removed = before_count - prev_status_map.len();
    if removed > 0 {
        debug!("Cleaned up {} stale entries from PREVIOUS_STATUS (kept {})", removed, prev_status_map.len());
    }
}

/// Extract a preview of content for debugging
fn get_content_preview(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => {
            let preview: String = s.chars().take(100).collect();
            format!("text: \"{}{}\"", preview, if s.len() > 100 { "..." } else { "" })
        }
        serde_json::Value::Array(arr) => {
            let types: Vec<String> = arr.iter()
                .filter_map(|v| v.get("type").and_then(|t| t.as_str()).map(String::from))
                .collect();
            format!("blocks: [{}]", types.join(", "))
        }
        _ => "unknown".to_string(),
    }
}

/// Convert a file system path like "/Users/ozan/Projects/my-project" to a directory name
/// This is the reverse of convert_dir_name_to_path
/// e.g., "/Users/ozan/Projects/my-project/.rsworktree/branch-name" -> "-Users-ozan-Projects-my-project--rsworktree-branch-name"
pub fn convert_path_to_dir_name(path: &str) -> String {
    // Remove leading slash and replace path separators with dashes
    let path = path.strip_prefix('/').unwrap_or(path);

    let mut result = String::from("-");
    let mut chars = path.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '/' => {
                // Check if next char starts a hidden folder (.)
                if chars.peek() == Some(&'.') {
                    // Hidden folder: use double dash and skip the dot
                    result.push('-');
                    result.push('-');
                    chars.next(); // skip the dot
                } else {
                    result.push('-');
                }
            }
            _ => result.push(c),
        }
    }

    result
}

/// Convert a directory name like "-Users-ozan-Repos-cleanernative-reskin-2" back to a path.
///
/// The challenge is that both path separators AND directory names can contain dashes.
/// We resolve this by walking the parts left-to-right and checking the filesystem:
/// if `/path/so/far/next_part` exists as a directory, treat the dash as a path separator;
/// otherwise, join the remaining parts with dashes as the leaf directory name.
///
/// Special case: Double dashes (--) indicate a hidden folder (starting with .)
/// e.g., "project--rsworktree-branch" becomes "project/.rsworktree/branch"
pub fn convert_dir_name_to_path(dir_name: &str) -> String {
    // Remove leading dash if present
    let name = dir_name.strip_prefix('-').unwrap_or(dir_name);

    // First handle double-dash (hidden folder) segments by splitting on "--"
    let segments: Vec<&str> = name.split("--").collect();

    // Process the first segment (before any hidden folder) with filesystem probing
    let first_segment = segments[0];
    let base_path = resolve_segment_with_fs(first_segment);

    if segments.len() == 1 {
        return base_path;
    }

    // Handle hidden folder segments: each "--" introduces a dot-prefixed folder
    // and subsequent dashes within that segment are subfolder separators
    let mut path = base_path;
    for hidden_segment in &segments[1..] {
        let sub_parts: Vec<&str> = hidden_segment.split('-').collect();
        if let Some((first, rest)) = sub_parts.split_first() {
            path.push_str(&format!("/.{}", first));
            for part in rest {
                path.push('/');
                path.push_str(part);
            }
        }
    }

    path
}

/// Resolve a dash-separated segment into a filesystem path by probing which
/// prefixes exist as directories. Once a prefix doesn't exist, the remaining
/// parts are joined with dashes as the leaf name.
fn resolve_segment_with_fs(segment: &str) -> String {
    let parts: Vec<&str> = segment.split('-').collect();

    if parts.is_empty() {
        return String::new();
    }

    // Walk left-to-right, checking if each prefix exists as a directory
    let mut confirmed_path = format!("/{}", parts[0]);
    let mut last_valid_idx = 0;

    for i in 1..parts.len() {
        let candidate = format!("{}/{}", confirmed_path, parts[i]);
        if std::path::Path::new(&candidate).is_dir() {
            confirmed_path = candidate;
            last_valid_idx = i;
        } else {
            break;
        }
    }

    // Everything after last_valid_idx is the leaf directory name (joined with dashes)
    if last_valid_idx < parts.len() - 1 {
        let leaf = parts[last_valid_idx + 1..].join("-");
        format!("{}/{}", confirmed_path, leaf)
    } else {
        confirmed_path
    }
}

/// Get all active Claude Code sessions (delegates to agent module)
pub fn get_sessions() -> SessionsResponse {
    crate::agent::get_all_sessions()
}

/// Internal function to get sessions for a specific agent type
/// Called by agent detectors (ClaudeDetector, OpenCodeDetector, etc.)
pub fn get_sessions_internal(processes: &[AgentProcess], agent_type: AgentType) -> Vec<Session> {
    info!("=== Getting sessions for {:?} ===", agent_type);
    debug!("Found {} processes total", processes.len());

    let mut sessions = Vec::new();

    // Build a map of cwd -> list of processes (multiple sessions can run in same folder)
    let mut cwd_to_processes: HashMap<String, Vec<&AgentProcess>> = HashMap::new();
    for process in processes {
        if let Some(cwd) = &process.cwd {
            let cwd_str = cwd.to_string_lossy().to_string();
            debug!("Mapping process pid={} to cwd={}", process.pid, cwd_str);
            cwd_to_processes.entry(cwd_str).or_default().push(process);
        } else {
            warn!("Process pid={} has no cwd, skipping", process.pid);
        }
    }

    // Scan ~/.claude/projects for session files
    let claude_dir = dirs::home_dir()
        .map(|h| h.join(".claude").join("projects"))
        .unwrap_or_default();

    debug!("Claude projects directory: {:?}", claude_dir);

    if !claude_dir.exists() {
        warn!("Claude projects directory does not exist: {:?}", claude_dir);
        return sessions;
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

            let mut project_path = convert_dir_name_to_path(dir_name);
            debug!("Checking project: {} -> {}", dir_name, project_path);

            // Check if this project has active processes
            // First try exact match
            let matching_processes = if let Some(p) = cwd_to_processes.get(&project_path) {
                debug!("Project {} has {} active processes (exact match)", project_path, p.len());
                p
            } else {
                // Try to find a matching cwd by converting each cwd to a dir name and comparing
                let matching_cwd = cwd_to_processes.keys().find(|cwd| {
                    let cwd_as_dir = convert_path_to_dir_name(cwd);
                    cwd_as_dir == dir_name
                });

                match matching_cwd {
                    Some(cwd) => {
                        debug!("Project {} matched via reverse lookup to cwd {}", dir_name, cwd);
                        // Use the actual cwd as project_path since the decoded path may be wrong
                        // (e.g. "homeaglowpub-cp-reskin" decoded as "homeaglowpub/cp-reskin")
                        project_path = cwd.clone();
                        cwd_to_processes.get(cwd).unwrap()
                    }
                    None => {
                        trace!("Project {} has no active processes, skipping", project_path);
                        continue;
                    }
                }
            };

            // Find all JSONL files that were recently modified (within last 30 seconds)
            // These are likely the active sessions
            let jsonl_files = get_recently_active_jsonl_files(&path, matching_processes.len());
            debug!("Found {} JSONL files for project {}", jsonl_files.len(), project_path);

            // Match processes to JSONL files
            // Use lsof to correctly match PIDs to their session files when multiple
            // processes share the same project directory (prevents status cross-contamination)
            let pid_to_jsonl = if matching_processes.len() > 1 {
                match_processes_to_files_by_time(matching_processes, &jsonl_files)
            } else {
                HashMap::new()
            };

            let assigned_count = matching_processes.len();
            let mut used_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();
            for process in matching_processes.iter() {
                // Try lsof-based matching first, fall back to first unassigned index
                let file_index = if let Some(matched_path) = pid_to_jsonl.get(&process.pid) {
                    let idx = jsonl_files.iter().position(|f| f == matched_path);
                    if let Some(i) = idx {
                        debug!("PID {} matched to JSONL file index {} via lsof", process.pid, i);
                    }
                    idx
                } else {
                    None
                };

                let file_index = match file_index {
                    Some(i) => i,
                    None => {
                        let fallback = (0..jsonl_files.len())
                            .find(|i| !used_indices.contains(i))
                            .unwrap_or(0);
                        debug!("PID {} falling back to JSONL file index {}", process.pid, fallback);
                        fallback
                    }
                };

                used_indices.insert(file_index);

                debug!("Matching process pid={} to JSONL file index {}", process.pid, file_index);
                if let Some(session) = find_session_for_process(&jsonl_files, &path, &project_path, process, file_index, agent_type.clone(), assigned_count) {
                    // Track status transitions
                    let mut prev_status_map = PREVIOUS_STATUS.lock().unwrap();
                    let prev_status = prev_status_map.get(&session.id).cloned();

                    // Log status transition if it changed
                    if let Some(prev) = &prev_status {
                        if *prev != session.status {
                            warn!(
                                "STATUS TRANSITION: project={}, {:?} -> {:?}, cpu={:.1}%, file_age=?, last_msg_role={:?}",
                                session.project_name, prev, session.status, session.cpu_usage, session.last_message_role
                            );
                        }
                    }

                    // Update stored status
                    prev_status_map.insert(session.id.clone(), session.status.clone());
                    drop(prev_status_map);

                    info!(
                        "Session created: id={}, project={}, status={:?}, pid={}, cpu={:.1}%",
                        session.id, session.project_name, session.status, session.pid, session.cpu_usage
                    );
                    sessions.push(session);
                } else {
                    warn!("Failed to create session for process pid={} in project {}", process.pid, project_path);
                }
            }
        }
    }

    info!(
        "=== Session scan complete for {:?}: {} total ===",
        agent_type, sessions.len()
    );

    sessions
}

/// Check if a JSONL file is a subagent file (named agent-*.jsonl)
fn is_subagent_file(path: &PathBuf) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|name| name.starts_with("agent-") && name.ends_with(".jsonl"))
        .unwrap_or(false)
}

/// Count active subagents for a given parent session.
/// Subagent files live in <project_dir>/<session_id>/subagents/agent-*.jsonl
fn count_active_subagents(project_dir: &PathBuf, parent_session_id: &str) -> usize {
    use std::time::{Duration, SystemTime};

    let subagents_dir = project_dir.join(parent_session_id).join("subagents");
    if !subagents_dir.exists() {
        trace!("No subagents directory for session {}", parent_session_id);
        return 0;
    }

    let active_threshold = Duration::from_secs(30);
    let now = SystemTime::now();

    let count = fs::read_dir(&subagents_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| is_subagent_file(&e.path()))
        .filter(|e| {
            // Check if file was recently modified
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .map(|d| d < active_threshold)
                .unwrap_or(false)
        })
        .count();

    trace!("Found {} active subagents for session {} in {:?}", count, parent_session_id, subagents_dir);
    count
}

/// Get JSONL files for a project, sorted by modification time (newest first)
/// Excludes subagent files (agent-*.jsonl) as they are counted separately
fn get_recently_active_jsonl_files(project_dir: &PathBuf, _expected_count: usize) -> Vec<PathBuf> {
    let mut jsonl_files: Vec<_> = fs::read_dir(project_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let path = e.path();
            path.extension()
                .map(|ext| ext == "jsonl")
                .unwrap_or(false)
                && !is_subagent_file(&path)
        })
        .filter_map(|e| {
            let path = e.path();
            let modified = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((path, modified))
        })
        .collect();

    // Sort by modification time (newest first)
    jsonl_files.sort_by(|a, b| b.1.cmp(&a.1));

    jsonl_files
        .into_iter()
        .map(|(path, _)| path)
        .collect()
}

/// Match process PIDs to their JSONL session files by correlating process
/// start times with file creation times. When a Claude session starts, both
/// the process and its JSONL file are created at roughly the same time.
/// Only needed when multiple processes share the same project directory.
fn match_processes_to_files_by_time(
    processes: &[&AgentProcess],
    candidate_files: &[PathBuf],
) -> HashMap<u32, PathBuf> {
    use std::time::UNIX_EPOCH;

    let mut result = HashMap::new();

    if processes.len() < 2 || candidate_files.is_empty() {
        return result;
    }

    // Get file creation times (birth time on macOS)
    let file_times: Vec<(usize, u64)> = candidate_files
        .iter()
        .enumerate()
        .filter_map(|(idx, path)| {
            let created = path.metadata().ok()?.created().ok()?;
            let secs = created.duration_since(UNIX_EPOCH).ok()?.as_secs();
            Some((idx, secs))
        })
        .collect();

    if file_times.is_empty() {
        return result;
    }

    debug!(
        "Matching {} processes to {} files by timestamp",
        processes.len(),
        file_times.len()
    );

    for p in processes.iter() {
        debug!("  Process PID {} start_time={}", p.pid, p.start_time);
    }
    for (idx, secs) in &file_times {
        debug!(
            "  File [{}] {:?} created_at={}",
            idx,
            candidate_files[*idx].file_name().unwrap_or_default(),
            secs
        );
    }

    // Greedy matching: for each process, find the file with the closest
    // creation time. Assign the closest pair first to avoid conflicts.
    // Build all (process_idx, file_idx, distance) pairs and sort by distance.
    let mut pairs: Vec<(usize, usize, u64)> = Vec::new();
    for (pi, proc) in processes.iter().enumerate() {
        for &(fi, file_time) in &file_times {
            let distance = if proc.start_time > file_time {
                proc.start_time - file_time
            } else {
                file_time - proc.start_time
            };
            pairs.push((pi, fi, distance));
        }
    }
    pairs.sort_by_key(|&(_, _, d)| d);

    let mut assigned_processes: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut assigned_files: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for (pi, fi, distance) in pairs {
        if assigned_processes.contains(&pi) || assigned_files.contains(&fi) {
            continue;
        }

        let proc = processes[pi];
        let file_path = &candidate_files[fi];

        debug!(
            "Matched PID {} (start={}) -> {:?} (created={}), distance={}s",
            proc.pid,
            proc.start_time,
            file_path.file_name().unwrap_or_default(),
            file_times.iter().find(|(i, _)| *i == fi).map(|(_, t)| *t).unwrap_or(0),
            distance
        );

        result.insert(proc.pid, file_path.clone());
        assigned_processes.insert(pi);
        assigned_files.insert(fi);
    }

    info!(
        "PID-to-JSONL matching: {}/{} processes matched by timestamp",
        result.len(),
        processes.len()
    );

    result
}

/// Find a session for a specific process from available JSONL files
/// Checks unassigned recent files and uses the most "active" status found
fn find_session_for_process(
    jsonl_files: &[PathBuf],
    project_dir: &PathBuf,
    project_path: &str,
    process: &AgentProcess,
    index: usize,
    agent_type: AgentType,
    assigned_count: usize,
) -> Option<Session> {
    use std::time::{Duration, SystemTime};

    // Get the primary JSONL file at the given index
    let primary_jsonl = jsonl_files.get(index)?;

    // Parse the primary file first
    let mut session = parse_session_file(primary_jsonl, project_path, process.pid, process.cpu_usage, agent_type.clone())?;

    // Count active subagents for this session
    session.active_subagent_count = count_active_subagents(project_dir, &session.id);

    // If there are active subagents, the session is processing (not waiting for user input).
    // The main JSONL file goes quiet when a subagent runs (activity is in agent-*.jsonl),
    // so status logic would otherwise think we're waiting/idle.
    if session.active_subagent_count > 0
        && matches!(session.status, SessionStatus::Waiting | SessionStatus::Idle)
    {
        debug!(
            "Overriding {:?} -> Processing: {} active subagents for session {}",
            session.status, session.active_subagent_count, session.id
        );
        session.status = SessionStatus::Processing;
    }

    // Check if any unassigned recent files show more active status
    // Only check files NOT already assigned to another process (index >= assigned_count)
    // Files at indices 0..assigned_count are each assigned to a specific process
    let now = SystemTime::now();
    let active_threshold = Duration::from_secs(10); // Check files modified in last 10 seconds

    for (file_idx, jsonl_path) in jsonl_files.iter().enumerate() {
        if jsonl_path == primary_jsonl {
            continue;
        }

        // Skip files assigned to other processes to prevent cross-contamination
        if file_idx < assigned_count {
            continue;
        }

        // Only check recently modified files
        let is_recent = jsonl_path
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .map(|d| d < active_threshold)
            .unwrap_or(false);

        if !is_recent {
            continue;
        }

        // Parse this file and check its status
        if let Some(other_session) = parse_session_file(jsonl_path, project_path, process.pid, process.cpu_usage, agent_type.clone()) {
            // If this file shows a more active status, use it
            let current_priority = status_sort_priority(&session.status);
            let other_priority = status_sort_priority(&other_session.status);

            if other_priority < current_priority {
                debug!(
                    "Found more active status in {:?}: {:?} -> {:?}",
                    jsonl_path, session.status, other_session.status
                );
                session.status = other_session.status;
                // Keep the original session's other fields (id, last_message, etc.)
            }
        }
    }

    Some(session)
}

/// Parse a JSONL session file and create a Session struct
pub fn parse_session_file(
    jsonl_path: &PathBuf,
    project_path: &str,
    pid: u32,
    cpu_usage: f32,
    agent_type: AgentType,
) -> Option<Session> {
    use std::time::SystemTime;

    debug!("Parsing JSONL file: {:?}", jsonl_path);

    // Check if the file was modified very recently (indicates active processing)
    let file_age_secs = jsonl_path
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .map(|d| d.as_secs_f32());

    debug!(
        "File age: {:.1}s",
        file_age_secs.unwrap_or(-1.0),
    );

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
    let mut last_is_interrupted = false;
    let mut found_status_info = false;
    let mut is_compacting = false;
    let mut last_usage = None;

    // Read last N lines for efficiency
    // Must be large enough to cover long stretches of progress entries during tool execution
    // (observed up to 275 consecutive non-content lines in real sessions)
    let lines: Vec<_> = reader.lines().flatten().collect();
    let recent_lines: Vec<_> = lines.iter().rev().take(500).collect();

    trace!("File has {} total lines, checking last {}", lines.len(), recent_lines.len());

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
            if last_usage.is_none() {
                if let Some(ref message) = msg.message {
                    if let Some(usage) = &message.usage {
                        last_usage = Some(super::model::TokenUsage {
                            input_tokens: usage.input_tokens,
                            cache_creation_input_tokens: usage.cache_creation_input_tokens,
                            cache_read_input_tokens: usage.cache_read_input_tokens,
                        });
                    }
                }
            }

            // Detect compaction: if we see compact_boundary before any content message
            // or isCompactSummary, the session is currently compacting.
            // Reading from newest to oldest: if compact_boundary comes first → compacting
            // If isCompactSummary comes first → compaction already finished
            if !found_status_info && !is_compacting {
                if msg.is_compact_summary == Some(true) {
                    // Compaction finished, summary already written - not compacting
                    // Continue to find status info normally
                } else if msg.subtype.as_deref() == Some("compact_boundary") {
                    is_compacting = true;
                    debug!("Detected active compaction (compact_boundary before any content)");
                }
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

                        if has_content && !is_thinking_only(c) {
                            last_msg_type = msg.msg_type.clone();
                            last_role = content.role.clone();
                            last_has_tool_use = has_tool_use(c);
                            last_has_tool_result = has_tool_result(c);
                            last_is_local_command = is_local_slash_command(c);
                            last_is_interrupted = is_interrupted_request(c);
                            found_status_info = true;

                            // Enhanced logging with content preview
                            let content_preview = get_content_preview(c);
                            debug!(
                                "Found status info: type={:?}, role={:?}, has_tool_use={}, has_tool_result={}, is_local_cmd={}, is_interrupted={}, content={}",
                                last_msg_type, last_role, last_has_tool_use, last_has_tool_result, last_is_local_command, last_is_interrupted, content_preview
                            );
                        }
                    }
                }
            }

            if session_id.is_some() && found_status_info && last_usage.is_some() {
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

    // Determine status using message content + file age + CPU usage
    let status = if is_compacting {
        SessionStatus::Compacting
    } else {
        determine_status(
            last_msg_type.as_deref(),
            last_has_tool_use,
            last_has_tool_result,
            last_is_local_command,
            last_is_interrupted,
            file_age_secs,
            cpu_usage,
        )
    };

    debug!(
        "Status determination: type={:?}, tool_use={}, tool_result={}, local_cmd={}, interrupted={}, compacting={}, file_age={:.1}s, cpu={:.1}% -> {:?}",
        last_msg_type, last_has_tool_use, last_has_tool_result, last_is_local_command, last_is_interrupted, is_compacting, file_age_secs.unwrap_or(-1.0), cpu_usage, status
    );

    // Extract project name from path
    let project_name = project_path
        .split('/')
        .filter(|s| !s.is_empty())
        .last()
        .unwrap_or("Unknown")
        .to_string();

    // Truncate message for preview (respecting UTF-8 char boundaries)
    let last_message = last_message.map(|m| {
        if m.chars().count() > 100 {
            format!("{}...", m.chars().take(100).collect::<String>())
        } else {
            m
        }
    });

    // Git enrichment (cached in git.rs)
    let github_url = git::get_github_url(project_path);
    let repo_name = git::get_repo_name(&github_url);
    let is_worktree = git::is_worktree(project_path);

    let (pr_info, commits_ahead, commits_behind) = if let Some(ref branch) = git_branch {
        let pr = git::get_pr_info(project_path, branch);
        let ab = git::get_ahead_behind(project_path, branch);
        let (ahead, behind) = ab.map(|(a, b)| (Some(a), Some(b))).unwrap_or((None, None));
        (pr, ahead, behind)
    } else {
        (None, None, None)
    };

    // Context window remaining % (how much is left before compression)
    let context_window_percent = last_usage.and_then(|u| {
        let input = u.input_tokens.unwrap_or(0)
            + u.cache_creation_input_tokens.unwrap_or(0)
            + u.cache_read_input_tokens.unwrap_or(0);
        if input > 0 {
            let used_pct = (input as f32 / 200_000.0) * 100.0;
            Some((100.0 - used_pct).max(0.0))
        } else {
            None
        }
    });

    let detected = detect_terminal_for_pid(pid);
    info!("Terminal detection for pid={}: {:?}", pid, detected);
    let terminal_app = match detected.as_str() {
        "iterm2" => TerminalApp::Iterm2,
        "warp" => TerminalApp::Warp,
        "cursor" => TerminalApp::Cursor,
        "vscode" => TerminalApp::Vscode,
        "terminal" => TerminalApp::Terminal,
        "tmux" => TerminalApp::Tmux,
        _ => TerminalApp::Unknown,
    };

    Some(Session {
        id: session_id,
        agent_type,
        project_name,
        project_path: project_path.to_string(),
        git_branch,
        github_url,
        status,
        last_message,
        last_message_role: last_role,
        last_activity_at: last_timestamp.unwrap_or_else(|| "Unknown".to_string()),
        pid,
        cpu_usage,
        active_subagent_count: 0, // Set by find_session_for_process
        terminal_app,
        is_worktree,
        repo_name,
        pr_info,
        commits_ahead,
        commits_behind,
        context_window_percent,
    })
}
