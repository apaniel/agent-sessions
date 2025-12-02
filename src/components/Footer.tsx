interface FooterProps {
  totalCount: number;
  waitingCount: number;
}

export function Footer({ totalCount, waitingCount }: FooterProps) {
  return (
    <div className="p-2 border-t border-[#2a2a2a] text-xs text-gray-500 text-center">
      {totalCount} session{totalCount !== 1 ? 's' : ''}
      {waitingCount > 0 && ` · ${waitingCount} waiting`}
    </div>
  );
}
