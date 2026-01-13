use super::{AgentDetector, AgentProcess};
use crate::session::{AgentType, Session};

pub struct OpenCodeDetector;

impl AgentDetector for OpenCodeDetector {
    fn name(&self) -> &'static str {
        "OpenCode"
    }

    fn agent_type(&self) -> AgentType {
        AgentType::OpenCode
    }

    fn find_processes(&self) -> Vec<AgentProcess> {
        // TODO: Implement OpenCode process detection
        Vec::new()
    }

    fn find_sessions(&self, _processes: &[AgentProcess]) -> Vec<Session> {
        // TODO: Implement OpenCode session parsing
        Vec::new()
    }
}
