//! BTCPay store groups on the Wallets list (`ui.sources.groups`,
//! docs/reference/daemon.md#btcpay-store-groups): each store is a section over the wallets it
//! pays into, each member names the payment methods that reach it, and methods no wallet books
//! show as invoice records only. A group is display structure: balances and sync stay per wallet,
//! and a wallet two stores share is one wallet shown under both.

use rune::{Cx, El, row, text};
use serde_json::Value;

use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;
use crate::ui::table::{Tone, chip, tone};

static BTCPAY_LOGO: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/logos/integrations/btcpay.svg"
));

#[derive(Clone, Debug, PartialEq)]
pub struct Method {
    pub id: String,
    /// `btc_onchain` | `liquid_onchain` | `lightning` | `lnurl` | `bark` | `plugin` | `non_bitcoin`.
    pub settlement: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub wallet_id: String,
    /// `mapped` | `wallet_source` | `payment_ledger`.
    pub role: String,
    pub methods: Vec<Method>,
    pub store_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub key: String,
    pub backend: String,
    pub store_id: String,
    pub label: Option<String>,
    pub wallets: Vec<Member>,
    pub records_only: Vec<Method>,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn methods(v: Option<&Value>) -> Vec<Method> {
    v.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|m| Method {
            id: s(m, "id"),
            settlement: s(m, "settlement"),
        })
        .collect()
}

/// The groups of a `ui.sources.groups` reply, in its order.
pub fn parse(v: &Value) -> Vec<Group> {
    v.get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|g| Group {
            key: s(g, "key"),
            backend: s(g, "backend"),
            store_id: s(g, "store_id"),
            label: g
                .get("label")
                .and_then(Value::as_str)
                .filter(|l| !l.trim().is_empty())
                .map(str::to_owned),
            wallets: g
                .get("wallets")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|m| Member {
                    wallet_id: s(m, "wallet_id"),
                    role: s(m, "role"),
                    methods: methods(m.get("payment_methods")),
                    store_count: m.get("store_count").and_then(Value::as_u64).unwrap_or(1) as usize,
                })
                .collect(),
            records_only: methods(g.get("records_only")),
        })
        .collect()
}

/// How many BTCPay servers the groups come from (the server name shows only with two or more).
pub fn backend_count(v: &Value) -> usize {
    v.pointer("/summary/backends")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
}

impl Group {
    /// The store's name, or its shortened id when setup never recorded one.
    pub fn name(&self) -> String {
        match &self.label {
            Some(l) => l.clone(),
            None => {
                let short: String = self.store_id.chars().take(8).collect();
                let more = if self.store_id.chars().count() > 8 {
                    "…"
                } else {
                    ""
                };
                tr(
                    "connections:groups.unnamed",
                    &[("id", &format!("{short}{more}"))],
                )
            }
        }
    }

    /// Every payment method of the store, each once, in the core's rail order.
    pub fn all_methods(&self) -> Vec<&Method> {
        let mut out: Vec<&Method> = Vec::new();
        for m in self
            .wallets
            .iter()
            .flat_map(|w| &w.methods)
            .chain(&self.records_only)
        {
            if !out.iter().any(|x| x.id == m.id) {
                out.push(m);
            }
        }
        out.sort_by_key(|m| rank(&m.settlement));
        out
    }
}

fn rank(settlement: &str) -> u8 {
    match settlement {
        "btc_onchain" => 0,
        "liquid_onchain" => 1,
        "lightning" => 2,
        "lnurl" => 3,
        "bark" => 4,
        "plugin" => 5,
        _ => 6,
    }
}

/// A payment method's short name: its rail, or the plugin's own id.
pub fn method_label(m: &Method) -> String {
    match m.settlement.as_str() {
        "btc_onchain" | "liquid_onchain" | "lightning" | "lnurl" | "bark" => {
            t(&format!("connections:groups.method.{}", m.settlement))
        }
        _ => m.id.clone(),
    }
}

/// A payment method as an outlined pill.
fn pill(tk: &Tk, label: String) -> El {
    row()
        .px(tk.r(0.375))
        .py(tk.r(0.0625))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .child(
            text(label)
                .font(tk.txs(500.0))
                .color(tk.muted_fg)
                .single_line(),
        )
}

/// The store's section header: chevron, BTCPay mark, name, server and wallet count, every payment
/// method, and on the right the summed balance of its wallets (`balance`, already formatted).
#[allow(clippy::too_many_arguments)]
pub fn header(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    g: &Group,
    show_server: bool,
    collapsed: bool,
    lead_w: f32,
    bal_w: f32,
    balance: El,
) -> El {
    let name = g.name();
    let mut meta = vec![t("connections:groups.kind")];
    if show_server {
        meta.push(g.backend.clone());
    }
    meta.push(tc("connections:table.wallets", g.wallets.len(), &[]));
    let logo = row()
        .size(tk.r(2.0), tk.r(2.0))
        .center()
        .radius(tk.r(0.5))
        .fill(tk.muted)
        .child(crate::logos::fitted(
            app,
            cx,
            crate::logos::Logo::svg(BTCPAY_LOGO),
            tk.r(1.125),
            tk.r(1.25),
        ));
    let mut chips = row().gap(tk.r(0.375)).center_y().wrap();
    for m in g.all_methods() {
        chips = chips.child(pill(tk, method_label(m)));
    }
    let key = g.key.clone();
    row()
        .key(format!("wallets-group-{}", g.key))
        .w_full()
        .items_stretch()
        .fill(tk.muted.alpha(0.3))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .hover(tk.muted.alpha(0.5))
        .role("button")
        .label(tr(
            if collapsed {
                "connections:groups.expand"
            } else {
                "connections:groups.collapse"
            },
            &[("store", &name)],
        ))
        .focusable(true)
        .on_press(move |a: &mut App| {
            let c = &mut a.wallets.list.collapsed;
            if !c.remove(&key) {
                c.insert(key.clone());
            }
        })
        .child(
            row()
                .w(lead_w)
                .pl(tk.r(0.5))
                .pr(tk.r(1.5))
                .py(tk.r(0.625))
                .gap(tk.r(0.75))
                .center_y()
                .child(ic(
                    if collapsed {
                        &I::CHEVRON_RIGHT
                    } else {
                        &I::CHEVRON_DOWN
                    },
                    tk.r(1.0),
                    tk.muted_fg,
                ))
                .child(logo)
                .child(
                    rune::col()
                        .min_w(0.0)
                        .child(crate::ui::table::cell_text(name, tk.tsm(600.0), tk.fg))
                        .child(
                            crate::ui::table::cell_text(
                                meta.join(" · "),
                                tk.txs(400.0),
                                tk.muted_fg,
                            )
                            .mt(tk.r(0.125)),
                        ),
                )
                .child(rune::el().grow(1.0))
                .child(chips),
        )
        .child(
            row()
                .w(bal_w)
                .px(tk.r(0.5))
                .py(tk.r(0.5))
                .center_y()
                .justify_end()
                .child(balance),
        )
}

/// The line under a member wallet's name: the methods that pay into it from this store, the
/// role when Kassiber does not sync the wallet itself, and other stores sharing it.
pub fn member_line(tk: &Tk, m: &Member) -> El {
    let mut r = row().gap(tk.r(0.375)).center_y().wrap().mt(tk.r(0.375));
    for method in &m.methods {
        r = r.child(pill(tk, method_label(method)));
    }
    match m.role.as_str() {
        "wallet_source" => {
            r = r.child(
                row()
                    .help(t("connections:groups.roleHelp.wallet_source"))
                    .child(chip(
                        tk,
                        Tone::Neutral,
                        None,
                        t("connections:groups.role.wallet_source"),
                    )),
            );
        }
        "payment_ledger" => {
            r = r.child(
                row()
                    .help(t("connections:groups.roleHelp.payment_ledger"))
                    .child(chip(
                        tk,
                        Tone::Warning,
                        Some(&I::TRIANGLE_ALERT),
                        t("connections:groups.role.payment_ledger"),
                    )),
            );
        }
        _ => {}
    }
    if m.store_count > 1 {
        r = r.child(chip(
            tk,
            Tone::Neutral,
            None,
            tc("connections:groups.shared", m.store_count - 1, &[]),
        ));
    }
    r
}

/// Methods of the store no wallet books: kept as invoice records, said once under its wallets.
pub fn records_line(tk: &Tk, g: &Group, w: f32, indent: f32, last: bool) -> El {
    let names: Vec<String> = g.records_only.iter().map(method_label).collect();
    let (_, ink, _) = tone(tk, Tone::Neutral);
    let mut r = row()
        .key(format!("wallets-group-records-{}", g.key))
        .w(w)
        .pl(indent)
        .pr(tk.r(0.5))
        .py(tk.r(0.5))
        .gap(tk.r(0.5))
        .center_y()
        .help(t("connections:groups.recordsOnlyHelp"))
        .child(ic(&I::RECEIPT, tk.r(0.875), tk.muted_fg))
        .child(
            text(tr(
                "connections:groups.recordsOnly",
                &[("methods", &names.join(", "))],
            ))
            .font(tk.txs(400.0))
            .color(if tk.dark { tk.muted_fg } else { ink }),
        );
    if !last {
        r = r
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout();
    }
    r
}

/// The divider before wallets no store pays into.
pub fn others_header(tk: &Tk, w: f32) -> El {
    row()
        .key("wallets-group-others")
        .w(w)
        .px(tk.r(0.5))
        .pt(tk.r(1.0))
        .pb(tk.r(0.5))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .child(crate::ui::table::th_text(tk, t("connections:groups.other")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn methods_list_once_in_rail_order_across_wallets_and_records() {
        let g = parse(&serde_json::json!({"groups": [{
            "key": "btcpay:b:S", "backend": "b", "store_id": "S", "label": null,
            "wallets": [
                {"wallet_id": "l", "role": "payment_ledger", "store_count": 1,
                 "payment_methods": [{"id": "BTC-LN", "settlement": "lightning"}]},
                {"wallet_id": "m", "role": "mapped", "store_count": 2,
                 "payment_methods": [{"id": "BTC-CHAIN", "settlement": "btc_onchain"},
                                     {"id": "BTC-LN", "settlement": "lightning"}]}],
            "records_only": [{"id": "BTC-ARK", "settlement": "bark"}]}]}));
        let ids: Vec<&str> = g[0].all_methods().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["BTC-CHAIN", "BTC-LN", "BTC-ARK"]);
        assert_eq!(g[0].wallets[1].store_count, 2);
    }
}
