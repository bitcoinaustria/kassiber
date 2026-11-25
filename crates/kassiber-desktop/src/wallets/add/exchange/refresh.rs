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
    /// The exchanges' status read that lists the targets is out.
    pub awaiting_status: bool,
    /// The one syncing now: (id, label).
    pub running: Option<(String, String)>,
    /// Any sync of this run finished (the book re-reads at the end).
    pub synced: bool,
}

impl RefreshRun {
    /// Still listing or syncing: its requests can still answer.
    pub fn active(&self) -> bool {
        self.awaiting_status || self.running.is_some() || !self.queue.is_empty()
    }
}

/// After a title-bar refresh: every exchange API source with a key, one sync each, like the
/// wallet page's Sync now (a running job, then a result or a failure).
pub fn sync_after_refresh(app: &mut App) {
    // A run still listing or syncing in this session already covers every exchange: another
    // click does not replace it while its requests can still answer (Codex 4660319612).
    let current = &st(app).refresh;
    if current.session == app.session && current.active() {
        return;
    }
    let scope = app.expected_scope();
    stm(app).refresh = RefreshRun {
        session: app.session,
        awaiting_status: true,
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
    st(app)
        .refresh
        .running
        .as_ref()
        .map(|(_, label)| label.as_str())
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
            stm(app).refresh.running = Some((id.clone(), label));
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

/// The wallet page's exchange card, when it shows `wallet`: its last error follows this sync and
/// its status is re-read, as after the card's own sync (Codex 4660319617).
fn card_follows(app: &mut App, wallet: &str, r: &Reply) {
    if wallet.is_empty() || st(app).sync.wallet.as_deref() != Some(wallet) {
        return;
    }
    stm(app).sync.last_error = r
        .error
        .as_ref()
        .map(|m| ErrorInfo::from_reply(&r.error_detail, Some(m)));
    stm(app).statuses.push_back(wallet.to_owned());
    app.send(
        "ui.connections.exchange.status",
        json!({"wallet": wallet}),
        "wallets.exchange.status",
    );
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if st(app).refresh.session != app.session {
        return;
    }
    if tag == "wallets.exchange.refresh_status" {
        stm(app).refresh.awaiting_status = false;
        if r.error.is_none() {
            stm(app).refresh.queue = refresh_targets(&r.data).into();
        }
        return refresh_next(app);
    }
    let (wallet, label) = stm(app).refresh.running.take().unwrap_or_default();
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
    card_follows(app, &wallet, r);
    refresh_next(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(kind: &str, data: Value, error: Option<&str>) -> Reply {
        Reply {
            kind: kind.into(),
            data,
            error: error.map(str::to_owned),
            error_detail: Value::Null,
            elapsed: Default::default(),
        }
    }

    fn status(ids: &[&str]) -> Reply {
        let rows: Vec<Value> = ids
            .iter()
            .map(|id| json!({"wallet_id": id, "wallet": id, "backend_configured": true, "has_api_key": true}))
            .collect();
        reply(
            "ui.connections.exchange.status",
            json!({"connections": rows}),
            None,
        )
    }

    fn running(app: &App) -> Option<String> {
        refresh_running(app).map(str::to_owned)
    }

    /// Codex 4660319610: opening an exchange form while a title-bar run syncs keeps the run.
    #[test]
    fn a_form_reset_keeps_the_refresh_run() {
        let mut app = crate::app::tests::quiet_app();
        sync_after_refresh(&mut app);
        on_reply(
            &mut app,
            "wallets.exchange.refresh_status",
            &status(&["a", "b"]),
        );
        super::super::reset(&mut app, "kraken");
        assert_eq!(running(&app).as_deref(), Some("a"));
        on_reply(
            &mut app,
            "wallets.exchange.refresh_sync",
            &reply("ui.connections.exchange.sync", json!({}), None),
        );
        assert_eq!(running(&app).as_deref(), Some("b"), "the queue survived");
    }

    /// Codex 4660319612: a second title-bar refresh while a run still syncs does not replace
    /// it; the running run already covers every exchange.
    #[test]
    fn a_second_refresh_leaves_the_running_run_alone() {
        let mut app = crate::app::tests::quiet_app();
        sync_after_refresh(&mut app);
        on_reply(
            &mut app,
            "wallets.exchange.refresh_status",
            &status(&["a", "b"]),
        );
        sync_after_refresh(&mut app);
        assert_eq!(running(&app).as_deref(), Some("a"));
        assert_eq!(st(&app).refresh.queue.len(), 1);
        on_reply(
            &mut app,
            "wallets.exchange.refresh_sync",
            &reply("ui.connections.exchange.sync", json!({}), None),
        );
        assert_eq!(running(&app).as_deref(), Some("b"));
        on_reply(
            &mut app,
            "wallets.exchange.refresh_sync",
            &reply("ui.connections.exchange.sync", json!({}), None),
        );
        assert_eq!(running(&app), None);
        // Once it ended, the next click starts a run again.
        sync_after_refresh(&mut app);
        assert!(st(&app).refresh.awaiting_status);
    }

    /// Codex 4660319617: a title-bar sync of the wallet whose card is open re-reads its status
    /// and replaces the card's last error, as the card's own sync does.
    #[test]
    fn a_title_bar_sync_refreshes_the_open_cards_status() {
        let mut app = crate::app::tests::quiet_app();
        stm(&mut app).sync = super::super::SyncState {
            wallet: Some("a".into()),
            last_error: Some(ErrorInfo::default()),
            ..Default::default()
        };
        sync_after_refresh(&mut app);
        on_reply(&mut app, "wallets.exchange.refresh_status", &status(&["a"]));
        on_reply(
            &mut app,
            "wallets.exchange.refresh_sync",
            &reply("ui.connections.exchange.sync", json!({}), None),
        );
        assert!(st(&app).sync.last_error.is_none(), "the old error is gone");
        assert!(
            st(&app).statuses.iter().any(|w| w == "a"),
            "its status is re-read"
        );
        sync_after_refresh(&mut app);
        on_reply(&mut app, "wallets.exchange.refresh_status", &status(&["a"]));
        on_reply(
            &mut app,
            "wallets.exchange.refresh_sync",
            &reply("ui.connections.exchange.sync", Value::Null, Some("refused")),
        );
        assert!(
            st(&app).sync.last_error.is_some(),
            "a failed sync shows on the card"
        );
    }

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
