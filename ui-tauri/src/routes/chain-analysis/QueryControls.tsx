import { useRef, useState, type Dispatch, type SetStateAction } from "react";
import { useTranslation } from "react-i18next";
import { Play } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DEFAULT_ANALYSIS_QUERY,
  analysisUtcInput,
  type AnalysisQuery,
} from "@/lib/chainAnalysis";

const FILTER_KEYS = [
  "observer",
  "direction",
  "chain",
  "network",
  "depth",
  "node_limit",
  "edge_limit",
  "min_amount_msat",
  "start",
  "end",
  "include_relations",
  "include_hypotheses",
] as const;

/** Number of collapsed controls that differ from the defaults, shown on the Filters summary. */
function activeFilterCount(query: AnalysisQuery): number {
  return FILTER_KEYS.filter(
    (key) => (query[key] || undefined) !== (DEFAULT_ANALYSIS_QUERY[key] || undefined),
  ).length;
}

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
  // A deep link or saved case can arrive in path mode without a target; start open so
  // the required field is visible. Afterwards the disclosure is plain user state.
  const [filtersOpen, setFiltersOpen] = useState(
    () => query.mode === "path" && !query.target?.trim(),
  );
  const details = useRef<HTMLDetailsElement>(null);
  const update = <K extends keyof AnalysisQuery>(
    key: K,
    value: AnalysisQuery[K],
  ) => setQuery((previous) => ({ ...previous, [key]: value }));
  // A hidden required input cannot receive focus for the browser's validation
  // message. `invalid` fires before the browser focuses the control, so open the
  // native element synchronously; the state write keeps the controlled prop in step.
  const reveal = () => {
    if (details.current) details.current.open = true;
    setFiltersOpen(true);
  };
  const filters = activeFilterCount(query);
  return (
    <form
      className="rounded-xl border bg-card p-3"
      onSubmit={(event) => {
        event.preventDefault();
        onRun();
      }}
    >
      <div className="ca-query">
        <input
          className="ca-input h-10 min-w-0 flex-1 basis-56 font-mono text-xs"
          placeholder={t("subjectPlaceholder")}
          aria-label={t("subject")}
          value={query.subject || ""}
          required={query.mode !== "overview"}
          onChange={(event) => {
            const subject = event.target.value;
            setQuery((previous) => ({
              ...previous,
              subject,
              mode:
                previous.mode === "overview" && subject.trim()
                  ? "trace"
                  : previous.mode === "trace" && !subject.trim()
                    ? "overview"
                    : previous.mode,
            }));
          }}
        />
        <Button className="h-10" type="submit" disabled={busy}>
          <Play className="size-3.5" />
          {busy ? t("running") : t("run")}
        </Button>
      </div>
      <details
        ref={details}
        className="mt-2"
        open={filtersOpen}
        onToggle={(event) => setFiltersOpen(event.currentTarget.open)}
      >
        <summary className="cursor-pointer text-xs font-medium text-muted-foreground">
          {t("filters")}
          {filters > 0 && (
            <span className="ml-1.5 font-mono text-[10px]">{filters}</span>
          )}
        </summary>
        <div className="mt-3 grid gap-3 sm:grid-cols-3 lg:grid-cols-6">
        {query.mode !== "overview" && (
          <label className="ca-field">
            {t("direction")}
          <select
            className="ca-select"
            aria-label={t("direction")}
            value={query.direction}
            onChange={(event) =>
              update("direction", event.target.value as AnalysisQuery["direction"])
            }
          >
            {(["both", "backward", "forward"] as const).map((item) => (
              <option key={item} value={item}>
                {t(item)}
              </option>
            ))}
          </select>
          </label>
        )}
        <label className="ca-field">
          {t("observer")}
        <select
          className="ca-select"
          aria-label={t("observer")}
          value={query.observer}
          onChange={(event) =>
            update("observer", event.target.value as AnalysisQuery["observer"])
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
          {query.mode === "path" && (
            <label className="ca-field sm:col-span-2 lg:col-span-3">
              {t("target")}
              <input
                className="ca-input font-mono"
                required
                value={query.target || ""}
                placeholder={t("targetPlaceholder")}
                onInvalid={reveal}
                onChange={(event) => update("target", event.target.value)}
              />
            </label>
          )}
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
                  onInvalid={reveal}
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
              onInvalid={reveal}
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
              onInvalid={reveal}
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
              onInvalid={reveal}
              onChange={(event) =>
                update("end", analysisUtcInput(event.target.value))
              }
            />
          </label>
        </div>
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
      </details>
    </form>
  );
}
