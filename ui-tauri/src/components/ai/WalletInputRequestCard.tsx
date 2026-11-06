import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { useUiStore } from "@/store/ui";
import { useActiveBookScope } from "./useActiveBookScope";
import type { WalletInputRequest } from "./walletInputRequest";

const AddConnectionDialog = React.lazy(() =>
  import("@/components/kb/AddConnectionDialog").then((module) => ({
    default: module.AddConnectionDialog,
  })),
);

type CardState = "idle" | "open" | "connected";

/**
 * The assistant asked for a connection it must not set up itself (descriptor,
 * xpub, credentials). The user enters that material in the desktop's own setup
 * dialog, bound to the book the request was made in; nothing reaches the chat.
 */
export function WalletInputRequestCard({ request }: { request: WalletInputRequest }) {
  const { t } = useTranslation("assistant");
  const daemonSession = useUiStore((state) => state.daemonSession);
  const bookScope = useActiveBookScope();
  const [state, setState] = React.useState<CardState>("idle");
  const [session, setSession] = React.useState<number | null>(null);
  const sameBook =
    bookScope?.workspace_id === request.workspace_id &&
    bookScope?.profile_id === request.profile_id;
  return (
    <section aria-label={t("walletRequest.label")} className="my-3 border-l-2 border-border pl-4 text-sm">
      <p className="font-medium">{t(`walletRequest.source.${request.source}`)}</p>
      <p className="mt-1 text-muted-foreground">{request.explanation || t("walletRequest.why")}</p>
      <div className="mt-3 flex flex-wrap items-center gap-3">
        <Button
          size="sm"
          variant="outline"
          disabled={!sameBook || state !== "idle"}
          onClick={() => {
            setSession(daemonSession);
            setState("open");
          }}
        >
          {t("walletRequest.action")}
        </Button>
      </div>
      <p role="status" aria-live="polite" className="mt-2 text-xs text-muted-foreground">
        {!sameBook ? t("walletRequest.otherBook") : t(`walletRequest.status.${state}`)}
      </p>
      {state === "open" && session !== null ? (
        <React.Suspense fallback={null}>
          <AddConnectionDialog
            open
            initialSourceId={request.source}
            scopeBoundary={{
              expectedScope: { workspace_id: request.workspace_id, profile_id: request.profile_id },
              daemonSession: session,
              isCurrent: () => useUiStore.getState().daemonSession === session,
            }}
            onCompleted={() => setState("connected")}
            onOpenChange={(open) => {
              if (!open) setState((current) => (current === "open" ? "idle" : current));
            }}
          />
        </React.Suspense>
      ) : null}
    </section>
  );
}
