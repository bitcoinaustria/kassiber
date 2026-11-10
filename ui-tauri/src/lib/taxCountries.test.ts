import { describe, expect, it } from "vitest";

import deCommon from "@/i18n/locales/de/common.json";
import enCommon from "@/i18n/locales/en/common.json";

import {
  TAX_COUNTRIES,
  defaultGainsAlgorithm,
  defaultPoolScope,
  gainsAlgorithmsFor,
  isPreviewTaxCountry,
  isTaxCountry,
  requiredFiatCurrency,
  taxCountryEntry,
  taxCountryName,
  usesLongTermDays,
} from "./taxCountries";
import {
  TAX_COUNTRY_PREVIEW_NOTICE,
  TAX_COUNTRY_REGISTRY,
} from "./taxCountries.generated";

describe("tax country registry", () => {
  it("lists the registry's countries in its order", () => {
    expect(TAX_COUNTRIES).toEqual(TAX_COUNTRY_REGISTRY.map((entry) => entry.code));
    expect(TAX_COUNTRIES.slice(0, 2)).toEqual(["generic", "at"]);
  });

  it("keeps Austria's and the generic country's pickers", () => {
    expect(gainsAlgorithmsFor("at")).toEqual([
      "MOVING_AVERAGE_AT",
      "FIFO",
      "LIFO",
      "HIFO",
      "LOFO",
    ]);
    expect(gainsAlgorithmsFor("generic")).toEqual([
      "FIFO",
      "LIFO",
      "HIFO",
      "LOFO",
      "MOVING_AVERAGE",
    ]);
    expect(defaultGainsAlgorithm("at")).toBe("MOVING_AVERAGE_AT");
    expect(requiredFiatCurrency("at")).toBe("EUR");
    expect(requiredFiatCurrency("generic")).toBeNull();
    expect(usesLongTermDays("generic")).toBe(true);
    expect(usesLongTermDays("at")).toBe(false);
    expect(defaultPoolScope("generic")).toBe("global");
  });

  it("offers every allowed method and only those, default first", () => {
    for (const entry of TAX_COUNTRY_REGISTRY) {
      const picker = gainsAlgorithmsFor(entry.code);
      expect(picker[0]).toBe(entry.default_method);
      for (const method of picker) {
        expect(entry.methods).toContain(method);
      }
      expect(new Set(picker).size).toBe(picker.length);
    }
  });

  it("offers the United States as a preview USD country with wallet pools", () => {
    expect(TAX_COUNTRIES).toContain("us");
    expect(isPreviewTaxCountry("us")).toBe(true);
    expect(isPreviewTaxCountry("US")).toBe(true);
    expect(gainsAlgorithmsFor("us")).toEqual(["FIFO", "LIFO", "HIFO", "LOFO"]);
    expect(requiredFiatCurrency("us")).toBe("USD");
    expect(usesLongTermDays("us")).toBe(false);
    expect(defaultPoolScope("us")).toBe("wallet");
    expect(taxCountryName("us", "de")).toBe("Vereinigte Staaten");
  });

  it("offers Germany and Switzerland as preview countries from the registry", () => {
    expect(TAX_COUNTRIES).toEqual(expect.arrayContaining(["de", "ch"]));
    expect(isPreviewTaxCountry("de")).toBe(true);
    expect(gainsAlgorithmsFor("de")).toEqual(["FIFO", "MOVING_AVERAGE_FIFO_PERIOD"]);
    expect(requiredFiatCurrency("de")).toBe("EUR");
    expect(usesLongTermDays("de")).toBe(false);
    expect(defaultPoolScope("de")).toBe("wallet");
    expect(taxCountryName("de", "de")).toBe("Deutschland");
    expect(isPreviewTaxCountry("ch")).toBe(true);
    expect(gainsAlgorithmsFor("ch")).toEqual(["FIFO", "MOVING_AVERAGE"]);
    expect(requiredFiatCurrency("ch")).toBe("CHF");
    expect(defaultPoolScope("ch")).toBe("global");
    expect(taxCountryName("ch", "en")).toBe("Switzerland");
  });

  it("offers Spain with global FIFO and Japan with its two averages", () => {
    expect(isPreviewTaxCountry("es")).toBe(true);
    expect(gainsAlgorithmsFor("es")).toEqual(["FIFO"]);
    expect(requiredFiatCurrency("es")).toBe("EUR");
    expect(usesLongTermDays("es")).toBe(false);
    expect(defaultPoolScope("es")).toBe("global");
    expect(taxCountryName("es", "de")).toBe("Spanien");
    expect(taxCountryName("es", "en")).toBe("Spain");
    expect(isPreviewTaxCountry("jp")).toBe(true);
    expect(gainsAlgorithmsFor("jp")).toEqual(["JP_TOTAL_AVERAGE", "JP_MOVING_AVERAGE"]);
    expect(defaultGainsAlgorithm("jp")).toBe("JP_TOTAL_AVERAGE");
    expect(requiredFiatCurrency("jp")).toBe("JPY");
    expect(taxCountryName("jp", "en")).toBe("Japan");
  });

  it("reads stored codes the backend's way and refuses unknown ones", () => {
    expect(taxCountryEntry(" AT ")?.code).toBe("at");
    expect(isTaxCountry("xx")).toBe(false);
    expect(isPreviewTaxCountry("xx")).toBe(false);
    expect(taxCountryName("at", "de")).toBe("Österreich");
    expect(taxCountryName("at", "en")).toBe("Austria");
  });

  it("shows the preview notice the exported reports carry", () => {
    expect(enCommon.taxCountry.previewNotice).toBe(TAX_COUNTRY_PREVIEW_NOTICE.en);
    expect(deCommon.taxCountry.previewNotice).toBe(TAX_COUNTRY_PREVIEW_NOTICE.de);
    for (const entry of TAX_COUNTRY_REGISTRY) {
      expect(isPreviewTaxCountry(entry.code)).toBe(entry.preview);
    }
  });
});
