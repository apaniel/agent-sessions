import { Session } from '../types/session';

interface SessionCardProps {
  session: Session;
  onClick: () => void;
}

const statusConfig = {
  waiting: {
    color: 'bg-yellow-500',
    label: 'Waiting',
  },
  processing: {
    color: 'bg-green-500',
    label: 'Processing',
  },
  idle: {
    color: 'bg-gray-500',
    label: 'Idle',
  },
};

function formatTimeAgo(timestamp: string): string {
  const date = new Date(timestamp);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMins = Math.floor(diffMs / 60000);

  if (diffMins < 1) return 'just now';
  if (diffMins < 60) return `${diffMins}m ago`;

  const diffHours = Math.floor(diffMins / 60);
  if (diffHours < 24) return `${diffHours}h ago`;

  const diffDays = Math.floor(diffHours / 24);
  return `${diffDays}d ago`;
}

function truncatePath(path: string, maxLength: number = 30): string {
  if (path.length <= maxLength) return path;

  // Replace home dir with ~
  const homePath = path.replace(/^\/Users\/[^/]+/, '~');
  if (homePath.length <= maxLength) return homePath;

  return '...' + homePath.slice(-(maxLength - 3));
}

export function SessionCard({ session, onClick }: SessionCardProps) {
  const config = statusConfig[session.status];

  return (
    <button
      onClick={onClick}
      className="w-full text-left p-3 bg-[#1a1a1a] hover:bg-[#252525] rounded-lg border border-[#2a2a2a] transition-colors cursor-pointer"
    >
      {/* Header: Status + Name + Branch */}
      <div className="flex items-center gap-2 mb-1">
        <span className={`w-2 h-2 rounded-full ${config.color}`} />
        <span className="font-medium text-sm text-white truncate flex-1">
          {session.projectName}
        </span>
        {session.gitBranch && (
          <span className="text-xs text-gray-500 truncate max-w-[80px]">
            {session.gitBranch}
          </span>
        )}
      </div>

      {/* Path */}
      <div className="text-xs text-gray-500 mb-2 truncate">
        {truncatePath(session.projectPath)}
      </div>

      {/* Message Preview */}
      {session.lastMessage && (
        <div className="text-xs text-gray-400 mb-2 line-clamp-2 italic">
          "{session.lastMessage}"
        </div>
      )}

      {/* Status + Time */}
      <div className="flex items-center justify-between text-xs">
        <span className="text-gray-500">
          {config.label}
        </span>
        <span className="text-gray-600">
          {formatTimeAgo(session.lastActivityAt)}
        </span>
      </div>
    </button>
  );
}
