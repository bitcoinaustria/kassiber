import { useContext } from "react";
import { useTranslation } from "react-i18next";

import { AssistantSessionContext } from "./assistantSession";
import { assistantScreenContextFor } from "./assistantScreenContext";

export interface AskAboutTarget {
  /** A held (quarantined) transaction gets the review tools; others the transaction tools. */
  kind: "transaction" | "quarantine";
  transactionId: string;
  /** Shown on the local composer chip only; never sent. */
  date?: string | null;
}

/**
 * "Ask about this": pin a transaction as typed screen context and open the
 * panel with a prefilled question. The id travels only as the typed entity
 * (run through the same allowlist as route context); the question text never
 * contains it. Returns null where no assistant is mounted (AI turned off).
 */
export function useAskAssistantAbout(): ((target: AskAboutTarget) => void) | null {
  const assistant = useContext(AssistantSessionContext);
  const { t } = useTranslation("assistant");
  const askAbout = assistant?.askAbout;
  if (!askAbout) return null;
  return ({ kind, transactionId, date }) => {
    const context = assistantScreenContextFor(
      kind === "quarantine" ? "/quarantine" : "/transactions",
      `?tx=${encodeURIComponent(transactionId)}`,
    );
    if (context.entityType !== "transaction" || !context.entityId) return;
    const day = date?.slice(0, 10) || "—";
    askAbout(
      {
        context,
        label: t(
          kind === "quarantine" ? "panel.focusQuarantine" : "panel.focusTransaction",
          { date: day },
        ),
      },
      t(
        kind === "quarantine"
          ? "panel.askAboutQuarantinePrompt"
          : "panel.askAboutTransactionPrompt",
      ),
    );
  };
}
