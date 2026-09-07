import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { draftForTransaction, type Transaction } from "@/components/transactions";
import type { Tx } from "@/mocks/seed";
import { toDashboardTransaction } from "./model";
import { transactionDetailSaveArgs, useTransactionDetailRecord } from "./useTransactionDetailRecord";

const daemon = vi.hoisted(() => vi.fn());
vi.mock("@/daemon/client", () => ({ useDaemon: daemon }));

const raw: Tx = {
  id: "incoming", date: "2026-04-15 08:00", type: "Transfer",
  account: "Self custody", counter: "Transfer BTC -> BTC", amountSat: 600_000,
  eur: null, rate: null, tag: "Transfer", conf: 3, feeSat: 0,
};
const incoming = toDashboardTransaction(raw, 0);
const aggregate = { ...incoming, amountBtc: 0, amount: 0 };

function readDetail(selected: Transaction | null) {
  let result: Transaction | null = null;
  function Probe() {
    result = useTransactionDetailRecord(selected).record;
    return null;
  }
  renderToStaticMarkup(<Probe />);
  return result as Transaction | null;
}

describe("exact transaction detail source", () => {
  beforeEach(() => daemon.mockReturnValue({}));

  it("resolves a clicked table row by its ID without depending on route updates", () => {
    daemon.mockReturnValue({ data: { data: { transaction: raw } } });
    expect(readDetail(aggregate)?.amountBtc).toBe(0.006);
    expect(daemon).toHaveBeenLastCalledWith("ui.transactions.resolve", { query: "incoming" }, { enabled: true });
    expect(aggregate.amountBtc).toBe(0);
    readDetail(null);
    expect(daemon).toHaveBeenLastCalledWith("ui.transactions.resolve", { query: "" }, { enabled: false });
  });

  it("does not expose the net table projection as an editable loading placeholder", () => {
    expect(readDetail(aggregate)).toBeNull();
  });

  it("does not trust the same-ID focused result from a shared-txid route", () => {
    expect(readDetail(aggregate)).toBeNull();
    daemon.mockReturnValue({ data: { data: { transaction: raw } } });
    expect(readDetail(aggregate)?.amountBtc).toBe(0.006);
  });

  it("never substitutes a previous selection or a same-txid peer", () => {
    daemon.mockReturnValue({ data: { data: { transaction: { ...raw, id: "fee" } } } });
    expect(readDetail(aggregate)).toBeNull();
  });

  it("saves price, tax, kind and tags for an exact detail outside the table page", () => {
    const detail = { ...incoming, tags: ["original"] };
    const baseline = draftForTransaction(detail);
    const changed = { ...baseline, kind: "buy", taxable: !baseline.taxable,
      manualPrice: "100000", manualCurrency: "EUR" };
    const args = transactionDetailSaveArgs("incoming", changed, [], detail, {});
    expect(args).toMatchObject({ transaction: "incoming", kind: "buy", taxable: changed.taxable, fiat_rate: "100000" });
    expect(args.tags).toContain("original");
    // A saved per-transaction draft remains the comparison baseline on later saves.
    const repeated = transactionDetailSaveArgs("incoming", changed, [], detail, { incoming: changed });
    expect(repeated).not.toHaveProperty("kind");
    expect(repeated).not.toHaveProperty("fiat_rate");
    expect(repeated).not.toHaveProperty("taxable");
  });
});
