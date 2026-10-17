import { describe, expect, it } from "vitest";

import { detectCrossCheckTool } from "./crossCheckFile";

const encode = (text: string) => btoa(text);

describe("detectCrossCheckTool", () => {
  it("recognizes each tool by its header", () => {
    expect(
      detectCrossCheckTool(
        encode('"Type","Buy Amount","Buy Currency","Sell Amount","Sell Currency","Date"\n"Deposit","1","BTC"'),
      ),
    ).toBe("cointracking");
    expect(
      detectCrossCheckTool(
        encode(
          "ï»¿Date (UTC);Integration Name;Label;Outgoing Asset;Outgoing Amount;Incoming Asset;Incoming Amount\n",
        ),
      ),
    ).toBe("blockpit");
  });

  it("stays unsure about anything else", () => {
    expect(detectCrossCheckTool(encode("date,txid,amount\n"))).toBeNull();
    expect(detectCrossCheckTool("not base64!")).toBeNull();
    expect(detectCrossCheckTool(encode("x".repeat(20000)))).toBeNull();
  });
});
