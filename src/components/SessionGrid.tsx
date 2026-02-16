import { useMemo } from 'react';
import { Session } from '../types/session';
import { SessionCard, GroupedSessionCard } from './SessionCard';

interface SessionGridProps {
  sessions: Session[];
  onSessionClick: (session: Session) => void;
}

export function SessionGrid({ sessions, onSessionClick }: SessionGridProps) {
  // Group sessions by projectPath, preserving original order (first session in each group)
  const groups = useMemo(() => {
    const groupMap = new Map<string, Session[]>();
    for (const s of sessions) {
      const existing = groupMap.get(s.projectPath);
      if (existing) {
        existing.push(s);
      } else {
        groupMap.set(s.projectPath, [s]);
      }
    }
    return Array.from(groupMap.values());
  }, [sessions]);

  return (
    <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
      {groups.map((group) =>
        group.length === 1 ? (
          <SessionCard
            key={`${group[0].id}-${group[0].pid}`}
            session={group[0]}
            onClick={() => onSessionClick(group[0])}
          />
        ) : (
          <GroupedSessionCard
            key={`group-${group[0].projectPath}`}
            sessions={group}
            onSessionClick={onSessionClick}
          />
        )
      )}
    </div>
  );
}
