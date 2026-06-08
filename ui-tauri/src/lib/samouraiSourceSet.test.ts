import { describe, expect, it } from "vitest";

import { buildSamouraiSourceSet } from "./samouraiSourceSet";

const emptyFields = {
  deposit: "",
  badbank: "",
  premix: "",
  postmix: "",
};

describe("buildSamouraiSourceSet", () => {
  it("turns the four Samourai fields into section-labeled xpub sources", () => {
    const { sourceSet, errors } = buildSamouraiSourceSet({
      deposit: "zpub6deposit",
      badbank: "xpub6badbank",
      premix: "xpub6premix",
      postmix: "xpub6postmix",
    });

    expect(errors).toEqual({});
    expect(sourceSet.xpubs).toEqual([
      {
        section: "deposit",
        script_type: "p2wpkh",
        root_path: "m/84'/0'/0'",
        xpub: "zpub6deposit",
      },
      {
        section: "badbank",
        script_type: "p2wpkh",
        root_path: "m/84'/0'/2147483644'",
        xpub: "xpub6badbank",
      },
      {
        section: "premix",
        script_type: "p2wpkh",
        root_path: "m/84'/0'/2147483645'",
        xpub: "xpub6premix",
      },
      {
        section: "postmix",
        script_type: "p2wpkh",
        root_path: "m/84'/0'/2147483646'",
        xpub: "xpub6postmix",
      },
    ]);
    expect(sourceSet.children).toEqual([]);
  });

  it("accepts receive/change descriptor text", () => {
    const { sourceSet, errors } = buildSamouraiSourceSet({
      ...emptyFields,
      postmix: [
        "descriptor=wpkh([00000000/84'/0'/2147483646']xpub/0/*)",
        "change_descriptor=wpkh([00000000/84'/0'/2147483646']xpub/1/*)",
      ].join("\n"),
    });

    expect(errors).toEqual({});
    expect(sourceSet.children).toEqual([
      {
        section: "postmix",
        script_type: "p2wpkh",
        root_path: "m/84'/0'/2147483646'",
        descriptor: "wpkh([00000000/84'/0'/2147483646']xpub/0/*)",
        change_descriptor: "wpkh([00000000/84'/0'/2147483646']xpub/1/*)",
      },
    ]);
  });

  it("reports unknown populated fields", () => {
    const { sourceSet, errors } = buildSamouraiSourceSet({
      ...emptyFields,
      premix: "not a descriptor",
    });

    expect(sourceSet.children).toEqual([]);
    expect(sourceSet.xpubs).toEqual([]);
    expect(errors.premix).toContain("Paste an output descriptor");
  });

  it("does not guess the Deposit script type from a bare xpub", () => {
    const { sourceSet, errors } = buildSamouraiSourceSet({
      ...emptyFields,
      deposit: "xpub6deposit",
    });

    expect(sourceSet.children).toEqual([]);
    expect(sourceSet.xpubs).toEqual([]);
    expect(errors.deposit).toContain("Bare Deposit xpub is ambiguous");
  });
});
