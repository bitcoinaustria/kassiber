import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { EvidenceDetails } from "./EvidenceDetails";
import { analysisNetworkInput } from "@/lib/chainAnalysis";

interface LocalLabel {
  id: string;
  subject: string;
  label: string;
  source: string;
  chain: string;
  network: string;
  category: string;
  confidence: string;
  revision: number;
}

export function LabelsPanel({
  initialSubject,
  initialChain,
  initialNetwork,
  onError,
}: {
  initialSubject: string;
  initialChain?: string;
  initialNetwork?: string;
  onError: (error: unknown) => void;
}) {
  const { t } = useTranslation("chainAnalysis");
  const labels = useDaemon<{ items: LocalLabel[] }>(
    "ui.chain_analysis.labels.list",
  );
  const save = useDaemonMutation("ui.chain_analysis.labels.upsert", {
    invalidateQueries: false,
  });
  const remove = useDaemonMutation("ui.chain_analysis.labels.delete", {
    invalidateQueries: false,
  });
  const [subject, setSubject] = useState(initialSubject);
  const [chain, setChain] = useState(initialChain || "bitcoin");
  const [network, setNetwork] = useState(analysisNetworkInput(initialNetwork));
  const [label, setLabel] = useState("");
  const [source, setSource] = useState("");
  const [category, setCategory] = useState("other");
  const [confidence, setConfidence] = useState("unverified");
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const busy = save.isPending || remove.isPending;
  return (
    <section className="space-y-3">
      <p className="text-xs text-muted-foreground">{t("labels.scope")}</p>
      <form
        className="space-y-3"
        onSubmit={async (event) => {
          event.preventDefault();
          if (busy) return;
          try {
            await save.mutateAsync({
              subject: subject.trim(),
              chain,
              network,
              label: label.trim(),
              source: source.trim(),
              category,
              confidence,
              cluster_defining: false,
            });
            setLabel("");
            setSource("");
            await labels.refetch();
          } catch (error) {
            onError(error);
          }
        }}
      >
        <fieldset disabled={busy} className="grid gap-3 sm:grid-cols-3">
          <label className="ca-field sm:col-span-2">
            {t("labels.subject")}
            <input
              className="ca-input font-mono"
              required
              value={subject}
              onChange={(event) => setSubject(event.target.value)}
            />
          </label>
          <div className="grid grid-cols-2 gap-2">
            <label className="ca-field">
              {t("chain")}
              <select
                className="ca-select"
                value={chain}
                onChange={(event) => setChain(event.target.value)}
              >
                <option value="bitcoin">Bitcoin</option>
                <option value="liquid">Liquid</option>
              </select>
            </label>
            <label className="ca-field">
              {t("network")}
              <select
                className="ca-select"
                value={network}
                onChange={(event) => setNetwork(event.target.value)}
              >
                {["main", "test", "signet", "regtest"].map((item) => (
                  <option key={item}>{item}</option>
                ))}
              </select>
            </label>
          </div>
          <label className="ca-field">
            {t("labels.label")}
            <input
              className="ca-input"
              required
              maxLength={200}
              value={label}
              onChange={(event) => setLabel(event.target.value)}
            />
          </label>
          <label className="ca-field sm:col-span-2">
            {t("labels.source")}
            <input
              className="ca-input"
              required
              maxLength={512}
              value={source}
              onChange={(event) => setSource(event.target.value)}
            />
          </label>
          <label className="ca-field">
            {t("labels.category")}
            <select
              className="ca-select"
              value={category}
              onChange={(event) => setCategory(event.target.value)}
            >
              {(
                [
                  "exchange",
                  "merchant",
                  "mixer",
                  "service",
                  "self",
                  "other",
                ] as const
              ).map((item) => (
                <option key={item} value={item}>
                  {t(`labels.${item}`)}
                </option>
              ))}
            </select>
          </label>
          <label className="ca-field">
            {t("labels.confidence")}
            <select
              className="ca-select"
              value={confidence}
              onChange={(event) => setConfidence(event.target.value)}
            >
              {(["user_confirmed", "imported", "unverified"] as const).map(
                (item) => (
                  <option key={item} value={item}>
                    {t(`labels.${item}`)}
                  </option>
                ),
              )}
            </select>
          </label>
          <Button
            className="self-end"
            variant="outline"
            size="sm"
            type="submit"
            disabled={
              busy || !subject.trim() || !label.trim() || !source.trim()
            }
          >
            {t("labels.save")}
          </Button>
        </fieldset>
      </form>
      {labels.isError && (
        <p className="text-sm text-destructive" role="alert">
          {labels.error.message}
        </p>
      )}
      {!labels.data?.data?.items.length && !labels.isFetching && (
        <p className="text-xs text-muted-foreground">{t("labels.empty")}</p>
      )}
      <div className="max-h-80 space-y-2 overflow-auto">
        {labels.data?.data?.items.map((item) => (
          <article key={item.id} className="rounded-md border p-3">
            <div className="flex items-start justify-between gap-3">
              <div>
                <p className="text-sm font-medium">{item.label}</p>
                <p className="mt-1 select-text break-all font-mono text-xs text-muted-foreground">
                  {item.subject}
                </p>
              </div>
              <Button
                size="sm"
                variant={confirmDelete === item.id ? "destructive" : "ghost"}
                disabled={busy}
                onClick={async () => {
                  if (confirmDelete !== item.id) {
                    setConfirmDelete(item.id);
                    return;
                  }
                  try {
                    await remove.mutateAsync({
                      id: item.id,
                      expected_revision: item.revision,
                    });
                    setConfirmDelete(null);
                    await labels.refetch();
                  } catch (error) {
                    onError(error);
                  }
                }}
              >
                {confirmDelete === item.id
                  ? t("labels.deleteConfirm")
                  : t("labels.delete")}
              </Button>
            </div>
            <EvidenceDetails value={item} />
          </article>
        ))}
      </div>
    </section>
  );
}
