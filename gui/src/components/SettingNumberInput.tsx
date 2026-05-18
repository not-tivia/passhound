import { useEffect, useRef, useState } from "react";

interface SettingNumberInputProps {
  label: string;
  unit: string;
  value: number;
  min?: number;
  max?: number;
  hint?: string;
  onSave: (next: number) => Promise<void>;
}

const DEBOUNCE_MS = 400;

export default function SettingNumberInput({
  label, unit, value, min, max, hint, onSave,
}: SettingNumberInputProps) {
  const [text, setText] = useState(String(value));
  const timerRef = useRef<number | null>(null);

  // Reset local text when the underlying value changes (e.g., context refresh).
  useEffect(() => {
    setText(String(value));
  }, [value]);

  useEffect(() => {
    return () => {
      if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    };
  }, []);

  const scheduleSave = (raw: string) => {
    if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    timerRef.current = window.setTimeout(() => {
      const parsed = parseInt(raw, 10);
      let next = Number.isFinite(parsed) && parsed >= 0 ? parsed : 0;
      if (typeof min === "number") next = Math.max(min, next);
      if (typeof max === "number") next = Math.min(max, next);
      void onSave(next);
    }, DEBOUNCE_MS);
  };

  return (
    <div className="settings-row">
      <label className="settings-row__label">{label}</label>
      <input
        className="settings-row__input"
        type="number"
        min={min}
        max={max}
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          scheduleSave(e.target.value);
        }}
      />
      <span className="settings-row__unit">{unit}</span>
      {hint && <span className="settings-row__hint">{hint}</span>}
    </div>
  );
}
