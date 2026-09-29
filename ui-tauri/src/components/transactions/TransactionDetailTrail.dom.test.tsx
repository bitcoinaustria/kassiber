// @vitest-environment happy-dom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { Transaction } from "./model";
import { useTransactionTrail } from "./TransactionDetailTrail";

const tx = (id: string) => ({ id, explorerId: id.repeat(64).slice(0, 64) }) as unknown as Transaction;

describe("transaction trail", () => {
  it("walks back along the coins followed and resets on an outside open", () => {
    const open = vi.fn();
    const { result, rerender } = renderHook(
      ({ transaction }: { transaction: Transaction }) => useTransactionTrail(transaction, true, open),
      { initialProps: { transaction: tx("a") } },
    );
    expect(result.current.back).toBeUndefined();

    act(() => result.current.follow?.("b"));
    expect(open).toHaveBeenLastCalledWith("b");
    rerender({ transaction: tx("b") });
    act(() => result.current.follow?.("c"));
    rerender({ transaction: tx("c") });
    expect(result.current.backLabel).toContain("bbbb");

    act(() => result.current.back?.());
    expect(open).toHaveBeenLastCalledWith("b");
    rerender({ transaction: tx("b") });
    expect(result.current.backLabel).toContain("aaaa");

    // Opening a row from the list (not via the sheet) starts over.
    rerender({ transaction: tx("z") });
    expect(result.current.back).toBeUndefined();
  });

  it("forgets the trail when the sheet closes", () => {
    const open = vi.fn();
    const { result, rerender } = renderHook(
      ({ transaction, shown }: { transaction: Transaction; shown: boolean }) =>
        useTransactionTrail(transaction, shown, open),
      { initialProps: { transaction: tx("a"), shown: true } },
    );
    act(() => result.current.follow?.("b"));
    rerender({ transaction: tx("b"), shown: true });
    expect(result.current.back).toBeDefined();
    rerender({ transaction: tx("b"), shown: false });
    rerender({ transaction: tx("b"), shown: true });
    expect(result.current.back).toBeUndefined();
  });
});
