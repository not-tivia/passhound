import { useState } from "react";
import Lock from "./views/Lock";
import Vault from "./views/Vault";
import { api } from "./api";

export default function App() {
  const [unlocked, setUnlocked] = useState(false);

  const handleLock = async () => {
    try {
      await api.vaultLock();
    } catch {
      // Best-effort; even if the IPC call fails, switch back to the lock view.
    }
    setUnlocked(false);
  };

  if (!unlocked) {
    return <Lock onUnlock={() => setUnlocked(true)} />;
  }

  return <Vault onLock={handleLock} />;
}
