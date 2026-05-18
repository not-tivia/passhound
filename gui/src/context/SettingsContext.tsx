import { createContext, useCallback, useContext, useEffect, useState } from "react";
import type { ReactNode } from "react";
import { api } from "../api";
import type { GuiError, SettingsView } from "../types";

interface SettingsContextValue {
  settings: SettingsView;
  refresh: () => Promise<void>;
}

const DEFAULT_SETTINGS: SettingsView = {
  idle_lock_seconds: 0,
  clipboard_clear_seconds: 0,
  analyze_top_n: 10,
  default_reveal: false,
  reveal_clear_seconds: 0,
};

const SettingsContext = createContext<SettingsContextValue>({
  settings: DEFAULT_SETTINGS,
  refresh: async () => {},
});

interface SettingsProviderProps {
  children: ReactNode;
  onLockedError: () => void;
}

export function SettingsProvider({ children, onLockedError }: SettingsProviderProps) {
  const [settings, setSettings] = useState<SettingsView>(DEFAULT_SETTINGS);

  const refresh = useCallback(async () => {
    try {
      const s = await api.getSettings();
      setSettings(s);
    } catch (e) {
      if ((e as GuiError).kind === "Locked") onLockedError();
      // Other errors -> keep stale values silently.
    }
  }, [onLockedError]);

  useEffect(() => { refresh(); }, [refresh]);

  return (
    <SettingsContext.Provider value={{ settings, refresh }}>
      {children}
    </SettingsContext.Provider>
  );
}

export function useSettings(): SettingsContextValue {
  return useContext(SettingsContext);
}
