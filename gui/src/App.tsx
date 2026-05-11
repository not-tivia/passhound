import { useState } from "react";
import Lock from "./views/Lock";

export default function App() {
  const [unlocked, setUnlocked] = useState(false);

  if (!unlocked) {
    return <Lock onUnlock={() => setUnlocked(true)} />;
  }

  return (
    <div style={{ padding: 24, color: "#ccc", fontFamily: "monospace" }}>
      <h2 style={{ color: "#fff" }}>Unlocked. Vault view lands in Task 5.</h2>
    </div>
  );
}
