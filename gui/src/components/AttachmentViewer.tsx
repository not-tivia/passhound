import { useEffect, useState } from "react";
import { api } from "../api";
import type { AttachmentSummary, GuiError } from "../types";

interface ViewerProps {
  attachment: AttachmentSummary;
  onClose: () => void;
}

export default function AttachmentViewer({ attachment, onClose }: ViewerProps) {
  const [src, setSrc] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .readAttachment(attachment.id)
      .then((r) => setSrc(`data:${r.mime_type};base64,${r.bytes_base64}`))
      .catch((e: GuiError) => setError(e.message ?? e.kind));
  }, [attachment.id]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="attachment-viewer" onClick={onClose}>
      {error && <div className="attachment-viewer__error">{error}</div>}
      {src && !error && (
        <img
          src={src}
          alt={attachment.filename}
          onClick={(e) => e.stopPropagation()}
        />
      )}
    </div>
  );
}
