import { useCallback, useState } from "react";
import Lock from "./views/Lock";
import Vault from "./views/Vault";
import { SettingsProvider, useSettings } from "./context/SettingsContext";
import { useIdleLock } from "./hooks/useIdleLock";
import { api } from "./api";

export default function App() {
  const [unlocked, setUnlocked] = useState(false);

  const handleLock = useCallback(async () => {
    try {
      await api.vaultLock();
    } catch {
      // Best-effort; even if the IPC call fails, switch back to the lock view.
    }
    setUnlocked(false);
  }, []);

  if (!unlocked) {
    return <Lock onUnlock={() => setUnlocked(true)} />;
  }

  return (
    <SettingsProvider onLockedError={handleLock}>
      <UnlockedShell onLock={handleLock} />
    </SettingsProvider>
  );
}

function UnlockedShell({ onLock }: { onLock: () => void }) {
  const { settings } = useSettings();
  useIdleLock(settings.idle_lock_seconds, onLock);
  return <Vault onLock={onLock} />;
}
