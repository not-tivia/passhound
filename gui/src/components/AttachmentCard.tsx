import { useEffect, useState } from "react";
import { api } from "../api";
import { useToast } from "./Toast";
import { useSettings } from "../context/SettingsContext";
import type { AttachmentSummary, AttachmentRead, GuiError } from "../types";

interface CardProps {
  summary: AttachmentSummary;
  onDelete: () => void;
  onView: (a: AttachmentSummary) => void;
}

function formatSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

export default function AttachmentCard({ summary, onDelete, onView }: CardProps) {
  const isImage = summary.mime_type.startsWith("image/");
  const [revealed, setRevealed] = useState(false);
  const [data, setData] = useState<AttachmentRead | null>(null);
  const toast = useToast();
  const { settings } = useSettings();

  const handleReveal = async () => {
    if (revealed || !isImage) return;
    try {
      const r = await api.readAttachment(summary.id);
      setData(r);
      setRevealed(true);
    } catch (e) {
      const err = e as GuiError;
      toast.show(`Thumbnail error: ${err.message ?? err.kind}`);
    }
  };

  useEffect(() => {
    if (!revealed) return;
    if (settings.reveal_clear_seconds === 0) return;
    const handle = setTimeout(() => {
      setRevealed(false);
      setData(null);
    }, settings.reveal_clear_seconds * 1000);
    return () => clearTimeout(handle);
  }, [revealed, settings.reveal_clear_seconds]);

  const handleDownload = async () => {
    try {
      const d = await api.readAttachment(summary.id);
      const path = await api.saveFileDialog(summary.filename);
      if (!path) return;
      // Use a <a download> Blob URL since we don't have the Tauri fs plugin.
      // Caveat: the OS save dialog returned a path the user chose, but
      // without the fs plugin we can't write to it directly — instead we
      // trigger a browser download which the user can save where they pick.
      const blob = new Blob([base64ToBytes(d.bytes_base64) as unknown as BlobPart], { type: d.mime_type });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = summary.filename;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
      toast.show(`Saved ${summary.filename}`);
    } catch (e) {
      const err = e as GuiError;
      toast.show(`Save failed: ${err.message ?? err.kind}`);
    }
  };

  const handleDelete = async (e: React.MouseEvent) => {
    e.stopPropagation();
    if (!confirm(`Delete attachment "${summary.filename}"?`)) return;
    try {
      await api.deleteAttachment(summary.id);
      onDelete();
    } catch (err) {
      const e = err as GuiError;
      toast.show(`Delete failed: ${e.message ?? e.kind}`);
    }
  };

  if (!isImage) {
    return (
      <div className="attachment-card">
        <div className="attachment-card__icon" onClick={handleDownload}>
          {"📄"}
        </div>
        <div className="attachment-card__meta">
          <div className="attachment-card__filename" title={summary.filename}>
            {summary.filename}
          </div>
          <div className="attachment-card__size">{formatSize(summary.size_bytes)}</div>
        </div>
        <button
          className="attachment-card__delete"
          onClick={handleDelete}
          title="Delete attachment"
        >
          &times;
        </button>
      </div>
    );
  }

  if (!revealed) {
    return (
      <div className="attachment-card">
        <button
          type="button"
          className="attachment-card__placeholder"
          onClick={handleReveal}
          aria-label={`Reveal image attachment: ${summary.filename}`}
        >
          <span className="attachment-card__placeholder-icon" aria-hidden="true">&#x1f5bc;&#xfe0f;</span>
          <span className="attachment-card__placeholder-hint">Click to view</span>
        </button>
        <div className="attachment-card__meta">
          <div className="attachment-card__filename" title={summary.filename}>
            {summary.filename}
          </div>
          <div className="attachment-card__size">{formatSize(summary.size_bytes)}</div>
        </div>
        <button
          className="attachment-card__delete"
          onClick={handleDelete}
          title="Delete attachment"
        >
          &times;
        </button>
      </div>
    );
  }

  return (
    <div className="attachment-card">
      <img
        className="attachment-card__thumb"
        src={`data:${data!.mime_type};base64,${data!.bytes_base64}`}
        alt={summary.filename}
        onClick={() => onView(summary)}
      />
      <div className="attachment-card__meta">
        <div className="attachment-card__filename" title={summary.filename}>
          {summary.filename}
        </div>
        <div className="attachment-card__size">{formatSize(summary.size_bytes)}</div>
      </div>
      <button
        className="attachment-card__delete"
        onClick={handleDelete}
        title="Delete attachment"
      >
        &times;
      </button>
    </div>
  );
}

function base64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}
