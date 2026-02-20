import { useCallback, useRef, useState } from "react";
import { breakDownRequirements } from "../../../services/tauri/agent";
import type { ToastKind } from "../hooks/useToast";

interface TaskBreakdownModalProps {
  isOpen: boolean;
  sessionId: string;
  onClose: () => void;
  onRefreshTasks: () => Promise<void>;
  onShowToast: (msg: string, kind?: ToastKind) => void;
}

/**
 * Modal for breaking down a requirements description into tasks.
 * Matches .modal-overlay + .modal-container structure from agent.html.
 */
export function TaskBreakdownModal({
  isOpen,
  sessionId,
  onClose,
  onRefreshTasks,
  onShowToast,
}: TaskBreakdownModalProps) {
  const [requirements, setRequirements] = useState("");
  const [fileName, setFileName] = useState<string | null>(null);
  const [fileContent, setFileContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [isDragOver, setIsDragOver] = useState(false);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleClose = useCallback(() => {
    setRequirements("");
    setFileName(null);
    setFileContent(null);
    onClose();
  }, [onClose]);

  const handleFileRead = useCallback((file: File) => {
    const reader = new FileReader();
    reader.onload = (e) => {
      const text = e.target?.result as string;
      setFileContent(text);
      setFileName(file.name);
    };
    reader.readAsText(file);
  }, []);

  const handleFileDrop = useCallback(
    (e: React.DragEvent<HTMLDivElement>) => {
      e.preventDefault();
      setIsDragOver(false);
      const file = e.dataTransfer.files[0];
      if (file) handleFileRead(file);
    },
    [handleFileRead],
  );

  const handleFileInput = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (file) handleFileRead(file);
    },
    [handleFileRead],
  );

  const handleBreakdown = useCallback(async () => {
    const text = fileContent
      ? `${requirements}\n\n--- File: ${fileName} ---\n${fileContent}`
      : requirements;
    if (!text.trim()) {
      onShowToast("Please enter requirements or upload a file.", "warning");
      return;
    }
    setLoading(true);
    try {
      const tasks = await breakDownRequirements(sessionId, text.trim());
      await onRefreshTasks();
      onShowToast(
        `Created ${tasks.length} task${tasks.length !== 1 ? "s" : ""} from requirements`,
        "success",
      );
      handleClose();
    } catch (e) {
      onShowToast(`Breakdown failed: ${e}`, "error");
    } finally {
      setLoading(false);
    }
  }, [
    requirements,
    fileContent,
    fileName,
    sessionId,
    onRefreshTasks,
    onShowToast,
    handleClose,
  ]);

  return (
    <div
      className={`modal-overlay${isOpen ? " visible" : ""}`}
      onClick={(e) => e.target === e.currentTarget && handleClose()}
    >
      <div className="modal-container">
        <div className="modal-header">
          <h3>Break Down Requirements</h3>
          <button className="modal-close" onClick={handleClose} title="Close">
            ×
          </button>
        </div>

        <div className="modal-content">
          <p className="modal-description">
            Describe what needs to be built. Gestura will break it down into
            actionable tasks.
          </p>

          <div className="modal-field">
            <label>Requirements</label>
            <textarea
              className="modal-textarea"
              value={requirements}
              onChange={(e) => setRequirements(e.target.value)}
              placeholder="Describe what needs to be built or done..."
              rows={6}
            />
          </div>

          <div className="modal-field">
            <label>Or upload a requirements file</label>
            <div
              className={`file-upload-area${isDragOver ? " drag-over" : ""}`}
              onDragOver={(e) => {
                e.preventDefault();
                setIsDragOver(true);
              }}
              onDragLeave={() => setIsDragOver(false)}
              onDrop={handleFileDrop}
              onClick={() => fileInputRef.current?.click()}
            >
              {fileName ? (
                <span className="file-name">{fileName}</span>
              ) : (
                <span className="file-upload-text">
                  Drop a file here or click to browse
                </span>
              )}
              <input
                ref={fileInputRef}
                type="file"
                accept=".txt,.md,.pdf,.doc,.docx"
                style={{ display: "none" }}
                onChange={handleFileInput}
              />
            </div>
          </div>
        </div>

        <div className="modal-footer">
          <button className="btn-secondary" onClick={handleClose}>
            Cancel
          </button>
          <button
            className="btn-primary"
            onClick={handleBreakdown}
            disabled={loading || (!requirements.trim() && !fileContent)}
          >
            {loading ? "Analyzing..." : "Break Down"}
          </button>
        </div>
      </div>
    </div>
  );
}

