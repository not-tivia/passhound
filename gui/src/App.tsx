import { useEffect, useState } from "react";

export default function App() {
  const [status, setStatus] = useState("loading...");

  useEffect(() => {
    // Smoke check: prove the IPC bridge works by hitting vault_exists.
    import("./api").then(({ api }) => {
      api
        .vaultExists()
        .then((exists) => setStatus(exists ? "vault present" : "no vault yet"))
        .catch((e) => setStatus(`error: ${JSON.stringify(e)}`));
    });
  }, []);

  return (
    <div style={{ padding: 24, color: "#ccc", background: "#0d0d10", fontFamily: "monospace", minHeight: "100vh" }}>
      <h1 style={{ color: "#fff", fontSize: 20 }}>PassHound (Phase 4.1 stub)</h1>
      <p>IPC status: {status}</p>
      <p style={{ color: "#888", fontSize: 12 }}>Lock view lands in Task 4.</p>
    </div>
  );
}
