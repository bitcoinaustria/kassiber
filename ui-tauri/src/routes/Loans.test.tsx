import { describe, expect, it } from "vitest";

import { router } from "@/routeTree";
import enLoans from "@/i18n/locales/en/loans.json";
import deLoans from "@/i18n/locales/de/loans.json";

describe("Loans route", () => {
  it("registers the /loans route", () => {
    expect(router.routesByPath["/loans"]).toBeTruthy();
  });

  it("keeps en/de loan custody keys in lockstep", () => {
    expect(Object.keys(enLoans.custody).sort()).toEqual(
      Object.keys(deLoans.custody).sort(),
    );
  });
});
