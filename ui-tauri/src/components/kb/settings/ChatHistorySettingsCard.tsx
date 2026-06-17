import * as React from "react";
import { MessagesSquare, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

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

const MODE_DESCRIPTION_KEYS: Record<string, string> = {
  auto: "chatHistory.mode.auto",
  on: "chatHistory.mode.on",
  off: "chatHistory.mode.off",
};

const MODE_LABEL_KEYS: Record<string, string> = {
  auto: "chatHistory.modeLabel.auto",
  on: "chatHistory.modeLabel.on",
  off: "chatHistory.modeLabel.off",
};

export function ChatHistorySettingsCard() {
  const { t } = useTranslation("settings");
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
      ? t("chatHistory.effectiveEncrypted")
      : t("chatHistory.effectiveOnUnencrypted")
    : config.database_encrypted
      ? t("chatHistory.effectiveNotStoredEncrypted")
      : mode === "auto"
        ? t("chatHistory.effectiveNotStoredAuto")
        : t("chatHistory.effectiveNotStored");

  return (
    <div className="space-y-3 rounded-md border bg-background p-4">
      <div className="flex items-start gap-3">
        <MessagesSquare
          className="mt-0.5 size-4 shrink-0 text-muted-foreground"
          aria-hidden="true"
        />
        <div className="min-w-0 flex-1 space-y-1">
          <h3 className="text-sm font-semibold">{t("chatHistory.heading")}</h3>
          <p className="text-sm text-muted-foreground">
            {t("chatHistory.description")}
          </p>
        </div>
      </div>

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <Label htmlFor="settings-chat-history-mode" className="text-sm">
          {t("chatHistory.storeLabel")}
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
                <span className="font-medium">{t(MODE_LABEL_KEYS[value])}</span>
                <span className="text-muted-foreground">
                  {" — "}
                  {t(MODE_DESCRIPTION_KEYS[value])}
                </span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <p className="text-xs text-muted-foreground">{effectiveLine}</p>

      <div className="flex items-center justify-between gap-3 border-t pt-3">
        <span className="text-sm text-muted-foreground">
          {t("chatHistory.storedCount", { count: sessionCount })}
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
              {t("chatHistory.deleteAll")}
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={() => setConfirmingClear(false)}
            >
              {t("common:actions.cancel")}
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
            {t("chatHistory.clearStored")}
          </Button>
        )}
      </div>
    </div>
  );
}
