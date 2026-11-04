//! Journal descriptions (design §3.6), composed with today's grammar.
//!
//! The adapter's `_compose_event_notes`, `_compose_transfer_notes`, and
//! `_compose_transfer_journal_description` (`tests/tax_oracle/adapter.py:311-486`),
//! ported literally. An event's notes and a move's notes are the `notes` of
//! the entries the walk builds, which the lot engine parses for its markers
//! (`austria.rs`, `pool.rs`, `swaps.rs`), and they become the persisted
//! descriptions of the journal entries composed from those entries. The
//! transfer legs' description is the persisted description of a booked
//! move's `transfer_out` and `transfer_in` entries.
//!
//! Each composer writes its tokens in a fixed order, joined with one space,
//! and leaves out every token whose value is empty: the lot engine treats an
//! absent marker and an empty one differently, so a bare `key=` is never
//! written. The text a row carries is written last and verbatim, so marker
//! tokens a user typed into it keep the effect they have today.

use crate::austria::DEFAULT_POOL;
use crate::manifest::{
    MARKER_ALT_IN, MARKER_ALT_OUT, MARKER_POOL, MARKER_REGIME, MARKER_REGIME_BASIS,
    MARKER_SWAP_LINK,
};

use super::normalize::GLOBAL_POOL;
use super::response::RegimeFlows;

/// The marker values and text of an event's notes.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EventNotes<'a> {
    pub regime: Option<&'a str>,
    /// Why a disposal has its regime (`wahlrecht`); the lot engine ignores it.
    pub regime_basis: Option<&'a str>,
    /// The cost-basis pool, written on Austrian books only.
    pub pool: Option<&'a str>,
    pub swap_link: Option<&'a str>,
    /// The row's text, else its id.
    pub description: &'a str,
}

/// The marker values and text of a move's notes, its `transfer_fee`
/// description.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TransferNotes<'a> {
    /// The regime of the move's fee.
    pub regime: Option<&'a str>,
    /// The source's cost-basis pool, written on Austrian books only.
    pub pool: Option<&'a str>,
    /// How the move was proven.
    pub pairing_source: Option<&'a str>,
    /// The out leg's text, else `Transfer <from> -> <to>`.
    pub description: &'a str,
}

/// What a booked move's transfer legs are described by: its audit row.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TransferLeg<'a> {
    pub regime: Option<&'a str>,
    /// The msat each regime moved.
    pub flows: Option<&'a RegimeFlows>,
    /// The pool the audit row records (Austrian books only).
    pub pool: Option<&'a str>,
    /// The wallets' labels; a missing one is written `None`.
    pub from_label: Option<&'a str>,
    pub to_label: Option<&'a str>,
    pub pairing_source: Option<&'a str>,
}

/// Whether CPython's `str.isspace()` holds for `c`, the set `str.strip()`
/// removes: Unicode's white space plus the ASCII separators U+001C-U+001F.
fn is_python_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// `_austrian_wire_pool_id`: the pool as Austrian markers write it, with
/// the global pool written `default`, or `None` for an empty one.
pub(crate) fn wire_pool(pool_id: Option<&str>) -> Option<String> {
    let pool = pool_id.unwrap_or("").trim_matches(is_python_space);
    match pool {
        "" => None,
        GLOBAL_POOL => Some(DEFAULT_POOL.to_owned()),
        other => Some(other.to_owned()),
    }
}

/// `marker_token`.
fn token(key: &str, value: impl std::fmt::Display) -> String {
    format!("{key}={value}")
}

/// A non-empty value.
fn present(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

/// `_compose_event_notes`: `at_regime=`, `at_regime_basis=`, `at_pool=`
/// (Austrian books only), `at_swap_link=`, then the text.
pub(crate) fn event_notes(event: &EventNotes, austrian: bool) -> String {
    let mut tokens = Vec::new();
    if let Some(regime) = present(event.regime) {
        tokens.push(token(MARKER_REGIME, regime));
    }
    if let Some(basis) = present(event.regime_basis) {
        tokens.push(token(MARKER_REGIME_BASIS, basis));
    }
    if let Some(pool) = austrian.then(|| wire_pool(event.pool)).flatten() {
        tokens.push(token(MARKER_POOL, pool));
    }
    if let Some(link) = present(event.swap_link) {
        tokens.push(token(MARKER_SWAP_LINK, link));
    }
    if !event.description.is_empty() {
        tokens.push(event.description.to_owned());
    }
    tokens.join(" ")
}

/// `_compose_transfer_notes`: `at_regime=`, `at_pool=` (Austrian books
/// only), the pairing phrase, then the text.
///
/// The adapter refuses an Austrian move between two cost-basis pools before
/// writing the pool; every move's pools are the global pool (no country
/// allows another scope, and the builder raises the policy's own error for
/// one), so that refusal has no port.
pub(crate) fn transfer_notes(transfer: &TransferNotes, austrian: bool) -> String {
    let mut tokens = Vec::new();
    if let Some(regime) = present(transfer.regime) {
        tokens.push(token(MARKER_REGIME, regime));
    }
    if let Some(pool) = austrian.then(|| wire_pool(transfer.pool)).flatten() {
        tokens.push(token(MARKER_POOL, pool));
    }
    match transfer.pairing_source {
        Some("ownership_derived") => {
            tokens.push(String::from("self-transfer proven by address ownership"))
        }
        Some("recorded_fanout") => tokens.push(String::from(
            "self-transfer proven by complete wallet observations",
        )),
        Some("multi_source_consolidation") => tokens.push(String::from(
            "self-transfer from multi-wallet consolidation",
        )),
        _ => {}
    }
    if !transfer.description.is_empty() {
        tokens.push(transfer.description.to_owned());
    }
    tokens.join(" ")
}

/// `_compose_transfer_journal_description`: `at_regime=` (only `alt` or
/// `neu`), `at_alt_out=` and `at_alt_in=` when the move has regime flows,
/// `at_pool=`, `Transfer <from> -> <to>`, then the provenance. The row's own
/// text is not part of it.
pub(crate) fn transfer_leg_description(leg: &TransferLeg) -> String {
    let mut tokens = Vec::new();
    if let Some(regime @ ("alt" | "neu")) = leg.regime {
        tokens.push(token(MARKER_REGIME, regime));
    }
    if let Some(flows) = leg.flows {
        tokens.push(token(MARKER_ALT_OUT, flows.out.alt));
        tokens.push(token(MARKER_ALT_IN, flows.inbound.alt));
    }
    if let Some(pool) = present(leg.pool) {
        tokens.push(token(MARKER_POOL, pool));
    }
    tokens.push(format!(
        "Transfer {} -> {}",
        leg.from_label.unwrap_or("None"),
        leg.to_label.unwrap_or("None")
    ));
    match leg.pairing_source {
        Some("ownership_derived") => tokens.push(String::from("(proven by address ownership)")),
        Some("recorded_fanout") => {
            tokens.push(String::from("(proven by complete wallet observations)"))
        }
        Some("multi_source_consolidation") => {
            tokens.push(String::from("(multi-wallet consolidation)"))
        }
        _ => {}
    }
    tokens.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::austria;
    use crate::ledger::response::Flows;

    fn event(description: &str) -> EventNotes<'_> {
        EventNotes {
            description,
            ..EventNotes::default()
        }
    }

    // The grammar cases of the adapter's composer, moved from
    // `tests/test_tax_descriptions.py` (`MarkerGrammarTest`): the expected
    // text is what the frozen oracle's `_compose_event_notes` returns for
    // the same values (its default writes Austrian markers).

    #[test]
    fn a_single_regime_marker_precedes_the_text() {
        let notes = EventNotes {
            regime: Some("neu"),
            ..event("Bought BTC")
        };
        assert_eq!(event_notes(&notes, true), "at_regime=neu Bought BTC");
    }

    #[test]
    fn markers_come_in_a_fixed_order() {
        let notes = EventNotes {
            regime: Some("neu"),
            pool: Some("wallet-1"),
            swap_link: Some("swap-42"),
            ..event("Swapped LBTC for BTC")
        };
        assert_eq!(
            event_notes(&notes, true),
            "at_regime=neu at_pool=wallet-1 at_swap_link=swap-42 Swapped LBTC for BTC"
        );
    }

    #[test]
    fn the_regime_basis_follows_the_regime_and_the_global_pool_is_default() {
        // The election case (`wahlrecht`), first pinned in `test_tax_events.py`.
        let notes = EventNotes {
            regime: Some("neu"),
            regime_basis: Some("wahlrecht"),
            pool: Some("global"),
            ..event("Sell")
        };
        assert_eq!(
            event_notes(&notes, true),
            "at_regime=neu at_regime_basis=wahlrecht at_pool=default Sell"
        );
    }

    #[test]
    fn a_description_alone_is_the_notes() {
        assert_eq!(event_notes(&event("Regular buy"), true), "Regular buy");
    }

    #[test]
    fn markers_without_a_description_end_the_notes() {
        let notes = EventNotes {
            regime: Some("alt"),
            pool: Some("wallet-2"),
            ..event("")
        };
        assert_eq!(event_notes(&notes, true), "at_regime=alt at_pool=wallet-2");
    }

    #[test]
    fn nothing_composes_to_empty_notes() {
        assert_eq!(event_notes(&event(""), true), "");
    }

    #[test]
    fn an_empty_marker_value_is_not_written() {
        // A bare `at_swap_link=` would fail in the lot engine.
        let swap = EventNotes {
            swap_link: Some(""),
            ..event("Buy")
        };
        assert_eq!(event_notes(&swap, true), "Buy");
        let notes = EventNotes {
            regime: Some(""),
            regime_basis: Some(""),
            pool: Some(" \t"),
            swap_link: Some(""),
            ..event("Buy")
        };
        assert_eq!(event_notes(&notes, true), "Buy");
    }

    #[test]
    fn generic_notes_carry_no_pool() {
        let notes = EventNotes {
            pool: Some("global"),
            ..event("Bought BTC")
        };
        assert_eq!(event_notes(&notes, false), "Bought BTC");
        assert_eq!(event_notes(&notes, true), "at_pool=default Bought BTC");
    }

    #[test]
    fn the_pool_is_stripped_as_python_strips_it() {
        assert_eq!(wire_pool(None), None);
        assert_eq!(wire_pool(Some("")), None);
        // `"\x1c　 \n".strip()` is empty in CPython.
        assert_eq!(wire_pool(Some("\u{1c}\u{3000} \n")), None);
        assert_eq!(wire_pool(Some(" global\t")).as_deref(), Some("default"));
        assert_eq!(
            wire_pool(Some("\u{1f}wallet-1\u{85}")).as_deref(),
            Some("wallet-1")
        );
        // A zero-width space is not white space to CPython.
        assert_eq!(
            wire_pool(Some("\u{200b}global")).as_deref(),
            Some("\u{200b}global")
        );
    }

    #[test]
    fn the_notes_markers_are_the_ones_the_lot_engine_reads() {
        let notes = event_notes(
            &EventNotes {
                regime: Some("neu"),
                regime_basis: Some("wahlrecht"),
                pool: Some("global"),
                swap_link: Some("swap-1"),
                description: "swap-out via Exchange",
            },
            true,
        );
        assert_eq!(
            austria::regime_from_notes(&notes).unwrap(),
            Some(austria::Regime::Neu)
        );
        assert_eq!(austria::pool_id(&notes).unwrap(), "default");
        assert_eq!(
            austria::swap_link_id(&notes).unwrap().as_deref(),
            Some("swap-1")
        );
    }

    #[test]
    fn move_notes_put_the_regime_and_pool_before_the_proof_and_the_text() {
        let notes = TransferNotes {
            regime: Some("neu"),
            pool: Some("global"),
            pairing_source: Some("reviewed_custody_component"),
            description: "move-out via Exchange",
        };
        // What design §10's `desc.py` rerun printed for an Austrian move and
        // a generic one, whose moves carry no regime.
        assert_eq!(
            transfer_notes(&notes, true),
            "at_regime=neu at_pool=default move-out via Exchange"
        );
        let generic = TransferNotes {
            regime: None,
            ..notes
        };
        assert_eq!(transfer_notes(&generic, false), "move-out via Exchange");
        assert_eq!(
            transfer_notes(&notes, false),
            "at_regime=neu move-out via Exchange"
        );
        for (source, phrase) in [
            (
                "ownership_derived",
                "self-transfer proven by address ownership",
            ),
            (
                "recorded_fanout",
                "self-transfer proven by complete wallet observations",
            ),
            (
                "multi_source_consolidation",
                "self-transfer from multi-wallet consolidation",
            ),
        ] {
            let proven = TransferNotes {
                pairing_source: Some(source),
                ..notes
            };
            assert_eq!(
                transfer_notes(&proven, true),
                format!("at_regime=neu at_pool=default {phrase} move-out via Exchange")
            );
        }
        assert_eq!(
            transfer_notes(
                &TransferNotes {
                    description: "",
                    ..generic
                },
                false
            ),
            ""
        );
    }

    #[test]
    fn transfer_legs_describe_the_move_without_its_text() {
        let flows = RegimeFlows {
            out: Flows { alt: 7, neu: 3 },
            inbound: Flows { alt: 5, neu: 3 },
        };
        let leg = TransferLeg {
            regime: Some("neu"),
            flows: Some(&flows),
            pool: Some("default"),
            from_label: Some("Exchange"),
            to_label: Some("Hot"),
            pairing_source: None,
        };
        assert_eq!(
            transfer_leg_description(&leg),
            "at_regime=neu at_alt_out=7 at_alt_in=5 at_pool=default Transfer Exchange -> Hot"
        );
        // Only `alt` or `neu` is written as a regime; a missing label is
        // `None`, as an f-string writes it.
        let bare = TransferLeg {
            regime: Some("mixed"),
            flows: None,
            pool: None,
            from_label: None,
            to_label: Some("Hot"),
            pairing_source: Some("reviewed_custody_component"),
        };
        assert_eq!(transfer_leg_description(&bare), "Transfer None -> Hot");
        for (source, suffix) in [
            ("ownership_derived", "(proven by address ownership)"),
            (
                "recorded_fanout",
                "(proven by complete wallet observations)",
            ),
            ("multi_source_consolidation", "(multi-wallet consolidation)"),
        ] {
            let proven = TransferLeg {
                pairing_source: Some(source),
                ..bare
            };
            assert_eq!(
                transfer_leg_description(&proven),
                format!("Transfer None -> Hot {suffix}")
            );
        }
    }
}
