//! The exchange syncs after a title-bar refresh (docs/reference/exchange-desktop.md): every
//! exchange API source with a key syncs after the book refresh, one request each, like the wallet
//! page's Sync now (`includeExchangeApis`). Background, startup and menu refreshes never sync an
//! exchange (docs/reference/privacy-and-security.md).

use serde_json::{Value, json};

use super::model::ErrorInfo;
use super::{st, stm};
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};

/// One title-bar refresh's exchange syncs (an explicit click or its shortcut; background,
/// startup and menu refreshes never sync exchanges, docs/reference/privacy-and-security.md).
/// They belong to the book the click was made in: every request names it (`expected_scope`),
/// and a book switch or lock drops the rest (the shell orphans their answers).
#[derive(Default)]
pub struct RefreshRun {
    /// The daemon session the click was made in.
    pub session: u64,
    /// Exchange wallets with a key still to sync: (id, label).
    pub queue: std::collections::VecDeque<(String, String)>,
    /// The one syncing now (its label).
    pub running: Option<String>,
    /// Any sync of this run finished (the book re-reads at the end).
    pub synced: bool,
}

/// After a title-bar refresh: every exchange API source with a key, one sync each, like the
/// wallet page's Sync now (a running job, then a result or a failure).
pub fn sync_after_refresh(app: &mut App) {
    let scope = app.expected_scope();
    stm(app).refresh = RefreshRun {
        session: app.session,
        ..RefreshRun::default()
    };
    app.send(
        "ui.connections.exchange.status",
        json!({"expected_scope": scope}),
        "wallets.exchange.refresh_status",
    );
}

/// The connection syncing now, for the notification center's running job.
pub fn refresh_running(app: &App) -> Option<&str> {
    st(app).refresh.running.as_deref()
}

/// The exchange wallets a refresh syncs: a key is stored and the backend is configured.
pub fn refresh_targets(status: &Value) -> Vec<(String, String)> {
    status["connections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["backend_configured"] == true && row["has_api_key"] == true)
        .filter_map(|row| {
            let id = row["wallet_id"].as_str()?.to_owned();
            let label = row["wallet"].as_str().unwrap_or(&id).to_owned();
            Some((id, label))
        })
        .collect()
}

fn refresh_next(app: &mut App) {
    let next = stm(app).refresh.queue.pop_front();
    match next {
        Some((id, label)) => {
            stm(app).refresh.running = Some(label);
            let scope = app.expected_scope();
            app.send(
                "ui.connections.exchange.sync",
                json!({"wallet": id, "expected_scope": scope}),
                "wallets.exchange.refresh_sync",
            );
        }
        None => {
            let synced = std::mem::take(&mut stm(app).refresh);
            if synced.synced {
                app.refresh_reads();
            }
        }
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if st(app).refresh.session != app.session {
        return;
    }
    if tag == "wallets.exchange.refresh_status" {
        if r.error.is_none() {
            stm(app).refresh.queue = refresh_targets(&r.data).into();
        }
        return refresh_next(app);
    }
    let label = stm(app).refresh.running.take().unwrap_or_default();
    match &r.error {
        // Another book is open now: the rest of the run is not this book's.
        Some(_) if r.error_detail["code"] == "stale_context" => {
            stm(app).refresh = RefreshRun::default();
            return;
        }
        None => {
            stm(app).refresh.synced = true;
            let imported = r.data["imported"].as_u64().unwrap_or(0) as usize;
            let title = t("chrome:notifications.result.connectionSynced.title");
            let body = format!(
                "{label} · {}",
                tc(
                    "chrome:notifications.result.connectionSynced.imported",
                    imported,
                    &[]
                )
            );
            let clock = app.clock;
            app.shell
                .note(&title, Some(body), ToastTone::Success, clock);
            app.toast(
                ToastTone::Success,
                &title,
                Some(label),
                Some("exchange-sync"),
            );
        }
        Some(m) => {
            stm(app).refresh.synced = true;
            let info = ErrorInfo::from_reply(&r.error_detail, Some(m));
            let msg = info.message.clone().unwrap_or_else(|| m.clone());
            let title = tr(
                "chrome:notifications.condition.connectionSyncFailed.titleNamed",
                &[("name", &label)],
            );
            let clock = app.clock;
            app.shell
                .note(&title, Some(msg.clone()), ToastTone::Error, clock);
            app.toast(ToastTone::Error, &title, Some(msg), Some("exchange-sync"));
        }
    }
    refresh_next(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refresh_syncs_only_exchange_wallets_with_a_key() {
        let status = json!({"connections": [
            {"wallet_id": "w1", "wallet": "Kraken", "backend_configured": true, "has_api_key": true},
            {"wallet_id": "w2", "wallet": "Old CSV", "backend_configured": false, "has_api_key": false},
            {"wallet_id": "w3", "wallet": "No key", "backend_configured": true, "has_api_key": false},
        ]});
        assert_eq!(
            refresh_targets(&status),
            vec![("w1".to_owned(), "Kraken".to_owned())]
        );
        assert!(refresh_targets(&json!({})).is_empty());
    }
}
