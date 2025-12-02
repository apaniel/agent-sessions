import { Session } from '../types/session';
import { SessionCard } from './SessionCard';

interface SessionGridProps {
  sessions: Session[];
  onSessionClick: (session: Session) => void;
}

export function SessionGrid({ sessions, onSessionClick }: SessionGridProps) {
  if (sessions.length === 0) {
    return (
      <div className="flex items-center justify-center h-40 text-gray-500 text-sm">
        No active Claude sessions
      </div>
    );
  }

  return (
    <div className="grid grid-cols-2 gap-2 p-2">
      {sessions.map((session) => (
        <SessionCard
          key={session.id}
          session={session}
          onClick={() => onSessionClick(session)}
        />
      ))}
    </div>
  );
}
