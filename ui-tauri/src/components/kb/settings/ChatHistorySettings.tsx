import * as React from "react";
import { Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";

import { AssistantSessionContext } from "@/components/ai/assistantSession";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useDaemon, useDaemonMutation } from "@/daemon/client";

import { SettingsRow } from "./SettingsControls";

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

/**
 * Settings → Assistant: whether chats are kept, and the ones that are. Rows of
 * the Chats group.
 */
export function ChatHistorySettings() {
  const { t } = useTranslation(["settings", "common"]);
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
    <>
      <SettingsRow
        label={t("chatHistory.storeLabel")}
        description={effectiveLine}
        htmlFor="settings-chat-history-mode"
      >
        <Select
          value={mode}
          onValueChange={(value) => configure.mutate({ history: value })}
        >
          <SelectTrigger id="settings-chat-history-mode" className="w-44">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {(["auto", "on", "off"] as const).map((value) => (
              <SelectItem
                key={value}
                value={value}
                // dynamic key
                title={t(MODE_DESCRIPTION_KEYS[value] as never)}
              >
                {/* dynamic key */}
                {t(MODE_LABEL_KEYS[value] as never)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </SettingsRow>
      <SettingsRow
        label={t("chatHistory.storedCount", { count: sessionCount })}
        description={t("chatHistory.description")}
      >
        {confirmingClear ? (
          <>
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
          </>
        ) : (
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="gap-2"
            disabled={sessionCount === 0}
            onClick={() => setConfirmingClear(true)}
          >
            <Trash2 className="size-4" aria-hidden="true" />
            {t("chatHistory.clearStored")}
          </Button>
        )}
      </SettingsRow>
    </>
  );
}
