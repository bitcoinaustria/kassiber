import {
  Check,
  FileText,
  Link2,
  Paperclip,
  Pencil,
  Repeat2,
  X,
} from "lucide-react";
import * as React from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { isFilePickerAvailable, pickFiles } from "@/lib/filePicker";
import { cn } from "@/lib/utils";

import { blurClass } from "./model";
import type { AttachmentItem } from "./TransactionDetailSheetParts";

function AttachLinksDialog({
  open,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (urls: string[]) => void | Promise<void>;
}) {
  const [text, setText] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const [submitting, setSubmitting] = React.useState(false);

  React.useEffect(() => {
    if (open) {
      setText("");
      setError(null);
      setSubmitting(false);
    }
  }, [open]);

  const parsed = React.useMemo(() => {
    const lines = text
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter(Boolean);
    const errors: string[] = [];
    const urls: string[] = [];
    for (const line of lines) {
      try {
        const u = new URL(line);
        if (
          u.protocol !== "http:" &&
          u.protocol !== "https:" &&
          u.protocol !== "ipfs:"
        ) {
          errors.push(`${line} — must start with http://, https://, or ipfs://`);
          continue;
        }
        urls.push(line);
      } catch {
        errors.push(`${line} — not a valid URL`);
      }
    }
    return { urls, errors };
  }, [text]);

  const submit = async () => {
    if (!parsed.urls.length) {
      setError("Add at least one URL.");
      return;
    }
    if (parsed.errors.length) {
      setError(parsed.errors[0]);
      return;
    }
    setSubmitting(true);
    try {
      await onSubmit(parsed.urls);
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Could not add links.");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Attach links</DialogTitle>
          <DialogDescription>
            One URL per line. Links stay as references; Kassiber may save a
            page title as link text.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-2">
          <Label htmlFor="attach-links-text">URLs</Label>
          <Textarea
            id="attach-links-text"
            value={text}
            onChange={(e) => {
              setText(e.target.value);
              setError(null);
            }}
            className="min-h-32 font-mono text-xs"
            placeholder={"https://btcpay.example/invoice/abc123\nhttps://drive.example/receipt-2026-04.pdf"}
            autoFocus
            aria-describedby="attach-links-status"
            aria-invalid={parsed.errors.length > 0}
          />
          <div className="flex items-center justify-between gap-2 text-xs">
            <span
              id="attach-links-status"
              role="status"
              aria-live="polite"
              className="text-muted-foreground"
            >
              {parsed.urls.length === 0
                ? "No URLs yet"
                : `${parsed.urls.length} URL${parsed.urls.length === 1 ? "" : "s"} ready`}
              {parsed.errors.length ? (
                <span className="ml-1 text-amber-600 dark:text-amber-400">
                  · {parsed.errors.length} invalid line
                  {parsed.errors.length === 1 ? "" : "s"} to fix
                </span>
              ) : null}
            </span>
            {error ? (
              <span role="alert" className="text-destructive">
                {error}
              </span>
            ) : null}
          </div>
        </div>
        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
            disabled={submitting}
          >
            Cancel
          </Button>
          <Button
            type="button"
            disabled={
              submitting ||
              parsed.urls.length === 0 ||
              parsed.errors.length > 0
            }
            onClick={submit}
          >
            {submitting
              ? "Attaching"
              : parsed.urls.length > 1
                ? `Attach ${parsed.urls.length} links`
                : "Attach link"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function AttachmentsPanel({
  items = [],
  hideSensitive,
  onAddFiles,
  onAddLinks,
  onReuseEvidence,
  onOpen,
  onRename,
  onRemove,
}: {
  items?: AttachmentItem[];
  hideSensitive: boolean;
  onAddFiles?: (paths: string[]) => void | Promise<void>;
  onAddLinks?: (urls: string[]) => void | Promise<void>;
  onReuseEvidence?: () => void;
  onOpen?: (item: AttachmentItem) => void;
  onRename?: (item: AttachmentItem, label: string) => void | Promise<void>;
  onRemove?: (item: AttachmentItem) => void;
}) {
  const [linkDialogOpen, setLinkDialogOpen] = React.useState(false);
  const [pickerBusy, setPickerBusy] = React.useState(false);
  const [editingId, setEditingId] = React.useState<string | null>(null);
  const [editingLabel, setEditingLabel] = React.useState("");
  const [renameError, setRenameError] = React.useState<string | null>(null);
  const [savingRenameId, setSavingRenameId] = React.useState<string | null>(
    null,
  );
  const wired = Boolean(onAddFiles || onAddLinks);
  const filePickerEnabled = Boolean(onAddFiles) && isFilePickerAvailable;

  const [pickerError, setPickerError] = React.useState<string | null>(null);
  const handlePickFiles = async () => {
    if (!onAddFiles) return;
    setPickerError(null);
    setPickerBusy(true);
    try {
      const paths = await pickFiles({
        title: "Attach files to this transaction",
      });
      if (paths.length) {
        await onAddFiles(paths);
      }
    } catch (err) {
      setPickerError(
        err instanceof Error ? err.message : "Could not open file picker.",
      );
    } finally {
      setPickerBusy(false);
    }
  };

  const startRename = (item: AttachmentItem) => {
    setEditingId(item.id);
    setEditingLabel(item.label);
    setRenameError(null);
  };

  const cancelRename = () => {
    setEditingId(null);
    setEditingLabel("");
    setRenameError(null);
  };

  const saveRename = async (item: AttachmentItem) => {
    if (!onRename) return;
    const label = editingLabel.trim();
    if (!label) {
      setRenameError("Add link text.");
      return;
    }
    if (label === item.label) {
      cancelRename();
      return;
    }
    setSavingRenameId(item.id);
    setRenameError(null);
    try {
      await onRename(item, label);
      cancelRename();
    } catch (err) {
      setRenameError(
        err instanceof Error ? err.message : "Could not rename link.",
      );
    } finally {
      setSavingRenameId(null);
    }
  };

  return (
    <div className="rounded-md border bg-card p-3">
      <div className="mb-2 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-sm font-semibold">
          <Paperclip
            className="size-4 text-muted-foreground"
            aria-hidden="true"
          />
          Attachments
        </div>
        <span className="text-xs tabular-nums text-muted-foreground">
          {items.length}
        </span>
      </div>
      {items.length ? (
        <ul className="mb-2 space-y-1.5">
          {items.map((item) => {
            const hiddenTitle = hideSensitive
              ? "Attachment detail hidden"
              : item.detail;
            const isEditing = editingId === item.id;
            const canRename = Boolean(
              onRename && item.kind === "url" && !hideSensitive,
            );
            const renameBusy = savingRenameId === item.id;
            return (
              <li
                key={item.id}
                className="flex min-w-0 items-center gap-2 rounded-md border bg-background px-2 py-1.5"
              >
                <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-muted text-muted-foreground">
                  {item.kind === "url" ? (
                    <Link2 className="size-3.5" aria-hidden="true" />
                  ) : (
                    <FileText className="size-3.5" aria-hidden="true" />
                  )}
                </span>
                {isEditing ? (
                  <div className="min-w-0 flex-1">
                    <Input
                      value={editingLabel}
                      onChange={(event) => {
                        setEditingLabel(event.target.value);
                        setRenameError(null);
                      }}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          event.preventDefault();
                          void saveRename(item);
                        }
                        if (event.key === "Escape") {
                          event.preventDefault();
                          cancelRename();
                        }
                      }}
                      className="h-7 text-xs"
                      aria-label={`Link text for ${item.detail || item.label}`}
                      disabled={renameBusy}
                      autoFocus
                    />
                    {item.detail ? (
                      <div className="truncate text-[10px] text-muted-foreground">
                        {item.detail}
                      </div>
                    ) : null}
                    {renameError ? (
                      <div role="alert" className="text-[10px] text-destructive">
                        {renameError}
                      </div>
                    ) : null}
                  </div>
                ) : onOpen ? (
                  <button
                    type="button"
                    onClick={() => onOpen(item)}
                    className="min-w-0 flex-1 text-left hover:underline"
                    title={hiddenTitle}
                  >
                    <div
                      className={cn(
                        "truncate text-xs font-medium",
                        blurClass(hideSensitive),
                      )}
                    >
                      {item.label}
                    </div>
                    {item.detail ? (
                      <div
                        className={cn(
                          "truncate text-[10px] text-muted-foreground",
                          blurClass(hideSensitive),
                        )}
                      >
                        {item.detail}
                      </div>
                    ) : null}
                    {item.copiedFromAttachmentId ? (
                      <Badge variant="secondary" className="mt-1 rounded-md">
                        reused
                      </Badge>
                    ) : null}
                  </button>
                ) : (
                  <div className="min-w-0 flex-1" title={hiddenTitle}>
                    <div
                      className={cn(
                        "truncate text-xs font-medium",
                        blurClass(hideSensitive),
                      )}
                    >
                      {item.label}
                    </div>
                    {item.detail ? (
                      <div
                        className={cn(
                          "truncate text-[10px] text-muted-foreground",
                          blurClass(hideSensitive),
                        )}
                      >
                        {item.detail}
                      </div>
                    ) : null}
                    {item.copiedFromAttachmentId ? (
                      <Badge variant="secondary" className="mt-1 rounded-md">
                        reused
                      </Badge>
                    ) : null}
                  </div>
                )}
                {isEditing ? (
                  <>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="size-6 shrink-0 text-muted-foreground"
                      aria-label="Save link text"
                      disabled={renameBusy || !editingLabel.trim()}
                      onClick={() => void saveRename(item)}
                    >
                      <Check className="size-3.5" aria-hidden="true" />
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="size-6 shrink-0 text-muted-foreground"
                      aria-label="Cancel link text edit"
                      disabled={renameBusy}
                      onClick={cancelRename}
                    >
                      <X className="size-3.5" aria-hidden="true" />
                    </Button>
                  </>
                ) : canRename ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="size-6 shrink-0 text-muted-foreground"
                    aria-label={`Edit link text for ${item.label}`}
                    onClick={() => startRename(item)}
                  >
                    <Pencil className="size-3.5" aria-hidden="true" />
                  </Button>
                ) : null}
                {!isEditing && onRemove ? (
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="size-6 shrink-0 text-muted-foreground"
                    aria-label={
                      hideSensitive ? "Remove attachment" : `Remove ${item.label}`
                    }
                    onClick={() => onRemove(item)}
                  >
                    <X className="size-3.5" aria-hidden="true" />
                  </Button>
                ) : null}
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="mb-2 text-xs text-muted-foreground">
          Attach receipts, invoices, or accountant notes. Add as many as you
          need — local files are copied into the attachments folder, links
          stay as references.
        </p>
      )}
      {pickerError ? (
        <p
          role="alert"
          className="mb-2 text-xs text-destructive"
        >
          {pickerError}
        </p>
      ) : null}
      <div className="grid gap-1.5 sm:grid-cols-3">
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="gap-1.5"
          disabled={!wired || !filePickerEnabled || pickerBusy}
          title={
            !filePickerEnabled && wired
              ? "Native file picker not available in this runtime"
              : undefined
          }
          onClick={handlePickFiles}
        >
          <FileText className="size-3.5" aria-hidden="true" />
          {pickerBusy ? "Picking…" : "Attach files"}
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="gap-1.5"
          disabled={!onAddLinks}
          onClick={() => setLinkDialogOpen(true)}
        >
          <Link2 className="size-3.5" aria-hidden="true" />
          Attach links
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="gap-1.5"
          disabled={!onReuseEvidence}
          onClick={onReuseEvidence}
        >
          <Repeat2 className="size-3.5" aria-hidden="true" />
          Reuse
        </Button>
      </div>
      {onAddLinks ? (
        <AttachLinksDialog
          open={linkDialogOpen}
          onOpenChange={setLinkDialogOpen}
          onSubmit={onAddLinks}
        />
      ) : null}
    </div>
  );
}
