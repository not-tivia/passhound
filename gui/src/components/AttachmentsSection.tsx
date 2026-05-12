import { useCallback, useEffect, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../api";
import AttachmentCard from "./AttachmentCard";
import AttachmentViewer from "./AttachmentViewer";
import { useToast } from "./Toast";
import type { AttachmentSummary, GuiError } from "../types";

interface SectionProps {
  accountId: number;
}

function guessMimeType(filename: string): string {
  const lower = filename.toLowerCase();
  if (lower.endsWith(".png")) return "image/png";
  if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
  if (lower.endsWith(".gif")) return "image/gif";
  if (lower.endsWith(".webp")) return "image/webp";
  if (lower.endsWith(".pdf")) return "application/pdf";
  if (lower.endsWith(".txt")) return "text/plain";
  return "application/octet-stream";
}

export default function AttachmentsSection({ accountId }: SectionProps) {
  const [list, setList] = useState<AttachmentSummary[]>([]);
  const [dragging, setDragging] = useState(false);
  const [viewing, setViewing] = useState<AttachmentSummary | null>(null);
  const toast = useToast();

  const refresh = useCallback(async () => {
    try {
      const result = await api.listAttachments(accountId);
      setList(result);
    } catch (e) {
      const err = e as GuiError;
      toast.show(`List failed: ${err.message ?? err.kind}`);
    }
  }, [accountId, toast]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const attachOne = async (file: File) => {
    const reader = new FileReader();
    reader.onload = async () => {
      try {
        // result is "data:<mime>;base64,<bytes>"; strip the prefix.
        const result = reader.result as string;
        const commaIdx = result.indexOf(",");
        const bytesBase64 = commaIdx >= 0 ? result.slice(commaIdx + 1) : result;
        await api.attachFile(
          accountId,
          file.name,
          file.type || guessMimeType(file.name),
          bytesBase64,
        );
        toast.show(`Attached ${file.name}`);
        refresh();
      } catch (e) {
        const err = e as GuiError;
        toast.show(`Attach failed: ${err.message ?? err.kind}`);
      }
    };
    reader.onerror = () => {
      toast.show(`Read failed for ${file.name}`);
    };
    reader.readAsDataURL(file);
  };

  const handleDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    setDragging(false);
    const files = Array.from(e.dataTransfer.files);
    for (const f of files) {
      await attachOne(f);
    }
  };

  const pickAndAttach = async () => {
    try {
      const picked = await openDialog({ multiple: true });
      if (!picked) return;
      const paths = Array.isArray(picked) ? picked : [picked];
      for (const p of paths) {
        // Read via fetch on the file path — Tauri's WebView allows this for
        // local file:// when capability permits, but more portably we use
        // a hidden <input type=file> instead. For simplicity, fall back to
        // a basic approach: ask the user to drag-drop for files. For now,
        // surface a toast asking the user to drag-drop.
        toast.show(`Selected ${p} — drag-drop to attach (file path read needs Phase 4.x fs plugin)`);
        void p;
      }
    } catch (e) {
      const err = e as GuiError;
      toast.show(`Pick failed: ${err.message ?? err.kind}`);
    }
  };

  return (
    <section className="attachments">
      <div className="attachments__label">
        ATTACHMENTS ({list.length})
      </div>

      <div
        className={`attachments__dropzone${dragging ? " is-dragging" : ""}`}
        onDragOver={(e) => {
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={handleDrop}
      >
        Drop files here, or{" "}
        <button onClick={pickAndAttach}>click to browse</button>
      </div>

      {list.length > 0 && (
        <div className="attachments__grid">
          {list.map((a) => (
            <AttachmentCard
              key={a.id}
              summary={a}
              onDelete={refresh}
              onView={setViewing}
            />
          ))}
        </div>
      )}

      {viewing && (
        <AttachmentViewer
          attachment={viewing}
          onClose={() => setViewing(null)}
        />
      )}
    </section>
  );
}
