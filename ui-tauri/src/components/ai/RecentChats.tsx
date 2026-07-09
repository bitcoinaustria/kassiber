import { useTranslation } from "react-i18next";
import { History } from "lucide-react";

import { useAssistantSession } from "@/components/ai/assistantSession";
import { useDaemon } from "@/daemon/client";

interface ChatSessionRow {
  id: string;
  title: string;
  updated_at: string;
  message_count?: number;
}

interface ChatSessionsListShape {
  sessions?: ChatSessionRow[];
  history_enabled?: boolean;
}

function shortDate(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

/**
 * Quick-resume list for the empty assistant page: the three most recent saved
 * chats, one click from continuing. Renders nothing while loading, when
 * history is off, or when there is nothing to resume — the empty page stays
 * quiet unless there is a real signal.
 */
export function RecentChats() {
  const { t } = useTranslation("assistant");
  const { isStreaming, resumeSession } = useAssistantSession();
  const list = useDaemon<ChatSessionsListShape>(
    "ui.chat.sessions.list",
    { limit: 3 },
    { staleTime: 30 * 1000 },
  );
  const sessions = list.data?.data?.sessions ?? [];
  if (sessions.length === 0) return null;

  return (
    <div className="w-full max-w-md">
      <div className="mb-1.5 px-3 text-left text-xs font-medium text-muted-foreground">
        {t("page.recentChats")}
      </div>
      <div className="flex flex-col">
        {sessions.map((session) => (
          <button
            key={session.id}
            type="button"
            disabled={isStreaming}
            onClick={() => void resumeSession(session.id).catch(() => {})}
            className="flex items-center gap-2.5 rounded-lg px-3 py-2 text-left text-sm outline-none transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
          >
            <History
              className="size-3.5 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
            <span className="min-w-0 flex-1 truncate text-foreground">
              {session.title}
            </span>
            <span className="shrink-0 text-xs text-muted-foreground">
              {shortDate(session.updated_at)}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
