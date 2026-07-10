import * as React from "react";
import { useTranslation } from "react-i18next";
import { currentUiLocale } from "@/lib/localeFormat";
import { History, Trash2 } from "lucide-react";

import { useAssistantSession } from "@/components/ai/assistantSession";
import { Button } from "@/components/ui/button";
import {
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from "@/components/ui/dropdown-menu";
import { useDaemon, useDaemonMutation } from "@/daemon/client";

interface ChatSessionRow {
  id: string;
  title: string;
  updated_at: string;
  message_count?: number;
}

interface ChatSessionsListShape {
  sessions?: ChatSessionRow[];
  history_mode?: string;
  history_enabled?: boolean;
}

function formatUpdatedAt(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString(currentUiLocale(), {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

/**
 * Saved-chat history rendered as a submenu so it can nest inside the
 * assistant's "more actions" (…) menu instead of taking a top-level toolbar
 * slot. The sessions query only runs while the submenu is open.
 */
export function ChatHistoryPanel() {
  const { t } = useTranslation("assistant");
  const { isStreaming, resumeSession, sessionId, forgetSession } =
    useAssistantSession();
  const [open, setOpen] = React.useState(false);
  const [resumeError, setResumeError] = React.useState<string | null>(null);
  const list = useDaemon<ChatSessionsListShape>(
    "ui.chat.sessions.list",
    { limit: 30 },
    { enabled: open, staleTime: 0 },
  );
  const deleteSession = useDaemonMutation("ui.chat.sessions.delete");

  const sessions = list.data?.data?.sessions ?? [];
  const historyEnabled = list.data?.data?.history_enabled ?? false;

  const onResume = React.useCallback(
    (id: string) => {
      setResumeError(null);
      setOpen(false);
      void resumeSession(id).catch((caught: unknown) => {
        setResumeError(
          caught instanceof Error ? caught.message : String(caught),
        );
        setOpen(true);
      });
    },
    [resumeSession],
  );

  return (
    <DropdownMenuSub open={open} onOpenChange={setOpen}>
      <DropdownMenuSubTrigger>
        <History className="size-4" aria-hidden="true" />
        {t("history.trigger")}
      </DropdownMenuSubTrigger>
      <DropdownMenuSubContent className="max-h-96 w-80 overflow-y-auto">
        <DropdownMenuLabel>{t("history.savedChats")}</DropdownMenuLabel>
        <DropdownMenuSeparator />
        {list.isLoading ? (
          <div className="px-2 py-3 text-sm text-muted-foreground">
            {t("history.loading")}
          </div>
        ) : null}
        {!list.isLoading && sessions.length === 0 ? (
          <div className="px-2 py-3 text-sm text-muted-foreground">
            {historyEnabled ? t("history.empty") : t("history.disabled")}
          </div>
        ) : null}
        {sessions.map((session) => (
          <div
            key={session.id}
            className="flex items-start gap-2 rounded-sm px-2 py-1.5 hover:bg-accent"
          >
            <button
              type="button"
              disabled={isStreaming}
              onClick={() => onResume(session.id)}
              className="min-w-0 flex-1 text-left outline-none disabled:pointer-events-none disabled:opacity-50"
            >
              <div className="truncate text-sm font-medium">
                {session.title}
                {session.id === sessionId ? ` ${t("history.current")}` : ""}
              </div>
              <div className="text-xs text-muted-foreground">
                {t("history.entryMeta", {
                  date: formatUpdatedAt(session.updated_at),
                  messages: t("history.messageCount", {
                    count: session.message_count ?? 0,
                  }),
                })}
              </div>
            </button>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="size-7 shrink-0 text-muted-foreground hover:text-destructive"
              aria-label={t("history.deleteChat", { title: session.title })}
              onClick={(event) => {
                event.preventDefault();
                event.stopPropagation();
                deleteSession.mutate({ session_id: session.id });
                // Deleting the conversation's own session must detach it,
                // or the next turn would target a missing session.
                forgetSession(session.id);
              }}
            >
              <Trash2 className="size-3.5" aria-hidden="true" />
            </Button>
          </div>
        ))}
        {resumeError ? (
          <div className="px-2 py-2 text-xs text-destructive">
            {resumeError}
          </div>
        ) : null}
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  );
}
