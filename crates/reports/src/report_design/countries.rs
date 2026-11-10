//! Country layers: what a country adds to the shared report design
//! (`docs/reference/report-design.md#country-layer`).
//!
//! A layer has two halves. `collect` reads the book when a report's model
//! is built and returns the layer's data, exact and unformatted; `sections`
//! turns that data into sections the report places after Income, and
//! `method` adds facts to the Method section. Each country registers its
//! layer here under its registry code and the generator name its registry
//! entry lists, so a new country adds its own layer without touching
//! another's. Austria's layer predates the hook and stays inline in the
//! builders (its E 1kv and exit tax are reports of their own).

use super::*;
use kassiber_store::rusqlite::Connection;

/// One country's layer.
pub struct CountryLayer {
    /// The registry code.
    pub country: &'static str,
    /// The registry's `report_generators` entry the layer provides.
    pub generator: &'static str,
    /// The layer's part of a report model; `wallet` limits it to one wallet.
    pub collect: fn(&Connection, &Value, Option<&str>) -> crate::Result<Value>,
    /// Its sections, after Income.
    pub sections: fn(&Value, Lang) -> Vec<Section>,
    /// Its facts for the Method section, from its data.
    pub method: fn(&Value, Lang) -> Vec<Fact>,
}

/// Every registered layer.
pub const LAYERS: &[CountryLayer] = &[
    CountryLayer {
        country: "us",
        generator: "us.form_8949",
        collect: crate::report_design::us::collect,
        sections: crate::report_design::us::sections,
        method: crate::report_design::us::method,
    },
    CountryLayer {
        country: "de",
        generator: "de.anlage_so",
        collect: crate::report_design::de::collect,
        sections: crate::report_design::de::sections,
        method: crate::report_design::de::method,
    },
    CountryLayer {
        country: "ch",
        generator: "ch.wealth_income",
        collect: crate::report_design::ch::collect,
        sections: crate::report_design::ch::sections,
        method: crate::report_design::ch::method,
    },
    CountryLayer {
        country: "ie",
        generator: "ie.cgt",
        collect: crate::report_design::ie::collect,
        sections: crate::report_design::ie::sections,
        method: crate::report_design::ie::method,
    },
    CountryLayer {
        country: "gb",
        generator: "gb.sa108",
        collect: crate::report_design::gb::collect,
        sections: crate::report_design::gb::sections,
        method: crate::report_design::gb::method,
    },
    CountryLayer {
        country: "es",
        generator: "es.irpf_monedas_virtuales",
        collect: crate::report_design::es::collect,
        sections: crate::report_design::es::sections,
        method: crate::report_design::es::method,
    },
    CountryLayer {
        country: "jp",
        generator: "jp.crypto_keisansho",
        collect: crate::report_design::jp::collect,
        sections: crate::report_design::jp::sections,
        method: crate::report_design::jp::method,
    },
];

/// The layer of the stored `tax_country` (compared exactly, as the
/// builders compare it).
pub fn layer(country: &str) -> Option<&'static CountryLayer> {
    LAYERS.iter().find(|layer| layer.country == country)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_tax_countries::{Rules, COUNTRIES};

    #[test]
    fn every_country_generator_has_its_layer() {
        for country in COUNTRIES {
            for generator in country.report_generators {
                let Some((code, _)) = generator.split_once('.') else {
                    continue;
                };
                assert_eq!(code, country.code, "{generator}");
                if country.rules == Rules::Austria {
                    // The E 1kv and the exit tax are reports of their own.
                    continue;
                }
                let layer =
                    layer(country.code).unwrap_or_else(|| panic!("{generator} has no layer"));
                assert_eq!(layer.generator, *generator);
            }
        }
        for layer in LAYERS {
            let entry = kassiber_tax_countries::lookup(layer.country).expect("a registry country");
            assert!(entry.report_generators.contains(&layer.generator));
        }
    }
}
