use super::{AgentDetector, AgentProcess};
use crate::session::{AgentType, Session, SessionStatus};

pub struct OpenCodeDetector;

impl AgentDetector for OpenCodeDetector {
    fn name(&self) -> &'static str {
        "OpenCode"
    }

    fn agent_type(&self) -> AgentType {
        AgentType::OpenCode
    }

    fn find_processes(&self) -> Vec<AgentProcess> {
        find_opencode_processes()
    }

    fn find_sessions(&self, processes: &[AgentProcess]) -> Vec<Session> {
        if processes.is_empty() {
            return Vec::new();
        }
        get_opencode_sessions(processes)
    }
}

/// Find running opencode processes
fn find_opencode_processes() -> Vec<AgentProcess> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        ProcessRefreshKind::new()
            .with_cpu()
            .with_cwd(UpdateKind::OnlyIfNotSet),
    );

    let mut processes = Vec::new();

    for (pid, process) in system.processes() {
        let name = process.name().to_string_lossy().to_lowercase();

        if name == "opencode" {
            processes.push(AgentProcess {
                pid: pid.as_u32(),
                cpu_usage: process.cpu_usage(),
                cwd: process.cwd().map(|p| p.to_path_buf()),
            });
        }
    }

    log::debug!("Found {} opencode processes", processes.len());
    processes
}

/// Get OpenCode sessions from SQLite databases
fn get_opencode_sessions(processes: &[AgentProcess]) -> Vec<Session> {
    use std::collections::HashMap;

    let mut sessions = Vec::new();

    // OpenCode data directory: ~/.local/share/opencode/project/
    let base_path = match dirs::data_local_dir() {
        Some(p) => p.join("opencode").join("project"),
        None => return sessions,
    };

    if !base_path.exists() {
        log::debug!("OpenCode data directory does not exist: {:?}", base_path);
        return sessions;
    }

    // Build cwd -> process map
    let mut cwd_to_process: HashMap<String, &AgentProcess> = HashMap::new();
    for process in processes {
        if let Some(cwd) = &process.cwd {
            cwd_to_process.insert(cwd.to_string_lossy().to_string(), process);
        }
    }

    // Scan project directories
    if let Ok(entries) = std::fs::read_dir(&base_path) {
        for entry in entries.flatten() {
            let project_dir = entry.path();
            if !project_dir.is_dir() {
                continue;
            }

            let db_path = project_dir.join("storage").join("db.sqlite");
            if !db_path.exists() {
                continue;
            }

            let project_slug = project_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            // Find matching process by checking if cwd contains project slug
            let matching_process = cwd_to_process
                .iter()
                .find(|(cwd, _)| cwd.contains(project_slug))
                .map(|(_, p)| *p);

            if let Some(process) = matching_process {
                if let Some(session) = parse_opencode_session(&db_path, project_slug, process) {
                    sessions.push(session);
                }
            }
        }
    }

    sessions
}

/// Parse a single OpenCode session from SQLite
fn parse_opencode_session(
    db_path: &std::path::Path,
    project_slug: &str,
    process: &AgentProcess,
) -> Option<Session> {
    use rusqlite::Connection;

    let conn = match Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Failed to open OpenCode database {:?}: {}", db_path, e);
            return None;
        }
    };

    // Get most recent session
    let session_row: Result<(String, String, i64), _> = conn.query_row(
        "SELECT id, title, updated_at FROM sessions ORDER BY updated_at DESC LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    );

    let (session_id, _title, updated_at) = match session_row {
        Ok(r) => r,
        Err(_) => return None,
    };

    // Get last message for status detection
    let last_msg: Result<(String, Option<i64>), _> = conn.query_row(
        "SELECT role, finished_at FROM messages WHERE session_id = ? ORDER BY created_at DESC LIMIT 1",
        [&session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );

    let status = match last_msg {
        Ok((role, finished_at)) => {
            if process.cpu_usage > 5.0 {
                SessionStatus::Processing
            } else if role == "assistant" && finished_at.is_some() {
                SessionStatus::Waiting
            } else if role == "user" {
                SessionStatus::Processing
            } else {
                SessionStatus::Idle
            }
        }
        Err(_) => SessionStatus::Idle,
    };

    // Get last message content for preview
    let last_message: Option<String> = conn
        .query_row(
            "SELECT parts FROM messages WHERE session_id = ? ORDER BY created_at DESC LIMIT 1",
            [&session_id],
            |row| row.get(0),
        )
        .ok();

    // Convert timestamp to ISO string
    let last_activity_at = chrono::DateTime::from_timestamp(updated_at, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    // Extract project name from slug
    let project_name = project_slug
        .split('-')
        .filter(|s| !s.is_empty())
        .last()
        .unwrap_or(project_slug)
        .to_string();

    Some(Session {
        id: session_id,
        agent_type: AgentType::OpenCode,
        project_name,
        project_path: format!("~/.local/share/opencode/project/{}", project_slug),
        git_branch: None,
        github_url: None,
        status,
        last_message,
        last_message_role: None,
        last_activity_at,
        pid: process.pid,
        cpu_usage: process.cpu_usage,
        active_subagent_count: 0,
    })
}
