interface SettingCheckboxProps {
  label: string;
  value: boolean;
  onSave: (next: boolean) => Promise<void>;
}

export default function SettingCheckbox({ label, value, onSave }: SettingCheckboxProps) {
  return (
    <div className="settings-row">
      <label className="settings-row__label">{label}</label>
      <input
        type="checkbox"
        className="settings-row__checkbox"
        checked={value}
        onChange={(e) => { void onSave(e.target.checked); }}
      />
    </div>
  );
}
