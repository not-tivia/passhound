import { useEffect, useState } from "react";
import { api } from "../api";
import { useToast } from "./Toast";
import type { AttachmentSummary, GuiError } from "../types";

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
  const [thumbUrl, setThumbUrl] = useState<string | null>(null);
  const toast = useToast();

  useEffect(() => {
    if (!isImage) return;
    let cancelled = false;
    api
      .readAttachment(summary.id)
      .then((r) => {
        if (!cancelled) setThumbUrl(`data:${r.mime_type};base64,${r.bytes_base64}`);
      })
      .catch((e: GuiError) => {
        if (!cancelled) toast.show(`Thumbnail error: ${e.message ?? e.kind}`);
      });
    return () => {
      cancelled = true;
    };
  }, [summary.id, isImage]);

  const handleDownload = async () => {
    try {
      const data = await api.readAttachment(summary.id);
      const path = await api.saveFileDialog(summary.filename);
      if (!path) return;
      // Use a <a download> Blob URL since we don't have the Tauri fs plugin.
      // Caveat: the OS save dialog returned a path the user chose, but
      // without the fs plugin we can't write to it directly — instead we
      // trigger a browser download which the user can save where they pick.
      const blob = new Blob([base64ToBytes(data.bytes_base64) as unknown as BlobPart], { type: data.mime_type });
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

  return (
    <div className="attachment-card">
      {isImage && thumbUrl ? (
        <img
          className="attachment-card__thumb"
          src={thumbUrl}
          alt={summary.filename}
          onClick={() => onView(summary)}
        />
      ) : (
        <div className="attachment-card__icon" onClick={handleDownload}>
          {isImage ? "..." : "📄"}
        </div>
      )}
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
