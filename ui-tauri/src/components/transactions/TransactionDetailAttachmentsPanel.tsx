import {
  Check,
  Copy,
  FileText,
  Link2,
  Paperclip,
  Pencil,
  X,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

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

const MAX_ATTACHMENT_LABEL_LENGTH = 200;

function AttachLinksDialog({
  open,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (urls: string[]) => void | Promise<void>;
}) {
  const { t } = useTranslation(["transactions", "common"]);
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
          errors.push(t("attachments.linksDialog.invalidProtocol", { url: line }));
          continue;
        }
        urls.push(line);
      } catch {
        errors.push(t("attachments.linksDialog.invalidUrl", { url: line }));
      }
    }
    return { urls, errors };
  }, [text, t]);

  const submit = async () => {
    if (!parsed.urls.length) {
      setError(t("attachments.linksDialog.addAtLeastOne"));
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
      setError(
        err instanceof Error
          ? err.message
          : t("attachments.linksDialog.couldNotAdd"),
      );
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("attachments.linksDialog.title")}</DialogTitle>
          <DialogDescription>
            {t("attachments.linksDialog.description")}
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-2">
          <Label htmlFor="attach-links-text">{t("attachments.linksDialog.urls")}</Label>
          <Textarea
            id="attach-links-text"
            value={text}
            onChange={(e) => {
              setText(e.target.value);
              setError(null);
            }}
            className="min-h-32 font-mono text-xs"
            placeholder={t("attachments.linksDialog.placeholder")}
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
                ? t("attachments.linksDialog.noUrls")
                : t("attachments.linksDialog.urlsReady", {
                    count: parsed.urls.length,
                  })}
              {parsed.errors.length ? (
                <span className="ml-1 text-amber-600 dark:text-amber-400">
                  {t("attachments.linksDialog.invalidLines", {
                    count: parsed.errors.length,
                  })}
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
            {t("common:actions.cancel")}
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
              ? t("attachments.linksDialog.attaching")
              : parsed.urls.length > 1
                ? t("attachments.linksDialog.attachMany", {
                    count: parsed.urls.length,
                  })
                : t("attachments.linksDialog.attachOne")}
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
  const { t } = useTranslation("transactions");
  const [linkDialogOpen, setLinkDialogOpen] = React.useState(false);
  const [pickerBusy, setPickerBusy] = React.useState(false);
  const [editingId, setEditingId] = React.useState<string | null>(null);
  const [editingLabel, setEditingLabel] = React.useState("");
  const [renameError, setRenameError] = React.useState<string | null>(null);
  const [savingRenameId, setSavingRenameId] = React.useState<string | null>(
    null,
  );
  const renameButtonRefs = React.useRef(new Map<string, HTMLButtonElement>());
  const wired = Boolean(onAddFiles || onAddLinks);
  const filePickerEnabled = Boolean(onAddFiles) && isFilePickerAvailable;

  const [pickerError, setPickerError] = React.useState<string | null>(null);
  const handlePickFiles = async () => {
    if (!onAddFiles) return;
    setPickerError(null);
    setPickerBusy(true);
    try {
      const paths = await pickFiles({
        title: t("attachments.pickerTitle"),
      });
      if (paths.length) {
        await onAddFiles(paths);
      }
    } catch (err) {
      setPickerError(
        err instanceof Error ? err.message : t("attachments.pickerError"),
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

  const focusRenameTrigger = (itemId: string | null) => {
    if (!itemId) return;
    window.requestAnimationFrame(() => {
      renameButtonRefs.current.get(itemId)?.focus();
    });
  };

  const cancelRename = (itemId = editingId) => {
    setEditingId(null);
    setEditingLabel("");
    setRenameError(null);
    focusRenameTrigger(itemId);
  };

  const saveRename = async (item: AttachmentItem) => {
    if (!onRename) return;
    const label = editingLabel.trim();
    if (!label) {
      setRenameError(t("attachments.addLinkText"));
      return;
    }
    if (label.length > MAX_ATTACHMENT_LABEL_LENGTH) {
      setRenameError(
        t("attachments.linkTextTooLong", { max: MAX_ATTACHMENT_LABEL_LENGTH }),
      );
      return;
    }
    if (label === item.label) {
      cancelRename(item.id);
      return;
    }
    setSavingRenameId(item.id);
    setRenameError(null);
    try {
      await onRename(item, label);
      cancelRename(item.id);
    } catch (err) {
      setRenameError(
        err instanceof Error ? err.message : t("attachments.couldNotRename"),
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
          {t("attachments.title")}
        </div>
        <div className="flex items-center gap-1.5">
          <span className="text-xs tabular-nums text-muted-foreground">
            {items.length}
          </span>
          {onReuseEvidence ? (
            <Button
              type="button"
              variant="outline"
              size="icon"
              className="size-7 shrink-0"
              aria-label={t("attachments.reuseEvidenceAria")}
              title={t("attachments.reuseEvidenceAria")}
              onClick={onReuseEvidence}
            >
              <Copy className="size-3.5" aria-hidden="true" />
            </Button>
          ) : null}
        </div>
      </div>
      {items.length ? (
        <ul className="mb-2 space-y-1.5">
          {items.map((item) => {
            const hiddenTitle = hideSensitive
              ? t("attachments.detailHidden")
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
                          cancelRename(item.id);
                        }
                      }}
                      maxLength={MAX_ATTACHMENT_LABEL_LENGTH}
                      className="h-7 text-xs"
                      aria-label={t("attachments.linkTextAria", {
                        name: item.detail || item.label,
                      })}
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
                        {t("attachments.reused")}
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
                        {t("attachments.reused")}
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
                      aria-label={t("attachments.saveLinkText")}
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
                      aria-label={t("attachments.cancelEdit")}
                      disabled={renameBusy}
                      onClick={() => cancelRename(item.id)}
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
                    aria-label={t("attachments.editLinkTextAria", { name: item.label })}
                    ref={(node) => {
                      if (node) {
                        renameButtonRefs.current.set(item.id, node);
                      } else {
                        renameButtonRefs.current.delete(item.id);
                      }
                    }}
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
                      hideSensitive
                        ? t("attachments.removeAttachment")
                        : t("attachments.removeNamed", { name: item.label })
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
          {t("attachments.emptyBody")}
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
      <div className="grid grid-cols-2 gap-1.5">
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="gap-1.5"
          disabled={!wired || !filePickerEnabled || pickerBusy}
          title={
            !filePickerEnabled && wired
              ? t("attachments.nativePickerUnavailable")
              : undefined
          }
          onClick={handlePickFiles}
        >
          <FileText className="size-3.5" aria-hidden="true" />
          {pickerBusy ? t("attachments.picking") : t("attachments.attachFiles")}
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
          {t("attachments.attachLinks")}
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
