import type { Dispatch, SetStateAction } from "react";
import { useTranslation } from "react-i18next";
import { Play } from "lucide-react";
import { Button } from "@/components/ui/button";
import { analysisUtcInput, type AnalysisQuery } from "@/lib/chainAnalysis";

export function QueryControls({
  query,
  setQuery,
  busy,
  onRun,
}: {
  query: AnalysisQuery;
  setQuery: Dispatch<SetStateAction<AnalysisQuery>>;
  busy: boolean;
  onRun: () => void;
}) {
  const { t } = useTranslation("chainAnalysis");
  const update = <K extends keyof AnalysisQuery>(
    key: K,
    value: AnalysisQuery[K],
  ) => setQuery((previous) => ({ ...previous, [key]: value }));
  return (
    <form
      className="rounded-xl border bg-card p-3 sm:p-4"
      onSubmit={(event) => {
        event.preventDefault();
        onRun();
      }}
    >
      <div className="ca-query">
        <label className="ca-field">
          {t("subject")}
          <input
            className="ca-input h-11 font-mono text-xs"
            placeholder={t("subjectPlaceholder")}
            aria-label={t("subject")}
            value={query.subject || ""}
            required={query.mode !== "overview"}
            onChange={(event) =>
              setQuery((previous) => ({
                ...previous,
                subject: event.target.value,
                mode:
                  previous.mode === "overview" && event.target.value
                    ? "trace"
                    : previous.mode,
              }))
            }
          />
        </label>
        <Button className="h-11 self-end" type="submit" disabled={busy}>
          <Play className="size-3.5" />
          {busy ? t("running") : t("run")}
        </Button>
      </div>
      <div className="mt-3 flex flex-wrap items-end gap-3">
        <label className="ca-field">
          {t("mode")}
          <select
            className="ca-select"
            value={query.mode}
            onChange={(event) =>
              update("mode", event.target.value as AnalysisQuery["mode"])
            }
          >
            {(["overview", "trace", "path"] as const).map((item) => (
              <option key={item} value={item}>
                {t(item)}
              </option>
            ))}
          </select>
        </label>
        <label className="ca-field">
          {t("direction")}
          <select
            className="ca-select"
            value={query.direction}
            onChange={(event) =>
              update(
                "direction",
                event.target.value as AnalysisQuery["direction"],
              )
            }
          >
            {(["both", "backward", "forward"] as const).map((item) => (
              <option key={item} value={item}>
                {t(item)}
              </option>
            ))}
          </select>
        </label>
        <label className="ca-field">
          {t("observer")}
          <select
            className="ca-select"
            value={query.observer}
            onChange={(event) =>
              update(
                "observer",
                event.target.value as AnalysisQuery["observer"],
              )
            }
          >
            {(["owner", "public", "disclosed"] as const).map((item) => (
              <option key={item} value={item}>
                {t(item)}
              </option>
            ))}
          </select>
        </label>
        <label className="ca-field">
          {t("chain")}
          <select
            className="ca-select"
            value={query.chain || ""}
            onChange={(event) =>
              update(
                "chain",
                (event.target.value as AnalysisQuery["chain"]) || undefined,
              )
            }
          >
            <option value="">{t("all")}</option>
            <option value="bitcoin">Bitcoin</option>
            <option value="liquid">Liquid</option>
          </select>
        </label>
        <label className="ca-field">
          {t("network")}
          <select
            className="ca-select"
            value={query.network || ""}
            onChange={(event) =>
              update("network", event.target.value || undefined)
            }
          >
            <option value="">{t("all")}</option>
            {["main", "test", "signet", "regtest"].map((item) => (
              <option key={item}>{item}</option>
            ))}
          </select>
        </label>
        {query.mode === "path" && (
          <label className="ca-field min-w-64 flex-1">
            {t("target")}
            <input
              className="ca-input font-mono"
              required
              value={query.target || ""}
              placeholder={t("targetPlaceholder")}
              onChange={(event) => update("target", event.target.value)}
            />
          </label>
        )}
      </div>
      <details className="mt-3 border-t pt-3">
        <summary className="cursor-pointer text-xs font-medium">
          {t("advanced")}
        </summary>
        <div className="mt-3 grid gap-3 sm:grid-cols-3 lg:grid-cols-6">
          {(["depth", "node_limit", "edge_limit"] as const).map(
            (key, index) => (
              <label className="ca-field" key={key}>
                {t((["depth", "nodeLimit", "edgeLimit"] as const)[index])}
                <input
                  className="ca-input"
                  type="number"
                  required
                  min={[1, 25, 50][index]}
                  max={[50, 2000, 6000][index]}
                  value={query[key]}
                  onChange={(event) => update(key, Number(event.target.value))}
                />
              </label>
            ),
          )}
          <label className="ca-field">
            {t("minAmount")}
            <input
              className="ca-input font-mono"
              inputMode="numeric"
              pattern="[0-9]*"
              value={query.min_amount_msat || ""}
              onChange={(event) =>
                update("min_amount_msat", event.target.value || undefined)
              }
            />
          </label>
          <label className="ca-field">
            {t("start")}
            <input
              className="ca-input"
              type="datetime-local"
              value={query.start?.replace(/Z$/, "") || ""}
              onChange={(event) =>
                update("start", analysisUtcInput(event.target.value))
              }
            />
          </label>
          <label className="ca-field">
            {t("end")}
            <input
              className="ca-input"
              type="datetime-local"
              value={query.end?.replace(/Z$/, "") || ""}
              onChange={(event) =>
                update("end", analysisUtcInput(event.target.value))
              }
            />
          </label>
        </div>
      </details>
      <div className="mt-3 flex flex-wrap gap-x-5 gap-y-2 text-xs">
        <label className="flex cursor-pointer items-center gap-2">
          <input
            type="checkbox"
            checked={query.include_relations}
            onChange={(event) =>
              update("include_relations", event.target.checked)
            }
          />
          {t("relations")}
        </label>
        <label className="flex cursor-pointer items-center gap-2">
          <input
            type="checkbox"
            checked={query.include_hypotheses}
            onChange={(event) =>
              update("include_hypotheses", event.target.checked)
            }
          />
          {t("hypotheses")}
        </label>
      </div>
      <p className="mt-3 max-w-4xl text-[11px] leading-relaxed text-muted-foreground">
        {t("observerHelp")}
      </p>
    </form>
  );
}
