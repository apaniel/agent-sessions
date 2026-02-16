export type SessionStatus = 'waiting' | 'processing' | 'thinking' | 'compacting' | 'idle';

export type AgentType = 'claude' | 'opencode';

export type TerminalApp = 'iterm2' | 'warp' | 'cursor' | 'vscode' | 'terminal' | 'tmux' | 'unknown';

export type CiStatus = 'success' | 'failure' | 'pending' | 'unknown';

export interface PrInfo {
  url: string;
  number: number;
  state: string;
  ciStatus: CiStatus | null;
}

export interface ProjectLink {
  label: string;
  url: string;
  icon?: string | null;
}

export interface Session {
  id: string;
  agentType: AgentType;
  projectName: string;
  projectPath: string;
  gitBranch: string | null;
  githubUrl: string | null;
  status: SessionStatus;
  lastMessage: string | null;
  lastMessageRole: 'user' | 'assistant' | null;
  lastActivityAt: string;
  pid: number;
  cpuUsage: number;
  activeSubagentCount: number;
  terminalApp: TerminalApp;
  isWorktree: boolean;
  repoName: string | null;
  prInfo: PrInfo | null;
  commitsAhead: number | null;
  commitsBehind: number | null;
  contextWindowPercent: number | null;
  projectLinks: ProjectLink[];
  sessionLinks: ProjectLink[];
}

export interface SessionsResponse {
  sessions: Session[];
  totalCount: number;
  waitingCount: number;
}
