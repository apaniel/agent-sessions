interface HeaderProps {
  onRefresh: () => void;
  isLoading: boolean;
}

export function Header({ onRefresh, isLoading }: HeaderProps) {
  return (
    <div className="flex items-center justify-between p-3 border-b border-[#2a2a2a]">
      <div>
        <h1 className="text-sm font-semibold text-white">Claude Sessions</h1>
      </div>
      <button
        onClick={onRefresh}
        disabled={isLoading}
        className="p-1.5 hover:bg-[#2a2a2a] rounded transition-colors disabled:opacity-50"
        title="Refresh"
      >
        <svg
          className={`w-4 h-4 text-gray-400 ${isLoading ? 'animate-spin' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15"
          />
        </svg>
      </button>
    </div>
  );
}
