import { createContext, useContext, useState, ReactNode } from "react";

interface ToastContext {
  show: (msg: string) => void;
}

const Ctx = createContext<ToastContext | null>(null);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [msg, setMsg] = useState<string | null>(null);

  const show = (m: string) => {
    setMsg(m);
    setTimeout(() => setMsg(null), 1500);
  };

  return (
    <Ctx.Provider value={{ show }}>
      {children}
      {msg && <div className="toast">{msg}</div>}
    </Ctx.Provider>
  );
}

export function useToast() {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useToast outside ToastProvider");
  return ctx;
}
