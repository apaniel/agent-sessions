import { useState, useEffect } from 'react';
import { Session, TerminalApp, ProjectLink } from '../types/session';
import { Card, CardContent } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from '@/components/ui/dialog';
import { Input } from '@/components/ui/input';
import { formatTimeAgo, truncatePath, statusConfig } from '@/lib/formatters';
import { openUrl } from '@tauri-apps/plugin-opener';
import { invoke } from '@tauri-apps/api/core';

// Terminal app icon - shows which terminal the session is running in
const terminalAppConfig: Record<TerminalApp, { label: string; icon: React.ReactNode } | null> = {
  cursor: {
    label: 'Cursor',
    icon: (
      <svg viewBox="0 0 24 24" className="w-4 h-4" fill="currentColor">
        <rect x="3" y="3" width="18" height="18" rx="4" fill="none" stroke="currentColor" strokeWidth="2"/>
        <path d="M8 8 L16 12 L8 16Z" />
      </svg>
    ),
  },
  vscode: {
    label: 'VS Code',
    icon: (
      <svg viewBox="0 0 24 24" className="w-4 h-4" fill="currentColor">
        <path d="M17.583 2.243L12.34 7.576 7.5 3.896 3 5.849v12.353l4.5 1.953 4.84-3.68 5.243 5.333L21 20.23V3.821l-3.417-1.578zM7.5 15.572V8.479l4.84 3.546-4.84 3.547zM17.583 17.25L13.5 12.05l4.083-5.2v10.4z"/>
      </svg>
    ),
  },
  iterm2: {
    label: 'iTerm2',
    icon: (
      <svg viewBox="0 0 24 24" className="w-4 h-4" fill="none" stroke="currentColor" strokeWidth="2">
        <rect x="3" y="3" width="18" height="18" rx="3"/>
        <path d="M7 8l4 4-4 4" strokeLinecap="round" strokeLinejoin="round"/>
        <path d="M13 16h4" strokeLinecap="round"/>
      </svg>
    ),
  },
  warp: {
    label: 'Warp',
    icon: (
      <svg viewBox="0 0 24 24" className="w-4 h-4" fill="currentColor">
        <path d="M12 2L4 6v12l8 4 8-4V6l-8-4zm0 2.5L17.5 7 12 9.5 6.5 7 12 4.5zM5.5 8.27l5.5 2.73v8.5l-5.5-2.75V8.27zm7 11.23V8.99L18 6.5v8.75l-5.5 4.25z"/>
      </svg>
    ),
  },
  terminal: {
    label: 'Terminal',
    icon: (
      <svg viewBox="0 0 24 24" className="w-4 h-4" fill="none" stroke="currentColor" strokeWidth="2">
        <rect x="3" y="3" width="18" height="18" rx="3"/>
        <path d="M7 8l4 4-4 4" strokeLinecap="round" strokeLinejoin="round"/>
      </svg>
    ),
  },
  tmux: {
    label: 'tmux',
    icon: (
      <svg viewBox="0 0 24 24" className="w-4 h-4" fill="none" stroke="currentColor" strokeWidth="2">
        <rect x="3" y="3" width="18" height="18" rx="3"/>
        <line x1="12" y1="3" x2="12" y2="21"/>
        <line x1="3" y1="12" x2="12" y2="12"/>
      </svg>
    ),
  },
  unknown: null,
};


interface SessionCardProps {
  session: Session;
  onClick: () => void;
}

// Helper to get/set custom data from localStorage
const CUSTOM_NAMES_KEY = 'agent-sessions-custom-names';
const CUSTOM_URLS_KEY = 'agent-sessions-custom-urls';

function getCustomNames(): Record<string, string> {
  try {
    const stored = localStorage.getItem(CUSTOM_NAMES_KEY);
    return stored ? JSON.parse(stored) : {};
  } catch {
    return {};
  }
}

function setCustomName(projectPath: string, name: string) {
  const names = getCustomNames();
  if (name.trim()) {
    names[projectPath] = name.trim();
  } else {
    delete names[projectPath];
  }
  localStorage.setItem(CUSTOM_NAMES_KEY, JSON.stringify(names));
}

function getCustomUrls(): Record<string, string> {
  try {
    const stored = localStorage.getItem(CUSTOM_URLS_KEY);
    return stored ? JSON.parse(stored) : {};
  } catch {
    return {};
  }
}

function setCustomUrl(projectPath: string, url: string) {
  const urls = getCustomUrls();
  if (url.trim()) {
    urls[projectPath] = url.trim();
  } else {
    delete urls[projectPath];
  }
  localStorage.setItem(CUSTOM_URLS_KEY, JSON.stringify(urls));
}

// --- Project link icon (auto-detected from URL domain) ---

// Favicon fetched from Google's service — works for virtually all domains.
// Falls back to a generic link icon on error.
function Favicon({ url, className }: { url: string; className?: string }) {
  const [failed, setFailed] = useState(false);
  const hostname = (() => { try { return new URL(url).hostname; } catch { return null; } })();

  if (!hostname || failed) {
    return (
      <svg className={className || "w-3.5 h-3.5"} fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
      </svg>
    );
  }

  return (
    <img
      src={`https://icons.duckduckgo.com/ip3/${hostname}.ico`}
      alt=""
      className={`${className || "w-3.5 h-3.5"} rounded-sm bg-white/90`}
      onError={() => setFailed(true)}
    />
  );
}

// --- Shared project header (used by both single and grouped cards) ---

function ProjectHeader({ session, children }: { session: Session; children?: React.ReactNode }) {
  const hasSessionLinks = session.sessionLinks && session.sessionLinks.length > 0;

  return (
    <>
      {/* Git branch + ahead/behind + PR + session links */}
      {(session.gitBranch || hasSessionLinks) && (
        <div className="flex items-center gap-1.5 flex-wrap">
          {session.gitBranch && (
            <>
              <svg className="w-3.5 h-3.5 text-muted-foreground shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 3v12M18 9a3 3 0 100-6 3 3 0 000 6zM6 21a3 3 0 100-6 3 3 0 000 6zM18 9a9 9 0 01-9 9" />
              </svg>
              <span className="text-xs text-muted-foreground truncate">
                {session.gitBranch}
              </span>
          {(session.commitsAhead != null && session.commitsAhead > 0 || session.commitsBehind != null && session.commitsBehind! > 0) && (
            <span className="text-[10px] font-mono shrink-0">
              {session.commitsAhead != null && session.commitsAhead > 0 && (
                <span className="text-emerald-400" title="Commits ahead of upstream">
                  {'\u2191'}{session.commitsAhead}
                </span>
              )}
              {session.commitsBehind != null && session.commitsBehind > 0 && (
                <span className="text-red-400 ml-0.5" title="Commits behind upstream">
                  {'\u2193'}{session.commitsBehind}
                </span>
              )}
            </span>
          )}
          {session.prInfo && (() => {
            const prState = session.prInfo.state;
            const stateColor = prState === 'MERGED' ? { border: 'rgba(168,85,247,0.4)', text: 'rgb(192,132,252)', dot: '#a855f7' } :
                               prState === 'CLOSED' ? { border: 'rgba(248,113,113,0.4)', text: 'rgb(252,165,165)', dot: '#f87171' } :
                               { border: 'rgba(52,211,153,0.4)', text: 'rgb(110,231,183)', dot: '#34d399' };
            const ciDot = session.prInfo.ciStatus === 'success' ? '#34d399' :
                          session.prInfo.ciStatus === 'failure' ? '#f87171' :
                          session.prInfo.ciStatus === 'pending' ? '#fbbf24' :
                          null;
            return (
              <button
                className="inline-flex items-center gap-1 text-[10px] px-1.5 py-0 rounded border shrink-0 hover:bg-primary/10 transition-colors"
                style={{ borderColor: stateColor.border, color: stateColor.text }}
                onClick={(e) => {
                  e.stopPropagation();
                  invoke('launch_chrome', { projectName: session.projectName, projectPath: session.projectPath, url: session.prInfo!.url })
                    .catch(() => openUrl(session.prInfo!.url));
                }}
                title={`PR #${session.prInfo.number} - ${prState}${session.prInfo.ciStatus ? ` (CI: ${session.prInfo.ciStatus})` : ''}`}
              >
                <span className="w-1.5 h-1.5 rounded-full inline-block" style={{ backgroundColor: stateColor.dot }} />
                #{session.prInfo.number}
                {ciDot && (
                  <span className="w-1.5 h-1.5 rounded-full inline-block" style={{ backgroundColor: ciDot }} title={`CI: ${session.prInfo.ciStatus}`} />
                )}
              </button>
            );
          })()}
            </>
          )}
          {session.sessionLinks && session.sessionLinks.map((link, i) => (
            <button
              key={`sl-${i}`}
              className="inline-flex items-center justify-center w-5 h-5 rounded border shrink-0 hover:bg-primary/10 transition-colors border-amber-500/40"
              onClick={(e) => {
                e.stopPropagation();
                invoke('launch_chrome', { projectName: session.projectName, projectPath: session.projectPath, url: link.url })
                  .catch(() => openUrl(link.url));
              }}
              title={link.label}
            >
              <Favicon url={link.url} className="w-3 h-3" />
            </button>
          ))}
          {children}
        </div>
      )}
    </>
  );
}

// --- Session menu (3-dot dropdown) ---

function SessionMenu({ session, onRename, onSetUrl, onProjectLinks, onSessionLinks, customUrl }: {
  session: Session;
  onRename: () => void;
  onSetUrl: () => void;
  onProjectLinks: () => void;
  onSessionLinks: () => void;
  customUrl: string;
}) {
  const handleOpenGitHub = async () => {
    if (session.githubUrl) {
      await openUrl(session.githubUrl);
    }
  };

  const handleKillSession = async () => {
    try {
      await invoke('kill_session', { pid: session.pid });
    } catch (error) {
      console.error('Failed to kill session:', error);
    }
  };

  const handleKillSessionAndCompanions = async () => {
    try {
      await invoke('kill_session_and_companions', { pid: session.pid, projectPath: session.projectPath });
    } catch (error) {
      console.error('Failed to kill session and companions:', error);
    }
  };

  const handleDetachChrome = async () => {
    try {
      await invoke('detach_chrome', { projectPath: session.projectPath });
    } catch (error) {
      console.error('Failed to detach Chrome:', error);
    }
  };

  const handleDetachCursor = async () => {
    try {
      await invoke('detach_cursor', { projectPath: session.projectPath });
    } catch (error) {
      console.error('Failed to detach Cursor:', error);
    }
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild onClick={(e) => e.stopPropagation()}>
        <Button
          variant="ghost"
          size="sm"
          className="h-6 w-6 p-0 opacity-0 group-hover:opacity-100 transition-opacity"
        >
          <svg
            className="w-4 h-4 text-muted-foreground"
            fill="currentColor"
            viewBox="0 0 20 20"
          >
            <path d="M10 6a2 2 0 110-4 2 2 0 010 4zM10 12a2 2 0 110-4 2 2 0 010 4zM10 18a2 2 0 110-4 2 2 0 010 4z" />
          </svg>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" onClick={(e) => e.stopPropagation()}>
        <DropdownMenuItem onClick={onRename}>
          <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z" />
          </svg>
          Rename
        </DropdownMenuItem>
        <DropdownMenuItem onClick={onSetUrl}>
          <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1" />
          </svg>
          {customUrl ? 'Edit URL' : 'Set URL'}
        </DropdownMenuItem>
        <DropdownMenuItem onClick={onProjectLinks}>
          <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 5a2 2 0 012-2h10a2 2 0 012 2v16l-7-3.5L5 21V5z" />
          </svg>
          Project Links
        </DropdownMenuItem>
        <DropdownMenuItem onClick={onSessionLinks}>
          <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 7h.01M7 3h5c.512 0 1.024.195 1.414.586l7 7a2 2 0 010 2.828l-7 7a2 2 0 01-2.828 0l-7-7A1.994 1.994 0 013 12V7a4 4 0 014-4z" />
          </svg>
          Session Links
        </DropdownMenuItem>
        <DropdownMenuItem onClick={handleDetachChrome}>
          <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <circle cx="12" cy="12" r="10" strokeWidth={2} />
            <circle cx="12" cy="12" r="4" strokeWidth={2} />
            <path strokeWidth={2} d="M21.17 8H12M14 12l5.98 4.35M9.88 16.25L7 12l-4.3.01" />
          </svg>
          Detach Chrome
        </DropdownMenuItem>
        <DropdownMenuItem onClick={handleDetachCursor}>
          <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <rect x="3" y="3" width="18" height="18" rx="4" fill="none" strokeWidth={2} />
            <path d="M8 8 L16 12 L8 16Z" fill="currentColor" stroke="none" />
          </svg>
          Detach Cursor
        </DropdownMenuItem>
        {session.githubUrl && (
          <DropdownMenuItem onClick={handleOpenGitHub}>
            <svg className="w-4 h-4 mr-2" fill="currentColor" viewBox="0 0 24 24">
              <path d="M12 0c-6.626 0-12 5.373-12 12 0 5.302 3.438 9.8 8.207 11.387.599.111.793-.261.793-.577v-2.234c-3.338.726-4.033-1.416-4.033-1.416-.546-1.387-1.333-1.756-1.333-1.756-1.089-.745.083-.729.083-.729 1.205.084 1.839 1.237 1.839 1.237 1.07 1.834 2.807 1.304 3.492.997.107-.775.418-1.305.762-1.604-2.665-.305-5.467-1.334-5.467-5.931 0-1.311.469-2.381 1.236-3.221-.124-.303-.535-1.524.117-3.176 0 0 1.008-.322 3.301 1.23.957-.266 1.983-.399 3.003-.404 1.02.005 2.047.138 3.006.404 2.291-1.552 3.297-1.23 3.297-1.23.653 1.653.242 2.874.118 3.176.77.84 1.235 1.911 1.235 3.221 0 4.609-2.807 5.624-5.479 5.921.43.372.823 1.102.823 2.222v3.293c0 .319.192.694.801.576 4.765-1.589 8.199-6.086 8.199-11.386 0-6.627-5.373-12-12-12z" />
            </svg>
            Open GitHub
          </DropdownMenuItem>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem onClick={handleKillSession}>
          <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
          Kill Session
        </DropdownMenuItem>
        <DropdownMenuItem onClick={handleKillSessionAndCompanions} className="text-destructive focus:text-destructive">
          <svg className="w-4 h-4 mr-2" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
          </svg>
          Kill Session + Companions
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

// --- Dialogs (rename + URL) ---

function SessionDialogs({ session, isRenameOpen, setIsRenameOpen, isUrlOpen, setIsUrlOpen, customName, setCustomNameState, customUrl, setCustomUrlState }: {
  session: Session;
  isRenameOpen: boolean;
  setIsRenameOpen: (v: boolean) => void;
  isUrlOpen: boolean;
  setIsUrlOpen: (v: boolean) => void;
  customName: string;
  setCustomNameState: (v: string) => void;
  customUrl: string;
  setCustomUrlState: (v: string) => void;
}) {
  const [renameValue, setRenameValue] = useState('');
  const [urlValue, setUrlValue] = useState('');

  useEffect(() => {
    if (isRenameOpen) setRenameValue(customName || session.projectName);
  }, [isRenameOpen, customName, session.projectName]);

  useEffect(() => {
    if (isUrlOpen) setUrlValue(customUrl);
  }, [isUrlOpen, customUrl]);

  const handleSaveRename = () => {
    const newName = renameValue.trim();
    if (newName === session.projectName) {
      setCustomName(session.projectPath, '');
      setCustomNameState('');
    } else {
      setCustomName(session.projectPath, newName);
      setCustomNameState(newName);
    }
    setIsRenameOpen(false);
  };

  const handleResetName = () => {
    setCustomName(session.projectPath, '');
    setCustomNameState('');
    setIsRenameOpen(false);
  };

  const handleSaveUrl = () => {
    const newUrl = urlValue.trim();
    setCustomUrl(session.projectPath, newUrl);
    setCustomUrlState(newUrl);
    setIsUrlOpen(false);
  };

  const handleClearUrl = () => {
    setCustomUrl(session.projectPath, '');
    setCustomUrlState('');
    setIsUrlOpen(false);
  };

  return (
    <>
      <Dialog open={isRenameOpen} onOpenChange={setIsRenameOpen}>
        <DialogContent onClick={(e) => e.stopPropagation()}>
          <DialogHeader>
            <DialogTitle>Rename Session</DialogTitle>
          </DialogHeader>
          <div className="py-4">
            <Input
              value={renameValue}
              onChange={(e) => setRenameValue(e.target.value)}
              placeholder="Enter custom name"
              onKeyDown={(e) => { if (e.key === 'Enter') handleSaveRename(); }}
              autoFocus
            />
            <p className="text-xs text-muted-foreground mt-2">
              Original: {session.projectName}
            </p>
          </div>
          <DialogFooter className="flex gap-2">
            {customName && (
              <Button variant="outline" onClick={handleResetName}>Reset to Original</Button>
            )}
            <Button variant="outline" onClick={() => setIsRenameOpen(false)}>Cancel</Button>
            <Button onClick={handleSaveRename}>Save</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={isUrlOpen} onOpenChange={setIsUrlOpen}>
        <DialogContent onClick={(e) => e.stopPropagation()}>
          <DialogHeader>
            <DialogTitle>Set Development URL</DialogTitle>
          </DialogHeader>
          <div className="py-4">
            <Input
              value={urlValue}
              onChange={(e) => setUrlValue(e.target.value)}
              placeholder="e.g., localhost:3000"
              onKeyDown={(e) => { if (e.key === 'Enter') handleSaveUrl(); }}
              autoFocus
            />
            <p className="text-xs text-muted-foreground mt-2">
              Quick access URL for this project (e.g., dev server)
            </p>
          </div>
          <DialogFooter className="flex gap-2">
            {customUrl && (
              <Button variant="outline" onClick={handleClearUrl}>Clear URL</Button>
            )}
            <Button variant="outline" onClick={() => setIsUrlOpen(false)}>Cancel</Button>
            <Button onClick={handleSaveUrl}>Save</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

// --- Links dialog (reusable for project links and session links) ---

function LinksDialog({ isOpen, onClose, title, links, onSave }: {
  isOpen: boolean;
  onClose: () => void;
  title: string;
  links: ProjectLink[];
  onSave: (links: ProjectLink[]) => void;
}) {
  const [rows, setRows] = useState<{ label: string; url: string }[]>([]);

  // Only initialize rows when the dialog opens — not when `links` changes
  // mid-edit (polling refreshes cause new array references every cycle).
  useEffect(() => {
    if (isOpen) {
      setRows(links.length > 0 ? links.map(l => ({ label: l.label, url: l.url })) : [{ label: '', url: '' }]);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isOpen]);

  const handleSave = () => {
    const filtered = rows.filter(r => r.label.trim() && r.url.trim());
    onSave(filtered);
    onClose();
  };

  return (
    <Dialog open={isOpen} onOpenChange={(open) => { if (!open) onClose(); }}>
      <DialogContent onClick={(e) => e.stopPropagation()}>
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <div className="py-4 space-y-2">
          {rows.map((row, i) => (
            <div key={i} className="flex items-center gap-2">
              <Input
                value={row.label}
                onChange={(e) => { const r = [...rows]; r[i] = { ...r[i], label: e.target.value }; setRows(r); }}
                placeholder="Label"
                className="flex-1"
              />
              <Input
                value={row.url}
                onChange={(e) => { const r = [...rows]; r[i] = { ...r[i], url: e.target.value }; setRows(r); }}
                placeholder="URL"
                className="flex-[2]"
                onKeyDown={(e) => { if (e.key === 'Enter') handleSave(); }}
              />
              <Button
                variant="ghost"
                size="sm"
                className="h-8 w-8 p-0 shrink-0 text-muted-foreground hover:text-destructive"
                onClick={() => setRows(rows.filter((_, j) => j !== i))}
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </Button>
            </div>
          ))}
          <Button
            variant="outline"
            size="sm"
            onClick={() => setRows([...rows, { label: '', url: '' }])}
            className="w-full"
          >
            + Add Link
          </Button>
        </div>
        <DialogFooter className="flex gap-2">
          <Button variant="outline" onClick={onClose}>Cancel</Button>
          <Button onClick={handleSave}>Save</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// --- Per-session state hook (custom name, url, dialog state) ---

function useSessionCustomData(session: Session) {
  const { projectPath, projectName } = session;
  const [customName, setCustomNameState] = useState<string>('');
  const [customUrl, setCustomUrlState] = useState<string>('');
  const [isRenameOpen, setIsRenameOpen] = useState(false);
  const [isUrlOpen, setIsUrlOpen] = useState(false);
  const [isProjectLinksOpen, setIsProjectLinksOpen] = useState(false);
  const [isSessionLinksOpen, setIsSessionLinksOpen] = useState(false);

  useEffect(() => {
    const names = getCustomNames();
    const urls = getCustomUrls();
    setCustomNameState(names[projectPath] || '');
    setCustomUrlState(urls[projectPath] || '');
  }, [projectPath]);

  const getFullUrl = () => {
    let url = customUrl;
    if (url && !url.startsWith('http://') && !url.startsWith('https://')) {
      url = 'http://' + url;
    }
    return url;
  };

  const handleOpenUrl = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (customUrl) {
      try {
        await invoke('launch_chrome', { projectName, projectPath, url: getFullUrl() });
      } catch {
        // Fallback to default browser
        await openUrl(getFullUrl());
      }
    }
  };

  const handleLaunchChrome = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      const url = customUrl ? getFullUrl() : null;
      await invoke('launch_chrome', { projectName, projectPath, url });
    } catch (error) {
      console.error('Failed to launch Chrome:', error);
    }
  };

  const handleLaunchCursor = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await invoke('launch_cursor', { projectPath });
    } catch (error) {
      console.error('Failed to launch Cursor companion:', error);
    }
  };

  const handleSaveProjectLinks = async (links: ProjectLink[]) => {
    try {
      await invoke('save_project_links', { projectPath, links });
    } catch (error) {
      console.error('Failed to save project links:', error);
    }
  };

  const handleSaveSessionLinks = async (links: ProjectLink[]) => {
    try {
      await invoke('save_session_links', { projectPath, sessionId: session.id, links });
    } catch (error) {
      console.error('Failed to save session links:', error);
    }
  };

  return { customName, setCustomNameState, customUrl, setCustomUrlState, isRenameOpen, setIsRenameOpen, isUrlOpen, setIsUrlOpen, isProjectLinksOpen, setIsProjectLinksOpen, isSessionLinksOpen, setIsSessionLinksOpen, handleOpenUrl, handleLaunchChrome, handleLaunchCursor, handleSaveProjectLinks, handleSaveSessionLinks };
}

// --- Single session card (unchanged layout for solo sessions) ---

export function SessionCard({ session, onClick }: SessionCardProps) {
  const config = statusConfig[session.status];
  const { customName, setCustomNameState, customUrl, setCustomUrlState, isRenameOpen, setIsRenameOpen, isUrlOpen, setIsUrlOpen, isProjectLinksOpen, setIsProjectLinksOpen, isSessionLinksOpen, setIsSessionLinksOpen, handleOpenUrl, handleLaunchChrome, handleLaunchCursor, handleSaveProjectLinks, handleSaveSessionLinks } = useSessionCustomData(session);

  const displayName = customName || session.projectName;

  return (
    <>
      <Card
        className={`relative group cursor-pointer transition-all duration-200 hover:shadow-lg py-0 gap-0 h-full flex flex-col ${config.cardBg} ${config.cardBorder} hover:border-primary/30`}
        onClick={onClick}
      >
        <CardContent className="p-4 flex flex-col flex-1">
          {/* Header: Project name + Menu + Status indicator */}
          <div className="flex items-start justify-between gap-2 mb-3">
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-1.5 min-w-0">
                <h3 className="font-semibold text-base text-foreground truncate group-hover:text-primary transition-colors">
                  {displayName}
                </h3>
                {session.projectLinks && session.projectLinks.map((link, i) => (
                  <button
                    key={`pl-${i}`}
                    className="inline-flex items-center justify-center w-5 h-5 rounded border shrink-0 hover:bg-primary/10 transition-colors border-sky-500/40"
                    onClick={(e) => {
                      e.stopPropagation();
                      invoke('launch_chrome', { projectName: session.projectName, projectPath: session.projectPath, url: link.url })
                        .catch(() => openUrl(link.url));
                    }}
                    title={link.label}
                  >
                    <Favicon url={link.url} className="w-3 h-3" />
                  </button>
                ))}
              </div>
              <p className="text-xs text-muted-foreground truncate mt-0.5 flex items-center gap-1.5">
                <span className="truncate">
                  {session.repoName || truncatePath(session.projectPath)}
                </span>
                {session.isWorktree && (
                  <span className="shrink-0 text-[10px] px-1 py-0 rounded bg-blue-500/20 text-blue-300 border border-blue-500/30">
                    worktree
                  </span>
                )}
              </p>
            </div>
            <div className="flex items-center gap-1.5 shrink-0">
              {customUrl && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 w-6 p-0 opacity-0 group-hover:opacity-100 transition-opacity hover:bg-primary/10"
                  onClick={handleOpenUrl}
                  title={`Open ${customUrl}`}
                >
                  <svg className="w-4 h-4 text-muted-foreground" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                  </svg>
                </Button>
              )}
              <Button
                variant="ghost"
                size="sm"
                className="h-6 w-6 p-0 opacity-0 group-hover:opacity-100 transition-opacity hover:bg-primary/10"
                onClick={handleLaunchChrome}
                title="Open isolated Chrome"
              >
                <svg className="w-4 h-4 text-muted-foreground" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <circle cx="12" cy="12" r="10" strokeWidth={2} />
                  <circle cx="12" cy="12" r="4" strokeWidth={2} />
                  <path strokeWidth={2} d="M21.17 8H12M14 12l5.98 4.35M9.88 16.25L7 12l-4.3.01" />
                </svg>
              </Button>
              {session.terminalApp !== 'cursor' && (
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 w-6 p-0 opacity-0 group-hover:opacity-100 transition-opacity hover:bg-primary/10"
                  onClick={handleLaunchCursor}
                  title="Open Cursor companion"
                >
                  <svg viewBox="0 0 24 24" className="w-4 h-4 text-muted-foreground" fill="none" stroke="currentColor">
                    <rect x="3" y="3" width="18" height="18" rx="4" strokeWidth="2"/>
                    <path d="M8 8 L16 12 L8 16Z" fill="currentColor" stroke="none" />
                  </svg>
                </Button>
              )}
              <SessionMenu session={session} onRename={() => setIsRenameOpen(true)} onSetUrl={() => setIsUrlOpen(true)} onProjectLinks={() => setIsProjectLinksOpen(true)} onSessionLinks={() => setIsSessionLinksOpen(true)} customUrl={customUrl} />
            </div>
          </div>

          <ProjectHeader session={session} />

          {/* spacer between branch/links row and message */}
          {(session.gitBranch || (session.sessionLinks && session.sessionLinks.length > 0)) && <div className="mb-3" />}

          {/* Message Preview */}
          <div className="flex-1">
            {session.lastMessage && (
              <div className="text-sm text-muted-foreground line-clamp-2 leading-relaxed">
                {session.lastMessage}
              </div>
            )}
          </div>

          {/* Footer: Status Badge + Terminal Icon + Time */}
          <div className="flex items-center justify-between pt-3 mt-3 border-t border-border">
            <div className="flex items-center gap-2">
              <Badge variant="outline" className={config.badgeClassName}>
                {config.label}
              </Badge>
              {session.activeSubagentCount > 0 && (
                <span className="text-xs text-muted-foreground">
                  [+{session.activeSubagentCount}]
                </span>
              )}
              {session.contextWindowPercent != null && (
                <span
                  className={`text-[10px] font-mono ${
                    session.contextWindowPercent < 30
                      ? 'text-red-400'
                      : session.contextWindowPercent < 50
                        ? 'text-amber-400'
                        : 'text-emerald-400'
                  }`}
                  title="Context window remaining"
                >
                  {Math.round(session.contextWindowPercent)}% ctx
                </span>
              )}
            </div>
            <div className="flex items-center gap-2">
              {session.terminalApp && session.terminalApp !== 'unknown' && session.terminalApp !== 'cursor' && (
                <button
                  className="text-muted-foreground hover:text-foreground transition-colors"
                  onClick={async (e) => { e.stopPropagation(); try { await invoke('open_in_cursor', { projectPath: session.projectPath }); } catch (err) { console.error('Failed to open Cursor:', err); } }}
                  title="Open in Cursor"
                >
                  <svg viewBox="0 0 24 24" className="w-4 h-4" fill="currentColor">
                    <rect x="3" y="3" width="18" height="18" rx="4" fill="none" stroke="currentColor" strokeWidth="2"/>
                    <path d="M8 8 L16 12 L8 16Z" />
                  </svg>
                </button>
              )}
              {session.terminalApp && session.terminalApp !== 'unknown' && (
                <span className="text-xs text-muted-foreground bg-muted px-1.5 py-0.5 rounded">
                  {terminalAppConfig[session.terminalApp]?.label ?? session.terminalApp}
                </span>
              )}
              <span className="text-xs text-muted-foreground">
                {formatTimeAgo(session.lastActivityAt)}
              </span>
            </div>
          </div>
        </CardContent>
      </Card>

      <SessionDialogs
        session={session}
        isRenameOpen={isRenameOpen} setIsRenameOpen={setIsRenameOpen}
        isUrlOpen={isUrlOpen} setIsUrlOpen={setIsUrlOpen}
        customName={customName} setCustomNameState={setCustomNameState}
        customUrl={customUrl} setCustomUrlState={setCustomUrlState}
      />
      <LinksDialog
        isOpen={isProjectLinksOpen}
        onClose={() => setIsProjectLinksOpen(false)}
        title="Project Links"
        links={session.projectLinks}
        onSave={handleSaveProjectLinks}
      />
      <LinksDialog
        isOpen={isSessionLinksOpen}
        onClose={() => setIsSessionLinksOpen(false)}
        title="Session Links"
        links={session.sessionLinks}
        onSave={handleSaveSessionLinks}
      />
    </>
  );
}

// --- Sub-card for a single session inside a grouped card ---

function SessionSubCard({ session, onClick }: { session: Session; onClick: () => void }) {
  const config = statusConfig[session.status];
  const { customName, setCustomNameState, customUrl, setCustomUrlState, isRenameOpen, setIsRenameOpen, isUrlOpen, setIsUrlOpen, isProjectLinksOpen, setIsProjectLinksOpen, isSessionLinksOpen, setIsSessionLinksOpen, handleOpenUrl, handleLaunchChrome, handleLaunchCursor, handleSaveProjectLinks, handleSaveSessionLinks } = useSessionCustomData(session);

  const displayName = customName || null; // only show if custom name set

  return (
    <>
      <div
        className={`group relative rounded-lg p-2.5 cursor-pointer transition-all duration-200 hover:shadow-md ${config.cardBg} border ${config.cardBorder} hover:border-primary/30`}
        onClick={onClick}
      >
        {/* Hover menu - absolute positioned */}
        <div className="absolute top-1.5 right-1.5 flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity z-10">
          {customUrl && (
            <Button
              variant="ghost"
              size="sm"
              className="h-5 w-5 p-0 hover:bg-primary/10"
              onClick={handleOpenUrl}
              title={`Open ${customUrl}`}
            >
              <svg className="w-3.5 h-3.5 text-muted-foreground" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
              </svg>
            </Button>
          )}
          <Button
            variant="ghost"
            size="sm"
            className="h-5 w-5 p-0 hover:bg-primary/10"
            onClick={handleLaunchChrome}
            title="Open isolated Chrome"
          >
            <svg className="w-3.5 h-3.5 text-muted-foreground" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <circle cx="12" cy="12" r="10" strokeWidth={2} />
              <circle cx="12" cy="12" r="4" strokeWidth={2} />
              <path strokeWidth={2} d="M21.17 8H12M14 12l5.98 4.35M9.88 16.25L7 12l-4.3.01" />
            </svg>
          </Button>
          {session.terminalApp !== 'cursor' && (
            <Button
              variant="ghost"
              size="sm"
              className="h-5 w-5 p-0 hover:bg-primary/10"
              onClick={handleLaunchCursor}
              title="Open Cursor companion"
            >
              <svg viewBox="0 0 24 24" className="w-3.5 h-3.5 text-muted-foreground" fill="none" stroke="currentColor">
                <rect x="3" y="3" width="18" height="18" rx="4" strokeWidth="2"/>
                <path d="M8 8 L16 12 L8 16Z" fill="currentColor" stroke="none" />
              </svg>
            </Button>
          )}
          <SessionMenu session={session} onRename={() => setIsRenameOpen(true)} onSetUrl={() => setIsUrlOpen(true)} onProjectLinks={() => setIsProjectLinksOpen(true)} onSessionLinks={() => setIsSessionLinksOpen(true)} customUrl={customUrl} />
        </div>

        {/* Custom name (if set) */}
        {displayName && (
          <div className="text-xs font-medium text-foreground truncate mb-1">{displayName}</div>
        )}

        {/* Message preview */}
        {session.lastMessage && (
          <div className="text-sm text-muted-foreground line-clamp-2 leading-snug mb-1.5">
            {session.lastMessage}
          </div>
        )}

        {/* Row 3: Status + context + terminal + time */}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Badge variant="outline" className={`text-[10px] px-1.5 py-0 ${config.badgeClassName}`}>
              {config.label}
            </Badge>
            {session.activeSubagentCount > 0 && (
              <span className="text-[10px] text-muted-foreground">
                [+{session.activeSubagentCount}]
              </span>
            )}
            {session.contextWindowPercent != null && (
              <span
                className={`text-[10px] font-mono ${
                  session.contextWindowPercent < 30
                    ? 'text-red-400'
                    : session.contextWindowPercent < 50
                      ? 'text-amber-400'
                      : 'text-emerald-400'
                }`}
                title="Context window remaining"
              >
                {Math.round(session.contextWindowPercent)}% ctx
              </span>
            )}
          </div>
          <div className="flex items-center gap-2">
            {session.terminalApp && session.terminalApp !== 'unknown' && session.terminalApp !== 'cursor' && (
              <button
                className="text-muted-foreground hover:text-foreground transition-colors"
                onClick={async (e) => { e.stopPropagation(); try { await invoke('open_in_cursor', { projectPath: session.projectPath }); } catch (err) { console.error('Failed to open Cursor:', err); } }}
                title="Open in Cursor"
              >
                <svg viewBox="0 0 24 24" className="w-3.5 h-3.5" fill="currentColor">
                  <rect x="3" y="3" width="18" height="18" rx="4" fill="none" stroke="currentColor" strokeWidth="2"/>
                  <path d="M8 8 L16 12 L8 16Z" />
                </svg>
              </button>
            )}
            {session.terminalApp && session.terminalApp !== 'unknown' && (
              <span className="text-[10px] text-muted-foreground bg-muted px-1 py-0.5 rounded">
                {terminalAppConfig[session.terminalApp]?.label ?? session.terminalApp}
              </span>
            )}
            <span className="text-[10px] text-muted-foreground">
              {formatTimeAgo(session.lastActivityAt)}
            </span>
          </div>
        </div>
      </div>

      <SessionDialogs
        session={session}
        isRenameOpen={isRenameOpen} setIsRenameOpen={setIsRenameOpen}
        isUrlOpen={isUrlOpen} setIsUrlOpen={setIsUrlOpen}
        customName={customName} setCustomNameState={setCustomNameState}
        customUrl={customUrl} setCustomUrlState={setCustomUrlState}
      />
      <LinksDialog
        isOpen={isProjectLinksOpen}
        onClose={() => setIsProjectLinksOpen(false)}
        title="Project Links"
        links={session.projectLinks}
        onSave={handleSaveProjectLinks}
      />
      <LinksDialog
        isOpen={isSessionLinksOpen}
        onClose={() => setIsSessionLinksOpen(false)}
        title="Session Links"
        links={session.sessionLinks}
        onSave={handleSaveSessionLinks}
      />
    </>
  );
}

// --- Grouped card for multiple sessions sharing the same folder ---

interface GroupedSessionCardProps {
  sessions: Session[];
  onSessionClick: (session: Session) => void;
}

export function GroupedSessionCard({ sessions, onSessionClick }: GroupedSessionCardProps) {
  // Use first session for shared project info
  const representative = sessions[0];

  return (
    <Card className="relative py-0 gap-0 h-full flex flex-col bg-amber-500/5 border-amber-500/25">
      <CardContent className="p-4 flex flex-col flex-1">
        {/* Shared header */}
        <div className="flex items-start justify-between gap-2 mb-2">
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-1.5 min-w-0">
              <h3 className="font-semibold text-base text-foreground truncate">
                {representative.projectName}
              </h3>
              {representative.projectLinks && representative.projectLinks.map((link, i) => (
                <button
                  key={`pl-${i}`}
                  className="inline-flex items-center justify-center w-5 h-5 rounded border shrink-0 hover:bg-primary/10 transition-colors border-sky-500/40"
                  onClick={(e) => {
                    e.stopPropagation();
                    invoke('launch_chrome', { projectName: representative.projectName, projectPath: representative.projectPath, url: link.url })
                      .catch(() => openUrl(link.url));
                  }}
                  title={link.label}
                >
                  <Favicon url={link.url} className="w-3 h-3" />
                </button>
              ))}
            </div>
            <p className="text-xs text-muted-foreground truncate mt-0.5 flex items-center gap-1.5">
              <span className="truncate">
                {representative.repoName || truncatePath(representative.projectPath)}
              </span>
              {representative.isWorktree && (
                <span className="shrink-0 text-[10px] px-1 py-0 rounded bg-blue-500/20 text-blue-300 border border-blue-500/30">
                  worktree
                </span>
              )}
              <span className="shrink-0 text-[10px] px-1 py-0 rounded bg-amber-500/20 text-amber-300 border border-amber-500/30">
                {sessions.length} agents
              </span>
            </p>
          </div>
        </div>

        <ProjectHeader session={representative} />

        {/* Sub-cards for each session */}
        <div className="flex flex-col gap-2 mt-3">
          {sessions.map((session) => (
            <SessionSubCard
              key={`${session.id}-${session.pid}`}
              session={session}
              onClick={() => onSessionClick(session)}
            />
          ))}
        </div>
      </CardContent>
    </Card>
  );
}
