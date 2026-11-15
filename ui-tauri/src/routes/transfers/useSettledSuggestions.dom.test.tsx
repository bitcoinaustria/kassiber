// @vitest-environment happy-dom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { TransferCandidate } from "@/components/review/transferModel";

import { useSettledSuggestions } from "./useSettledSuggestions";

const candidate = (id: string) => ({ out_id: id, in_id: `${id}-in` }) as unknown as TransferCandidate;

describe("useSettledSuggestions", () => {
  it("keeps the list read before a recalculation until the run is done and read again", async () => {
    let resolve: () => void = () => {};
    const refetch = vi.fn(() => new Promise<void>((done) => (resolve = done)));
    const { result, rerender } = renderHook(
      ({ live, updating }: { live: TransferCandidate[] | undefined; updating: boolean }) =>
        useSettledSuggestions(live, updating, refetch),
      { initialProps: { live: [] as TransferCandidate[] | undefined, updating: false } },
    );
    expect(result.current).toEqual([]);
    rerender({ live: [], updating: true });
    // Mid-run the matcher reads stale journals and suggests linked moves.
    rerender({ live: [candidate("a"), candidate("b")], updating: true });
    expect(result.current).toEqual([]);
    rerender({ live: [candidate("a"), candidate("b")], updating: false });
    expect(refetch).toHaveBeenCalledTimes(1);
    expect(result.current).toEqual([]);
    await act(async () => resolve());
    rerender({ live: [], updating: false });
    expect(result.current).toEqual([]);
  });

  it("shows the live list when nothing recalculates", () => {
    const { result } = renderHook(() => useSettledSuggestions([candidate("a")], false, vi.fn()));
    expect(result.current).toEqual([candidate("a")]);
  });
});
