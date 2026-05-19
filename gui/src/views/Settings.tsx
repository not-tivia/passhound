import { useCallback, useEffect, useState } from "react";
import { useSettings } from "../context/SettingsContext";
import { useToast } from "../components/Toast";
import SettingsSection from "../components/SettingsSection";
import SettingNumberInput from "../components/SettingNumberInput";
import SettingCheckbox from "../components/SettingCheckbox";
import ChangeMasterPasswordModal from "../components/ChangeMasterPasswordModal";
import ConfirmReunlockModal from "../components/ConfirmReunlockModal";
import ResetLearningModal from "../components/ResetLearningModal";
import EraFormModal from "../components/EraFormModal";
import { api } from "../api";
import type { EraSummary, GuiError, SettingChange } from "../types";

interface SettingsProps {
  onLockedError: () => void;
  onLock: () => void;
}

export default function Settings({ onLockedError, onLock }: SettingsProps) {
  const { settings, refresh } = useSettings();
  const toast = useToast();
  const [changeOpen, setChangeOpen] = useState(false);
  const [confirmReunlockOpen, setConfirmReunlockOpen] = useState(false);
  const [resetLearningOpen, setResetLearningOpen] = useState(false);

  const [eras, setEras] = useState<EraSummary[]>([]);
  const [eraModalOpen, setEraModalOpen] = useState(false);
  const [editingEra, setEditingEra] = useState<EraSummary | null>(null);

  const refreshEras = useCallback(async () => {
    try {
      setEras(await api.listEras());
    } catch {
      setEras([]);
    }
  }, []);

  useEffect(() => { void refreshEras(); }, [refreshEras]);

  const handleAddEra = () => {
    setEditingEra(null);
    setEraModalOpen(true);
  };
  const handleEditEra = (era: EraSummary) => {
    setEditingEra(era);
    setEraModalOpen(true);
  };
  const handleDeleteEra = async (era: EraSummary) => {
    if (!window.confirm(`Delete era "${era.name}"? Accounts and recovery filters referencing it will fall back to "All".`)) return;
    try {
      await api.deleteEra(era.id);
      void refreshEras();
    } catch (e) {
      const err = e as GuiError;
      alert(`Failed to delete: ${err.message ?? err.kind}`);
    }
  };

  const save = async (change: SettingChange) => {
    try {
      await api.setSetting(change);
      await refresh();
    } catch (e) {
      const err = e as GuiError;
      if (err.kind === "Locked") onLockedError();
      else toast.show(`Save failed: ${err.message ?? err.kind}`);
    }
  };

  return (
    <div className="settings">
      <div className="settings__title">Settings</div>

      <SettingsSection title="MASTER PASSWORD">
        <div className="settings-row">
          <label className="settings-row__label">Master password</label>
          <button
            className="settings-row__change-pw-btn"
            onClick={() => setChangeOpen(true)}
          >
            Change…
          </button>
        </div>
      </SettingsSection>

      <SettingsSection title="SECURITY">
        <SettingNumberInput
          label="Idle auto-lock"
          unit="minutes"
          hint="0 = off"
          min={0}
          max={1440}
          value={Math.floor(settings.idle_lock_seconds / 60)}
          onSave={(minutes) => save({ key: "idle_lock_seconds", value: minutes * 60 })}
        />
        <SettingNumberInput
          label="Clipboard auto-clear"
          unit="seconds"
          hint="0 = off"
          min={0}
          max={3600}
          value={settings.clipboard_clear_seconds}
          onSave={(seconds) => save({ key: "clipboard_clear_seconds", value: seconds })}
        />
        <SettingNumberInput
          label="Auto-mask revealed passwords after"
          unit="seconds"
          hint="0 = off"
          min={0}
          max={600}
          value={settings.reveal_clear_seconds}
          onSave={(seconds) => save({ key: "reveal_clear_seconds", value: seconds })}
        />
      </SettingsSection>

      <SettingsSection title="RECOVERY">
        <SettingNumberInput
          label="Auto-favorite top-N"
          unit="words"
          min={1}
          max={100}
          value={settings.analyze_top_n}
          onSave={(n) => save({ key: "analyze_top_n", value: n })}
        />
      </SettingsSection>

      <SettingsSection title="DISPLAY">
        <SettingCheckbox
          label="Default reveal"
          value={settings.default_reveal}
          onSave={(b) => save({ key: "default_reveal", value: b })}
        />
      </SettingsSection>

      <SettingsSection title="LEARNING">
        <div className="settings-row">
          <label className="settings-row__label">Reset learning</label>
          <button
            className="settings-row__change-pw-btn"
            onClick={() => setResetLearningOpen(true)}
          >
            Reset…
          </button>
        </div>
      </SettingsSection>

      <SettingsSection title="ERAS">
        <div className="settings-eras">
          {eras.length === 0 && (
            <p className="settings-eras__empty">
              No eras yet. Add one to filter accounts by time window in the Vault view.
            </p>
          )}
          {eras.length > 0 && (
            <table className="settings-eras__table">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Start</th>
                  <th>End</th>
                  <th>Notes</th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {eras.map((e) => (
                  <tr key={e.id}>
                    <td>{e.name}</td>
                    <td>{e.start_date ?? "—"}</td>
                    <td>{e.end_date ?? "—"}</td>
                    <td>{e.notes ?? "—"}</td>
                    <td className="settings-eras__actions">
                      <button onClick={() => handleEditEra(e)}>Edit</button>
                      <button onClick={() => void handleDeleteEra(e)} className="settings-eras__delete">Delete</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <button onClick={handleAddEra} className="settings-eras__add">+ Add era</button>
        </div>
      </SettingsSection>

      {eraModalOpen && (
        <EraFormModal
          initial={editingEra}
          onSaved={() => void refreshEras()}
          onClose={() => setEraModalOpen(false)}
        />
      )}

      <div className="settings__footnote">All settings save automatically as you type.</div>
      {changeOpen && (
        <ChangeMasterPasswordModal
          onClose={() => setChangeOpen(false)}
          onChanged={() => {
            setChangeOpen(false);
            setConfirmReunlockOpen(true);
          }}
          onLockedError={onLockedError}
        />
      )}
      {confirmReunlockOpen && (
        <ConfirmReunlockModal
          onReunlock={() => {
            setConfirmReunlockOpen(false);
            onLock();
          }}
        />
      )}
      {resetLearningOpen && (
        <ResetLearningModal
          onClose={() => setResetLearningOpen(false)}
          onConfirmed={async () => {
            try {
              const n = await api.clearRecoveryFeedback();
              toast.show(`Cleared ${n} feedback rows. Auto-tune reset to neutral.`);
              setResetLearningOpen(false);
            } catch (e) {
              const err = e as GuiError;
              if (err.kind === "Locked") onLockedError();
              else toast.show(`Reset failed: ${err.message ?? err.kind}`);
            }
          }}
        />
      )}
    </div>
  );
}
