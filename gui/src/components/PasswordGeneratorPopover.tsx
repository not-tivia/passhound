import { useState } from "react";
import { api } from "../api";
import type { GeneratorOptionsPayload, GuiError } from "../types";

interface PasswordGeneratorPopoverProps {
  onChoose: (password: string) => void;
  onClose: () => void;
}

export default function PasswordGeneratorPopover({ onChoose, onClose }: PasswordGeneratorPopoverProps) {
  const [length, setLength] = useState(16);
  const [lowercase, setLowercase] = useState(true);
  const [uppercase, setUppercase] = useState(true);
  const [digits, setDigits] = useState(true);
  const [symbols, setSymbols] = useState(true);
  const [avoidAmbiguous, setAvoidAmbiguous] = useState(false);
  const [generated, setGenerated] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleGenerate = async () => {
    setBusy(true);
    setError(null);
    try {
      const payload: GeneratorOptionsPayload = {
        length,
        lowercase,
        uppercase,
        digits,
        symbols,
        avoidAmbiguous,
      };
      const pw = await api.generatePassword(payload);
      setGenerated(pw);
    } catch (e) {
      const err = e as GuiError;
      setError(err.message ?? err.kind);
    } finally {
      setBusy(false);
    }
  };

  const handleUse = () => {
    if (generated) {
      onChoose(generated);
      onClose();
    }
  };

  return (
    <div className="popover-backdrop" onClick={onClose}>
      <div className="popover popover--password-gen" onClick={(e) => e.stopPropagation()}>
        <div className="popover__header">Password generator</div>
        <div className="popover__field">
          <label>Length: {length}</label>
          <input
            type="range"
            min={8} max={64} step={1}
            value={length}
            onChange={(e) => setLength(parseInt(e.target.value, 10))}
            disabled={busy}
          />
        </div>
        <div className="popover__checks">
          <label><input type="checkbox" checked={lowercase} onChange={(e) => setLowercase(e.target.checked)} disabled={busy} /> lowercase</label>
          <label><input type="checkbox" checked={uppercase} onChange={(e) => setUppercase(e.target.checked)} disabled={busy} /> UPPERCASE</label>
          <label><input type="checkbox" checked={digits} onChange={(e) => setDigits(e.target.checked)} disabled={busy} /> digits</label>
          <label><input type="checkbox" checked={symbols} onChange={(e) => setSymbols(e.target.checked)} disabled={busy} /> symbols</label>
        </div>
        <label className="popover__avoid">
          <input type="checkbox" checked={avoidAmbiguous} onChange={(e) => setAvoidAmbiguous(e.target.checked)} disabled={busy} />
          Avoid ambiguous (l, I, L, O, 0, 1)
        </label>
        {generated && (
          <div className="popover__output">{generated}</div>
        )}
        {error && <div className="popover__error">{error}</div>}
        <div className="popover__actions">
          <button onClick={handleGenerate} disabled={busy}>{busy ? "Generating…" : "Generate"}</button>
          <button onClick={handleUse} disabled={!generated || busy}>Use this</button>
          <button onClick={onClose} disabled={busy}>Cancel</button>
        </div>
      </div>
    </div>
  );
}
