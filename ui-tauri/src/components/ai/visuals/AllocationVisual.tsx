import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import type { AllocationVisual as AllocationVisualModel, HoldingRow } from "./model";
import type { VisualFormat } from "./format";
import { seriesColorMap, type VisualPalette } from "./palette";
import { stagger } from "./motion";
import { CountUp, GrowBar, Legend, Segmented } from "./primitives";

type Measure = "market_value" | "quantity" | "cost_basis" | "unrealized_pnl";
type Group = "wallet" | "asset";

const FIELD: Record<Measure, keyof Pick<HoldingRow, "marketValue" | "quantity" | "costBasis" | "unrealized">> = {
  market_value: "marketValue",
  quantity: "quantity",
  cost_basis: "costBasis",
  unrealized_pnl: "unrealized",
};

interface Entity {
  key: string;
  label: string;
  sublabel: string;
  asset: string | null;
  value: number | null;
}

/**
 * Group holdings. Fiat measures add up across assets; an amount does not
 * (a BTC and an L-BTC amount are different assets), so by wallet each asset
 * keeps its own row. One unknown part makes the whole entry unknown.
 */
function groupRows(rows: HoldingRow[], group: Group, measure: Measure): Entity[] {
  const field = FIELD[measure];
  const byKey = new Map<string, Entity & { wallets: Set<string>; assets: Set<string> }>();
  for (const row of rows) {
    const splitByAsset = measure === "quantity" || group === "asset";
    const key = group === "asset" ? row.asset : splitByAsset ? `${row.wallet}\u0000${row.asset}` : row.wallet;
    const current = byKey.get(key) ?? {
      key,
      label: group === "asset" ? row.asset : row.wallet,
      sublabel: "",
      asset: splitByAsset ? row.asset : null,
      value: 0,
      wallets: new Set<string>(),
      assets: new Set<string>(),
    };
    const part = row[field];
    current.value = current.value === null || part === null ? null : current.value + part;
    current.wallets.add(row.wallet);
    current.assets.add(row.asset);
    byKey.set(key, current);
  }
  return [...byKey.values()]
    .map(({ wallets, assets, ...entity }) => ({
      ...entity,
      sublabel: group === "asset" ? [...wallets].join(", ") : [...assets].join(", "),
    }))
    .sort((a, b) => Math.abs(b.value ?? 0) - Math.abs(a.value ?? 0) || a.label.localeCompare(b.label));
}

interface Props {
  visual: AllocationVisualModel;
  fmt: VisualFormat;
  palette: VisualPalette;
  hideSensitive: boolean;
}

export function AllocationVisual({ visual, fmt, palette, hideSensitive }: Props) {
  const { t } = useTranslation("assistant");
  const [measure, setMeasure] = useState<Measure>((visual.measure as Measure) ?? "market_value");
  const [group, setGroup] = useState<Group>(visual.groupBy);
  const entities = useMemo(() => groupRows(visual.rows, group, measure), [visual.rows, group, measure]);
  const signed = measure === "unrealized_pnl";
  const fiat = measure !== "quantity";
  const known = entities.filter((entity) => entity.value !== null);
  const total = known.reduce((sum, entity) => sum + (entity.value ?? 0), 0);
  const positiveTotal = known.reduce((sum, entity) => sum + Math.max(entity.value ?? 0, 0), 0);
  const maxAbs = Math.max(1e-12, ...known.map((entity) => Math.abs(entity.value ?? 0)));
  const assets = [...new Set(visual.rows.map((row) => row.asset))];
  const assetColors = seriesColorMap(assets, palette);
  // Wallets have no fixed identity colour; within one visual each keeps the
  // slot of its first appearance, and the tail folds to grey.
  const entityColor = (entity: Entity, index: number) =>
    group === "asset" || entity.asset
      ? assetColors.get(entity.asset ?? entity.label) ?? palette.muted
      : (index < palette.series.length ? palette.series[index] : palette.muted);
  const format = (value: number, asset: string | null) =>
    fiat ? fmt.fiat(value) : fmt.asset(value, asset ?? "BTC");
  const mixedAmount = !fiat && new Set(known.map((entity) => entity.asset)).size > 1;

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0">
          <div className="text-2xs text-muted-foreground">
            {t(`visual.allocation.total.${measure}`)}
            {known.length < entities.length ? ` · ${t("visual.allocation.pricedOnly")}` : ""}
          </div>
          {mixedAmount ? (
            <div className="text-sm text-muted-foreground">{t("visual.allocation.mixedAssets")}</div>
          ) : (
            <CountUp
              value={known.length ? total : null}
              format={(value) => (signed ? fmt.signed(format(Math.abs(value), known[0]?.asset ?? null), value) : format(value, known[0]?.asset ?? null))}
              sensitive={hideSensitive}
              className="text-2xl font-semibold tracking-tight"
            />
          )}
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Segmented
            label={t("visual.controls.groupBy")}
            value={group}
            options={[
              { value: "wallet" as const, label: t("visual.allocation.byWallet") },
              { value: "asset" as const, label: t("visual.allocation.byAsset") },
            ]}
            onChange={setGroup}
          />
          <Segmented
            label={t("visual.controls.measure")}
            value={measure}
            options={(["market_value", "quantity", "cost_basis", "unrealized_pnl"] as const).map((value) => ({
              value,
              label: t(`visual.measure.${value}`),
            }))}
            onChange={setMeasure}
          />
        </div>
      </div>
      {!signed && positiveTotal > 0 ? (
        <div className="flex h-3 w-full gap-[2px] overflow-hidden rounded-full" role="img" aria-label={t("visual.allocation.share")}>
          {known.map((entity, index) =>
            (entity.value ?? 0) > 0 ? (
              <span
                key={entity.key}
                className="kb-visual-grow-x h-full origin-left first:rounded-l-full last:rounded-r-full"
                title={entity.label}
                style={{
                  width: `${((entity.value ?? 0) / positiveTotal) * 100}%`,
                  backgroundColor: entityColor(entity, entities.indexOf(entity)),
                  animationDelay: `${stagger(index, 60)}ms`,
                }}
              />
            ) : null,
          )}
        </div>
      ) : null}
      <ul className="space-y-1.5">
        {entities.map((entity, index) => {
          const value = entity.value;
          const fraction = value === null ? 0 : Math.abs(value) / maxAbs;
          const color = signed
            ? (value ?? 0) < 0 ? palette.negative : palette.positive
            : entityColor(entity, index);
          return (
            <li
              key={entity.key}
              className="kb-visual-rise grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1"
              style={{ animationDelay: `${stagger(index)}ms` }}
            >
              <div className="flex min-w-0 items-center gap-2 text-xs">
                <span className="size-2 shrink-0 rounded-xs" style={{ backgroundColor: color }} aria-hidden="true" />
                <span className="truncate font-medium">{entity.label}</span>
                {entity.asset && group === "wallet" ? (
                  <span className="shrink-0 text-2xs text-muted-foreground">{entity.asset}</span>
                ) : entity.sublabel ? (
                  <span className="truncate text-2xs text-muted-foreground">{entity.sublabel}</span>
                ) : null}
              </div>
              <div className="flex items-baseline gap-2 text-right text-xs">
                <span className={cn("font-medium tabular-nums", hideSensitive && "sensitive")}>
                  {value === null
                    ? t("visual.unknownValue")
                    : signed
                      ? fmt.signed(format(Math.abs(value), entity.asset), value)
                      : format(value, entity.asset)}
                </span>
                {!signed && positiveTotal > 0 && value !== null && value > 0 && !mixedAmount ? (
                  <span className="w-11 text-2xs tabular-nums text-muted-foreground">{fmt.percent(value / positiveTotal)}</span>
                ) : null}
              </div>
              <div className="col-span-2 h-1.5 w-full rounded-full bg-muted/50">
                {value !== null ? (
                  <GrowBar fraction={fraction} color={color} delayMs={stagger(index) + 80} className="opacity-80" />
                ) : null}
              </div>
            </li>
          );
        })}
      </ul>
      {group === "wallet" && !entities.some((entity) => entity.asset) ? null : (
        <Legend shape="square" items={assets.map((asset) => ({ key: asset, label: asset, color: assetColors.get(asset) ?? palette.muted }))} />
      )}
    </div>
  );
}
