import * as React from "react";
import { MessagesSquare, Trash2 } from "lucide-react";

import { AssistantSessionContext } from "@/components/ai/assistantSession";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useDaemon, useDaemonMutation } from "@/daemon/client";

interface ChatHistoryConfigShape {
  history?: "auto" | "on" | "off";
  history_enabled?: boolean;
  database_encrypted?: boolean;
}

interface ChatSessionsListShape {
  sessions?: { id: string }[];
}

const MODE_DESCRIPTIONS: Record<string, string> = {
  auto: "Store chats only while the database is encrypted.",
  on: "Always store chats, even on an unencrypted database.",
  off: "Never store chats.",
};

export function ChatHistorySettingsCard() {
  // Without a `history` arg the configure kind is a pure read of the
  // effective policy state.
  const configQuery = useDaemon<ChatHistoryConfigShape>(
    "ui.chat.history.configure",
    {},
  );
  const sessionsQuery = useDaemon<ChatSessionsListShape>(
    "ui.chat.sessions.list",
    { limit: 200 },
    { staleTime: 0 },
  );
  const configure = useDaemonMutation("ui.chat.history.configure");
  const clearSessions = useDaemonMutation("ui.chat.sessions.clear");
  const [confirmingClear, setConfirmingClear] = React.useState(false);
  // Optional: when the Assistant provider is mounted, clearing storage must
  // also detach the live conversation from its (now deleted) session.
  const assistantSession = React.useContext(AssistantSessionContext);

  const config = configQuery.data?.data ?? {};
  const mode = config.history ?? "auto";
  const sessionCount = sessionsQuery.data?.data?.sessions?.length ?? 0;

  const effectiveLine = config.history_enabled
    ? config.database_encrypted
      ? "Chats are currently stored inside the encrypted database."
      : "Chats are currently stored inside the unencrypted database because history is set to on."
    : config.database_encrypted
      ? "Chats are not stored."
      : mode === "auto"
        ? "Chats are not stored: the database is not encrypted yet. Encrypting it (secrets init) turns history on."
        : "Chats are not stored.";

  return (
    <div className="space-y-3 rounded-md border bg-background p-4">
      <div className="flex items-start gap-3">
        <MessagesSquare
          className="mt-0.5 size-4 shrink-0 text-muted-foreground"
          aria-hidden="true"
        />
        <div className="min-w-0 flex-1 space-y-1">
          <h3 className="text-sm font-semibold">Chat history</h3>
          <p className="text-sm text-muted-foreground">
            Stored chats live inside the database, never as plaintext files.
            Diagnostics and audit exports do not include them. Resuming a chat
            replays that conversation as model context; otherwise past chats
            are not exposed as an assistant tool.
          </p>
        </div>
      </div>

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <Label htmlFor="settings-chat-history-mode" className="text-sm">
          Store conversations
        </Label>
        <Select
          value={mode}
          onValueChange={(value) => configure.mutate({ history: value })}
        >
          <SelectTrigger
            id="settings-chat-history-mode"
            className="w-full sm:w-72"
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {(["auto", "on", "off"] as const).map((value) => (
              <SelectItem key={value} value={value}>
                <span className="font-medium capitalize">{value}</span>
                <span className="text-muted-foreground">
                  {" — "}
                  {MODE_DESCRIPTIONS[value]}
                </span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <p className="text-xs text-muted-foreground">{effectiveLine}</p>

      <div className="flex items-center justify-between gap-3 border-t pt-3">
        <span className="text-sm text-muted-foreground">
          {sessionCount} stored chat{sessionCount === 1 ? "" : "s"}
        </span>
        {confirmingClear ? (
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="destructive"
              size="sm"
              onClick={() => {
                clearSessions.mutate({});
                assistantSession?.forgetSession();
                setConfirmingClear(false);
              }}
            >
              Delete all stored chats
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => setConfirmingClear(false)}
            >
              Cancel
            </Button>
          </div>
        ) : (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="gap-2 text-muted-foreground hover:text-destructive"
            disabled={sessionCount === 0}
            onClick={() => setConfirmingClear(true)}
          >
            <Trash2 className="size-4" aria-hidden="true" />
            Clear stored chats
          </Button>
        )}
      </div>
    </div>
  );
}
