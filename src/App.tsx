import { Header } from './components/Header';
import { SessionGrid } from './components/SessionGrid';
import { Footer } from './components/Footer';
import { useSessions } from './hooks/useSessions';

function App() {
  const {
    sessions,
    totalCount,
    waitingCount,
    isLoading,
    error,
    refresh,
    focusSession,
  } = useSessions();

  return (
    <div className="min-h-screen bg-[#0a0a0a] flex flex-col">
      <Header
        totalCount={totalCount}
        waitingCount={waitingCount}
        onRefresh={refresh}
        isLoading={isLoading}
      />

      <div className="flex-1 overflow-y-auto">
        {error ? (
          <div className="p-4 text-red-400 text-sm text-center">
            {error}
          </div>
        ) : (
          <SessionGrid
            sessions={sessions}
            onSessionClick={focusSession}
          />
        )}
      </div>

      <Footer totalCount={totalCount} waitingCount={waitingCount} />
    </div>
  );
}

export default App;
