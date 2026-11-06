import { File, FileImage, FileSpreadsheet, FileText, Loader2, X } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  chatAttachmentKind,
  chatAttachmentTypeLabel,
  formatAttachmentSize,
  type ChatAttachmentView,
} from "@/components/ai/chatAttachments";
import { cn } from "@/lib/utils";

const KIND_ICONS = {
  spreadsheet: FileSpreadsheet,
  delimited: FileSpreadsheet,
  pdf: FileText,
  image: FileImage,
} as const;

/**
 * A file in the composer or on the message it was sent with, modelled on how
 * T3 Code shows composer attachments: a thumbnail or type tile, the name, the
 * type and size, and a remove button on hover. Kassiber's files are mostly
 * exports, so the name stays readable instead of shrinking to a thumbnail.
 */
export function ChatAttachmentCard({
  attachment,
  onRemove,
  uploading = false,
  note,
  className,
}: {
  attachment: ChatAttachmentView;
  onRemove?: () => void;
  uploading?: boolean;
  /** A short line under the name in place of the type and size. */
  note?: string;
  className?: string;
}) {
  const { t, i18n } = useTranslation("assistant");
  const kind = chatAttachmentKind(attachment.filename, attachment.kind);
  const Icon = kind ? KIND_ICONS[kind] : File;
  const type = chatAttachmentTypeLabel(attachment.filename);
  const details =
    note ??
    [type, attachment.sizeBytes ? formatAttachmentSize(attachment.sizeBytes, i18n.language) : null]
      .filter(Boolean)
      .join(" · ");
  return (
    <div
      className={cn(
        "group/attachment relative inline-flex max-w-[16rem] min-w-0 items-center gap-2.5 rounded-xl border border-border/70 bg-background/60 p-1.5 pr-3 text-left",
        className,
      )}
      data-chat-attachment=""
      aria-busy={uploading || undefined}
    >
      <span className="relative flex size-9 shrink-0 items-center justify-center overflow-hidden rounded-lg bg-muted text-muted-foreground">
        {attachment.previewUrl ? (
          <img src={attachment.previewUrl} alt="" className="size-full object-cover" />
        ) : (
          <Icon className="size-4" aria-hidden="true" />
        )}
        {uploading ? (
          <span className="absolute inset-0 flex items-center justify-center bg-background/70">
            <Loader2 className="size-4 animate-spin" aria-hidden="true" />
          </span>
        ) : null}
      </span>
      <span className="flex min-w-0 flex-col">
        <span className="truncate text-xs font-medium text-foreground" title={attachment.filename}>
          {attachment.filename}
        </span>
        <span className="truncate text-2xs text-muted-foreground">
          {uploading ? t("composer.attachmentUploading") : details}
        </span>
      </span>
      {onRemove ? (
        <button
          type="button"
          onClick={onRemove}
          aria-label={t("composer.removeAttachmentNamed", { filename: attachment.filename })}
          title={t("composer.removeAttachment")}
          className="absolute -top-1.5 -right-1.5 flex size-5 items-center justify-center rounded-full border border-border bg-background text-muted-foreground opacity-0 shadow-sm outline-none transition-opacity group-hover/attachment:opacity-100 hover:text-foreground focus-visible:opacity-100 focus-visible:ring-2 focus-visible:ring-ring"
        >
          <X className="size-3" aria-hidden="true" />
        </button>
      ) : null}
    </div>
  );
}
