//! Swap provider evidence a `raw_json` may carry, as the provider matcher
//! reads it (`custody/src/interpretation/matcher/providers.rs`): provider
//! names, swap flows and statuses. Each parser recognizes a known spelling
//! (as written, upper- or lower-cased) and prints that constant again;
//! anything else is pseudonymized by the caller, which keeps equal values
//! equal.

/// Provider names: the matcher folds every spelling containing `boltz` or
/// `bullbitcoin` (or exactly `bull`) to one provider.
const PROVIDERS: &[&str] = &[
    "boltz",
    "Boltz",
    "boltz-exchange",
    "boltz_exchange",
    "Boltz Exchange",
    "bullbitcoin",
    "BullBitcoin",
    "bull-bitcoin",
    "bull_bitcoin",
    "Bull Bitcoin",
    "bull",
    "Bull",
];

/// Swap flows the matcher's `kind` and `flow` read.
const FLOWS: &[&str] = &[
    "chain",
    "chain-swap",
    "chain_swap",
    "chainswap",
    "reverse",
    "reverse-swap",
    "reverse_swap",
    "reverse-submarine",
    "reverse_submarine",
    "reverse-submarine-swap",
    "reverse_submarine_swap",
    "submarine",
    "submarine-swap",
    "submarine_swap",
    "refund",
    "swap-refund",
    "swap_refund",
    "failed-swap-refund",
    "failed_swap_refund",
];

/// Swap statuses: Boltz's event names and the generic ones providers use.
const STATUSES: &[&str] = &[
    "swap.created",
    "swap.expired",
    "swap.refunded",
    "invoice.set",
    "invoice.pending",
    "invoice.paid",
    "invoice.settled",
    "invoice.failedToPay",
    "invoice.expired",
    "transaction.mempool",
    "transaction.confirmed",
    "transaction.claim.pending",
    "transaction.claimed",
    "transaction.refunded",
    "transaction.failed",
    "transaction.lockupFailed",
    "transaction.server.mempool",
    "transaction.server.confirmed",
    "pending",
    "confirmed",
    "completed",
    "complete",
    "settled",
    "claimed",
    "refunded",
    "refund",
    "failed",
    "expired",
    "final",
    "finalized",
    "success",
    "succeeded",
];

fn known(values: &[&str], text: &str) -> Option<String> {
    values.iter().find_map(|value| {
        [
            (*value).to_owned(),
            value.to_ascii_uppercase(),
            value.to_ascii_lowercase(),
        ]
        .into_iter()
        .find(|spelling| spelling == text)
    })
}

pub fn provider(text: &str) -> Option<String> {
    known(PROVIDERS, text)
}

pub fn flow(text: &str) -> Option<String> {
    known(FLOWS, text)
}

pub fn status(text: &str) -> Option<String> {
    known(STATUSES, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_spellings_are_printed_again_and_nothing_else_parses() {
        assert_eq!(provider("BOLTZ").as_deref(), Some("BOLTZ"));
        assert_eq!(flow("Reverse_Swap"), None);
        assert_eq!(flow("reverse_swap").as_deref(), Some("reverse_swap"));
        assert_eq!(
            status("transaction.claimed").as_deref(),
            Some("transaction.claimed")
        );
        for text in ["", "boltz.private.example", "alice", "transaction.claimed "] {
            assert_eq!(provider(text), None);
            assert_eq!(flow(text), None);
            assert_eq!(status(text), None);
        }
    }
}
