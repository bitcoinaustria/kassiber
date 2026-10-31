import type { TFunction } from "i18next";

import { csvCell, type VisualFormat } from "./format";
import type { ChatVisual } from "./model";

/**
 * The table twin of every visual: the accessible equivalent, and the source
 * of "Copy CSV". Display strings follow the UI language; CSV keeps raw
 * numbers so a spreadsheet reads them as numbers.
 */

export interface VisualCell {
  display: string;
  raw: string | number | null;
  numeric?: boolean;
}

export interface VisualTable {
  columns: string[];
  rows: VisualCell[][];
}

type T = TFunction<"assistant">;

function textCell(value: string | null | undefined): VisualCell {
  return { display: value || "—", raw: value ?? null };
}

function numberCell(value: number | null, format: (value: number) => string, unknown: string): VisualCell {
  return { display: value === null ? unknown : format(value), raw: value, numeric: true };
}

export function visualTable(visual: ChatVisual, fmt: VisualFormat, t: T): VisualTable {
  const unknown = t("visual.unknownValue");
  const fiat = (value: number | null) => numberCell(value, fmt.fiat, unknown);
  switch (visual.source) {
    case "overview":
      return {
        columns: [t("visual.table.metric"), t("visual.table.value")],
        rows: [
          [textCell(t("visual.overview.portfolioValue")), fiat(visual.stats.portfolioValue)],
          [textCell(t("visual.measure.quantity")), numberCell(visual.stats.btcBalance, fmt.btc, unknown)],
          [textCell(t("visual.overview.costBasis")), fiat(visual.stats.costBasis)],
          [textCell(t("visual.overview.unrealized")), fiat(visual.stats.unrealized)],
          [textCell(t("visual.overview.realizedYtd")), fiat(visual.stats.realizedYtd)],
          [textCell(t("visual.overview.btcPrice")), fiat(visual.stats.btcPrice)],
          ...visual.wallets.map((wallet) => [
            textCell(wallet.label ?? t("visual.overview.otherWallets", { count: wallet.otherCount })),
            numberCell(wallet.balanceBtc, fmt.btc, unknown),
          ]),
        ],
      };
    case "balance_history":
      return {
        columns: [t("visual.table.period"), t("visual.table.asset"), t("visual.measure.quantity"), t("visual.measure.market_value"), t("visual.measure.cost_basis")],
        rows: visual.rows.map((row) => [
          { display: fmt.date(row.period, visual.interval === "month" ? "month" : "day"), raw: row.period },
          textCell(row.asset),
          numberCell(row.quantity, (value) => fmt.asset(value, row.asset), unknown),
          fiat(row.marketValue),
          fiat(row.costBasis),
        ]),
      };
    case "portfolio":
      return {
        columns: [t("visual.table.wallet"), t("visual.table.asset"), t("visual.measure.quantity"), t("visual.measure.market_value"), t("visual.measure.cost_basis"), t("visual.measure.unrealized_pnl")],
        rows: visual.rows.map((row) => [
          textCell(row.wallet),
          textCell(row.asset),
          numberCell(row.quantity, (value) => fmt.asset(value, row.asset), unknown),
          fiat(row.marketValue),
          fiat(row.costBasis),
          fiat(row.unrealized),
        ]),
      };
    case "tax_summary":
      return {
        columns: [t("visual.table.year"), t("visual.measure.proceeds"), t("visual.measure.cost_basis"), t("visual.measure.gain_loss")],
        rows: visual.years.map((year) => [
          { display: String(year.year), raw: year.year },
          fiat(year.proceeds),
          fiat(year.costBasis),
          fiat(year.gainLoss),
        ]),
      };
    case "cash_flow":
      return {
        columns: [t("visual.table.period"), t("visual.cashFlow.received"), t("visual.cashFlow.sent"), `${t("visual.cashFlow.received")} (${visual.fiatCurrency})`, `${t("visual.cashFlow.sent")} (${visual.fiatCurrency})`, t("visual.table.count")],
        rows: visual.rows.map((row) => [
          { display: fmt.date(row.period, visual.bucket), raw: row.period },
          numberCell(row.inBtc, fmt.btc, unknown),
          numberCell(row.outBtc, fmt.btc, unknown),
          fiat(row.inFiat),
          fiat(row.outFiat),
          numberCell(row.inCount + row.outCount, fmt.count, unknown),
        ]),
      };
    case "transactions":
      return {
        columns: [t("visual.table.date"), t("visual.table.description"), t("visual.table.wallet"), t("visual.table.amount"), t("visual.table.fiat"), t("visual.table.status")],
        rows: visual.rows.map((row) => [
          { display: row.date ? fmt.date(row.date, "short") : "—", raw: row.date },
          textCell(row.counterparty || row.note || row.type),
          textCell(row.wallet),
          numberCell(row.amountSat / 1e8, (value) => fmt.asset(value, row.asset), unknown),
          fiat(row.fiatValue),
          textCell(row.status),
        ]),
      };
    case "transaction_flow":
      return {
        columns: [t("visual.table.side"), t("visual.table.label"), t("visual.table.amount")],
        rows: visual.nodes.map((node) => [
          textCell(t(`visual.flow.side.${node.side}`)),
          textCell(node.labelKind === "wallet" ? node.label : node.labelKind === "fee" ? t("visual.flow.fee") : node.labelKind === "overflow" ? t("visual.flow.overflow", { count: node.overflowCount ?? 0 }) : t(node.side === "input" ? "visual.flow.input" : "visual.flow.output", { position: node.position + 1 })),
          numberCell(node.valueSats, fmt.sats, unknown),
        ]),
      };
    case "readiness":
      return {
        columns: [t("visual.table.check"), t("visual.table.status"), t("visual.table.count")],
        rows: visual.checks.map((check) => [
          textCell(check.check === "blocker" ? check.title ?? check.detail : t(`visual.readiness.check.${check.check}`)),
          textCell(t(`visual.readiness.status.${check.status}`)),
          numberCell(check.count, fmt.count, "—"),
        ]),
      };
  }
}

export function tableCsv(table: VisualTable): string {
  const header = table.columns.map(csvCell).join(",");
  const lines = table.rows.map((row) =>
    row
      .map((cell) =>
        cell.raw === null ? "" : typeof cell.raw === "number" ? String(cell.raw) : csvCell(cell.raw),
      )
      .join(","),
  );
  return [header, ...lines].join("\n");
}
