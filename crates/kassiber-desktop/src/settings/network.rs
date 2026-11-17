//! Bitcoin, Lightning and Liquid (NetworkLayerSettingsPanel.tsx): the layer's backends with
//! Set as default / Edit / Add, and on Bitcoin the book network binding (BookNetworkSettings.tsx)
//! with its partition export (NetworkPartitionSettings.tsx).

use rune::{El, TextEdit, col, row, text};
use serde_json::{Value, json};

use super::controls::{
    Btn, PressIf, amber, btn, desc, disclosure, error_line, group, input, opt, para, select, srow,
    summary_title, switch,
};
use super::model::{self, Backend, Layer};
use super::send;
use crate::app::{App, Reply};
use crate::i18n::{t, tc, tr};
use crate::theme::Tk;

const ENVIRONMENTS: [&str; 4] = ["main", "test", "signet", "regtest"];

#[derive(Default)]
pub struct BookNet {
    pub env: Option<String>,
    pub instance: TextEdit,
    pub declare_all: bool,
    pub plan: Option<Value>,
    pub error: Option<String>,
    pub part_selected: Vec<String>,
    pub part_dest: TextEdit,
    pub part_pass: TextEdit,
    pub part_confirm: TextEdit,
    /// (args key, plan) of the last partition preview.
    pub part_preview: Option<(String, Value)>,
    pub part_key_sent: Option<String>,
    pub part_error: Option<String>,
    pub part_saved: bool,
    /// `toastopen`: Show in Finder for the partition just exported.
    pub part_saved_actions: Vec<crate::toastopen::ToastAction>,
}

pub fn load(app: &mut App) {
    send(app, "ui.networks.binding", Value::Null, "settings.binding");
}

fn binding(app: &App) -> Option<&Value> {
    app.read("ui.networks.binding")
}

/// The inventory of an unbound book, when it belongs to the bound read's profile.
fn inventory(app: &App) -> Option<&Value> {
    let b = binding(app)?;
    if b["state"].as_str() != Some("unbound") {
        return None;
    }
    let inv = app.read("ui.networks.inventory")?;
    let state = inv["binding"]["state"].as_str().or(inv["state"].as_str());
    let pid = inv["binding"]["profile_id"]
        .as_str()
        .or(inv["profile_id"].as_str());
    (state == Some("unbound") && pid == b["profile_id"].as_str()).then_some(inv)
}

/// `bookEvidenceNetwork`: the one environment the connections show, else mainnet.
fn evidence_network(inv: Option<&Value>) -> String {
    let mut seen: Vec<String> = vec![];
    for w in inv
        .and_then(|i| i["wallets"].as_array())
        .into_iter()
        .flatten()
    {
        for e in w["environments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !seen.iter().any(|s| s == e) {
                seen.push(e.to_owned());
            }
        }
    }
    if seen.len() == 1 && ENVIRONMENTS.contains(&seen[0].as_str()) {
        seen[0].clone()
    } else {
        "main".into()
    }
}

fn environment(app: &App) -> String {
    app.settings
        .bn
        .env
        .clone()
        .unwrap_or_else(|| evidence_network(inventory(app)))
}

fn undeclared(inv: &Value) -> Vec<String> {
    inv["wallets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|w| w["requires_declaration"].as_bool() == Some(true))
        .filter_map(|w| w["wallet_id"].as_str().map(str::to_owned))
        .collect()
}

fn recipe(app: &App) -> Value {
    let env = environment(app);
    let declared: Vec<String> = if app.settings.bn.declare_all {
        inventory(app).map(undeclared).unwrap_or_default()
    } else {
        vec![]
    };
    json!({
        "environment": env,
        "chain_instance_id": if env == "regtest" { json!(app.settings.bn.instance.text) } else { Value::Null },
        "declared_wallet_ids": declared,
    })
}

fn sorted(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    out.sort();
    out
}

/// The preview still matches the book and the form.
fn current_plan(app: &App) -> Option<&Value> {
    let plan = app.settings.bn.plan.as_ref()?;
    let inv = inventory(app)?;
    let r = recipe(app);
    (plan["profile_id"] == inv["profile_id"]
        && plan["inventory_digest"] == inv["inventory_digest"]
        && plan["environment"] == r["environment"]
        && plan["chain_instance_id"] == r["chain_instance_id"]
        && sorted(&plan["declared_wallet_ids"]) == sorted(&r["declared_wallet_ids"]))
    .then_some(plan)
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "settings.binding" => {
            if r.error.is_none() && r.data["state"].as_str() == Some("unbound") {
                send(
                    app,
                    "ui.networks.inventory",
                    json!({}),
                    "settings.inventory",
                );
            }
        }
        "settings.bn_plan" => match &r.error {
            None => app.settings.bn.plan = Some(r.data.clone()).filter(|d| !d.is_null()),
            Some(e) => app.settings.bn.error = Some(e.clone()),
        },
        "settings.bn_bind" => match &r.error {
            None => {
                app.settings.bn.plan = None;
                load(app);
            }
            Some(e) => app.settings.bn.error = Some(e.clone()),
        },
        "settings.part_plan" => match &r.error {
            None => {
                if let Some(k) = app.settings.bn.part_key_sent.take() {
                    app.settings.bn.part_preview = Some((k, r.data.clone()));
                }
            }
            Some(e) => app.settings.bn.part_error = Some(e.clone()),
        },
        "settings.part_export" => match &r.error {
            None => {
                app.settings.bn.part_pass = TextEdit::new("");
                app.settings.bn.part_confirm = TextEdit::new("");
                app.settings.bn.part_saved = true;
                app.settings.bn.part_saved_actions =
                    crate::toastopen::for_path(&app.settings.bn.part_dest.text, false);
            }
            Some(e) => app.settings.bn.part_error = Some(e.clone()),
        },
        _ => {}
    }
}

/// The layer page.
pub fn view(app: &App, tk: &Tk, layer: &mut Vec<El>, l: Layer) -> El {
    let backends = model::backends(
        app.read("ui.backends.settings.list"),
        app.read("ui.maintenance.settings"),
    );
    // Backends first. The book's network binding follows while it still needs a decision; once
    // bound it is reference detail, behind Advanced (an owner request; the web leads with it).
    let page = col()
        .w_full()
        .gap(tk.r(1.5))
        .child(layer_group(app, tk, &backends, l));
    if l != Layer::Bitcoin {
        return page;
    }
    if is_bound(app) {
        page.child(super::controls::advanced(
            app,
            tk,
            ADVANCED,
            vec![book_network(app, tk, layer)],
        ))
    } else {
        page.child(book_network(app, tk, layer))
    }
}

/// The Bitcoin page's Advanced fold (`search::reveal_key`).
pub const ADVANCED: &str = "set-adv-bitcoin";

/// Whether the book's network is bound (the binding is then only shown, not decided).
pub fn is_bound(app: &App) -> bool {
    binding(app).is_some_and(|b| b["state"].as_str() == Some("bound"))
}

fn layer_group(app: &App, tk: &Tk, backends: &[Backend], l: Layer) -> El {
    let id = l.id();
    let rows = model::for_layer(backends, l);
    let list = app.read("ui.backends.settings.list");
    let fallbacks = app.settings.prefs.explorer_fallbacks_allowed
        && model::public_fallbacks(backends, model::bootstrap_mode(list));
    let explorer = if l == Layer::Lightning {
        None
    } else {
        rows.iter().find_map(|b| model::explorer_base(b))
    };
    let links = match l {
        Layer::Lightning => t("settings:networkLayer.readOnly"),
        _ => match explorer {
            Some(base) => tr(
                "settings:networkLayer.explorerLinks",
                &[("host", &model::host_label(&base))],
            ),
            None if fallbacks => tr(
                "settings:networkLayer.publicLinks",
                &[(
                    "host",
                    if l == Layer::Bitcoin {
                        "mempool.bitcoin-austria.at"
                    } else {
                        "Liquid Network"
                    },
                )],
            ),
            None => t("settings:networkLayer.noPublicLinks"),
        },
    };
    let anchor = match l {
        Layer::Bitcoin => "bitcoinBackends",
        Layer::Lightning => "lightningNodes",
        Layer::Liquid => "liquidBackends",
    };
    let type_id = match l {
        Layer::Bitcoin => "bitcoin",
        Layer::Lightning => "lnd",
        Layer::Liquid => "liquid",
    };
    let add = btn(
        tk,
        format!("set-add-{id}"),
        &t(&format!("settings:networkLayer.{id}.add")),
        Btn::Outline,
        true,
    )
    .on_press(move |a: &mut App| super::backend::open_add(a, Some(type_id)));
    let mut g = group()
        .anchor(anchor)
        .title(t(&format!("settings:networkLayer.{id}.heading")))
        .action(add)
        .footer(format!(
            "{} {links}",
            t(&format!("settings:networkLayer.{id}.blurb"))
        ));
    if rows.is_empty() {
        g = g.row(para(
            tk,
            t(&format!("settings:networkLayer.{id}.empty")),
            tk.muted_fg,
        ));
    } else {
        for b in rows {
            g = g.row(backend_row(app, tk, b));
        }
    }
    g.build(app, tk)
}

/// `BackendLayerRow`.
fn backend_row(app: &App, tk: &Tk, b: &Backend) -> El {
    let trust = model::backend_trust(b);
    let explorer = model::explorer_base(b);
    let mut parts: Vec<String> = vec![];
    if b.is_default {
        parts.push(t("settings:networkLayer.default"));
    }
    parts.push(model::protocol_label(b).to_owned());
    parts.push(trust.label.clone());
    if let Some(e) = &explorer {
        parts.push(tr(
            "settings:networkLayer.links",
            &[("host", &model::host_label(e))],
        ));
    }
    if b.silent_payments == Some(true) {
        parts.push(t("settings:networkLayer.silentPayments"));
    }
    let mut d = col()
        .w_full()
        .min_w(0.0)
        .child(
            text(parts.join(" · "))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .help(trust.note.clone()),
        )
        .child(
            text(b.url.clone())
                .font(tk.txs(400.0).mono())
                .color(tk.muted_fg)
                .truncate(),
        );
    if model::is_coreln_commando(b) {
        d = d.child(
            text(format!(
                "{} {}",
                t("settings:networkLayer.commandoUnsupported"),
                t("settings:backendModal.corelnCommandoHint")
            ))
            .font(tk.tsm(400.0))
            .color(amber(tk))
            .wrap()
            .role("alert"),
        );
    }
    let mut controls = vec![];
    if !b.on {
        controls.push(
            text(t("settings:networkLayer.noEndpoint"))
                .font(tk.tsm(400.0))
                .color(amber(tk)),
        );
    }
    if !b.is_default {
        let pending = app.settings.setting_default.as_deref() == Some(b.id.as_str());
        let enabled = app.settings.setting_default.is_none();
        let id = b.id.clone();
        controls.push(
            btn(
                tk,
                format!("set-default-{}", b.id),
                &t(if pending {
                    "settings:networkLayer.settingDefault"
                } else {
                    "settings:networkLayer.setDefault"
                }),
                Btn::Outline,
                enabled,
            )
            .press_if(enabled, move |a| {
                a.settings.setting_default = Some(id.clone());
                send(
                    a,
                    "ui.backends.set_default",
                    json!({"name": id}),
                    "settings.set_default",
                );
            }),
        );
    }
    let edit = b.clone();
    controls.push(
        btn(
            tk,
            format!("set-edit-{}", b.id),
            &t("common:actions.edit"),
            Btn::Outline,
            true,
        )
        .label(tr("settings:networkLayer.editAria", &[("name", &b.name)]))
        .on_press(move |a: &mut App| super::backend::open_edit(a, &edit)),
    );
    srow(app, tk, b.name.clone(), Some(d), controls)
}

// ---- Book network ----

fn book_network(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let bn = &app.settings.bn;
    let b = binding(app);
    let bound = b.is_some_and(|b| b["state"].as_str() == Some("bound"));
    let inv = inventory(app);
    let mut g = group()
        .anchor("bookNetwork")
        .title(t("settings:bookNetwork.title"));
    if bound {
        g = g.footer(t("settings:bookNetwork.immutable"));
    } else if inv.is_some() {
        g = g.footer(t("settings:bookNetwork.help"));
    }
    let binding_loading = app.settings.is_busy("settings.binding") && b.is_none();
    let inv_loading = app.settings.is_busy("settings.inventory") && inv.is_none();
    if binding_loading || inv_loading {
        g = g.row(para(tk, t("common:state.loading"), tk.muted_fg));
    }
    if b.is_none() && !app.settings.is_busy("settings.binding") && app.settings.loaded.is_some() {
        g = g.row(srow(
            app,
            tk,
            t("settings:bookNetwork.loadError"),
            None,
            vec![
                btn(
                    tk,
                    "set-bn-retry",
                    &t("common:actions.retry"),
                    Btn::Outline,
                    true,
                )
                .on_press(|a: &mut App| load(a)),
            ],
        ));
    }
    if let Some(b) = b.filter(|_| bound) {
        let domains = b["domains"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| {
                format!(
                    "{}: {}",
                    d["chain"].as_str().unwrap_or(""),
                    d["network"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join(" · ");
        let mut d = col()
            .w_full()
            .child(text(domains).font(tk.tsm(400.0)).color(tk.muted_fg).wrap());
        if let Some(ci) = b["chain_instance_id"].as_str() {
            d = d.child(
                text(ci.to_owned())
                    .font(tk.txs(400.0).mono())
                    .color(tk.muted_fg)
                    .wrap_anywhere(),
            );
        }
        let env = b["environment"]
            .as_str()
            .or(b["environment_id"].as_str())
            .unwrap_or("main");
        g = g.row(srow(
            app,
            tk,
            t("settings:bookNetwork.environment"),
            Some(d),
            vec![
                text(t(&format!("settings:bookNetwork.environments.{env}")))
                    .font(tk.tsm(400.0))
                    .color(tk.fg),
            ],
        ));
    } else if let Some(inv) = inv {
        let env = environment(app);
        let env_sel = select(
            app,
            tk,
            layer,
            "set-bn-env",
            &t("settings:bookNetwork.environment"),
            &env,
            ENVIRONMENTS
                .iter()
                .map(|e| opt(*e, t(&format!("settings:bookNetwork.environments.{e}"))))
                .collect(),
            true,
            None,
            |a, v| a.settings.bn.env = Some(v.to_owned()),
        );
        g = g.row(srow(
            app,
            tk,
            t("settings:bookNetwork.environment"),
            None,
            vec![env_sel],
        ));
        if env == "regtest" {
            let field = input(
                tk,
                &bn.instance,
                "set-bn-instance",
                &t("settings:bookNetwork.instance"),
                "",
                false,
                true,
                tk.r(2.0),
                true,
                |a: &mut App| &mut a.settings.bn.instance,
            )
            .w(tk.r(18.0));
            let newb = btn(
                tk,
                "set-bn-new-instance",
                &t("settings:bookNetwork.newInstance"),
                Btn::Outline,
                true,
            )
            .on_press(|a: &mut App| a.settings.bn.instance = TextEdit::new(model::uuid_v4()));
            g = g.row(srow(
                app,
                tk,
                t("settings:bookNetwork.instance"),
                None,
                vec![field, newb],
            ));
        }
        let und = undeclared(inv);
        if !und.is_empty() {
            let env_label = t(&format!("settings:bookNetwork.environments.{env}"));
            let d = desc(
                tk,
                tc(
                    "settings:bookNetwork.declareAll.description",
                    und.len(),
                    &[("environment", &env_label)],
                ),
            );
            let sw = switch(
                tk,
                "set-bn-declare",
                bn.declare_all,
                &t("settings:bookNetwork.declareAll.label"),
                true,
                |a, v| a.settings.bn.declare_all = v,
            );
            g = g.row(srow(
                app,
                tk,
                t("settings:bookNetwork.declareAll.label"),
                d,
                vec![sw],
            ));
        }
        // Each connection's recorded network, folded behind one expander row.
        let wallets: Vec<&Value> = inv["wallets"].as_array().into_iter().flatten().collect();
        if !wallets.is_empty() {
            let open = super::is_open(app, "set-bn-wallets", false);
            let label = tc("settings:bookNetwork.connectionsList", wallets.len(), &[]);
            g = g.row(
                row()
                    .key("set-bn-wallets")
                    .w_full()
                    .px(tk.r(1.0))
                    .py(tk.r(0.75))
                    .gap(tk.r(0.5))
                    .center_y()
                    .hover(tk.muted.alpha(0.4))
                    .role("button")
                    .label(label.clone())
                    .expanded(open)
                    .focusable(true)
                    .on_press(|a: &mut App| super::toggle_open(a, "set-bn-wallets", false))
                    .child(
                        crate::ui::ic(
                            if open {
                                &crate::icons::CHEVRON_DOWN
                            } else {
                                &crate::icons::CHEVRON_RIGHT
                            },
                            tk.r(1.0),
                            tk.muted_fg,
                        )
                        .a11y_hidden(),
                    )
                    .child(
                        text(label)
                            .font(tk.tsm(500.0))
                            .color(tk.fg)
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            );
            if open {
                for w in wallets {
                    let envs = w["environments"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ");
                    let envs = if envs.is_empty() {
                        t("settings:bookNetwork.unknown")
                    } else {
                        envs
                    };
                    g = g.row(srow(
                        app,
                        tk,
                        w["label"].as_str().unwrap_or("").to_owned(),
                        desc(tk, envs),
                        vec![],
                    ));
                }
            }
        }
        let plan = current_plan(app);
        if let Some(p) = plan {
            for blocker in p["blockers"].as_array().into_iter().flatten() {
                let code = blocker["code"].as_str().unwrap_or("");
                let label = blocker["wallet_id"]
                    .as_str()
                    .and_then(|id| {
                        inv["wallets"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find(|w| w["wallet_id"].as_str() == Some(id))
                    })
                    .and_then(|w| w["label"].as_str())
                    .unwrap_or("");
                g = g.row(para(
                    tk,
                    format!(
                        "{} {label}",
                        t(&format!("settings:bookNetwork.blockers.{code}"))
                    ),
                    tk.destructive,
                ));
            }
        }
        let can_apply = plan.is_some_and(|p| p["can_apply"].as_bool() == Some(true));
        if can_apply {
            g = g.row(para(
                tk,
                t("settings:bookNetwork.confirmHelp"),
                super::controls::amber_soft(tk),
            ));
        }
        let busy =
            app.settings.is_busy("settings.bn_plan") || app.settings.is_busy("settings.bn_bind");
        let mut actions = row()
            .w_full()
            .px(tk.r(1.0))
            .py(tk.r(0.75))
            .gap(tk.r(0.5))
            .justify_end()
            .child(
                btn(
                    tk,
                    "set-bn-preview",
                    &t("settings:bookNetwork.preview"),
                    Btn::Outline,
                    !busy,
                )
                .press_if(!busy, |a| {
                    a.settings.bn.error = None;
                    let r = recipe(a);
                    send(a, "ui.networks.plan", r, "settings.bn_plan");
                }),
            );
        if can_apply {
            let bind_busy = app.settings.is_busy("settings.bn_bind");
            actions = actions.child(
                btn(
                    tk,
                    "set-bn-bind",
                    &t("settings:bookNetwork.bind"),
                    Btn::Outline,
                    !bind_busy,
                )
                .press_if(!bind_busy, |a| {
                    a.settings.bn.error = None;
                    let plan_id =
                        current_plan(a).and_then(|p| p["plan_id"].as_str().map(str::to_owned));
                    if let Some(id) = plan_id {
                        let mut r = recipe(a);
                        r["plan_id"] = json!(id);
                        send(a, "ui.networks.bind", r, "settings.bn_bind");
                    }
                }),
            );
        }
        g = g.row(actions);
    }
    if let Some(e) = &bn.error {
        g = g.row(
            col()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .child(error_line(tk, e.clone())),
        );
    }
    let mut c = col().w_full().gap(tk.r(0.75)).child(g.build(app, tk));
    if let (Some(inv), false) = (inv, bound) {
        c = c.child(partition(app, tk, inv));
    }
    c
}

fn partition_args(app: &App) -> Value {
    let env = environment(app);
    let selected = &app.settings.bn.part_selected;
    let declared: Vec<String> = if app.settings.bn.declare_all {
        inventory(app).map(undeclared).unwrap_or_default()
    } else {
        vec![]
    };
    json!({
        "environment": env,
        "chain_instance_id": if env == "regtest" { json!(app.settings.bn.instance.text) } else { Value::Null },
        "wallet_ids": selected,
        "declared_wallet_ids": declared.into_iter().filter(|id| selected.contains(id)).collect::<Vec<_>>(),
    })
}

fn partition_key(app: &App, inv: &Value) -> String {
    json!([
        inv["profile_id"],
        inv["inventory_digest"],
        partition_args(app)
    ])
    .to_string()
}

/// `NetworkPartitionSettings`.
fn partition(app: &App, tk: &Tk, inv: &Value) -> El {
    let bn = &app.settings.bn;
    let mut rows: Vec<El> = vec![para(
        tk,
        t("settings:bookNetwork.partition.help"),
        tk.muted_fg,
    )];
    for w in inv["wallets"].as_array().into_iter().flatten() {
        let id = w["wallet_id"].as_str().unwrap_or("").to_owned();
        let label = w["label"].as_str().unwrap_or("").to_owned();
        let on = bn.part_selected.contains(&id);
        let sw = switch(
            tk,
            format!("set-split-{id}"),
            on,
            &label,
            true,
            move |a, v| {
                if v {
                    if !a.settings.bn.part_selected.contains(&id) {
                        a.settings.bn.part_selected.push(id.clone());
                    }
                } else {
                    a.settings.bn.part_selected.retain(|x| x != &id);
                }
            },
        );
        rows.push(srow(app, tk, label, None, vec![sw]));
    }
    let key = partition_key(app, inv);
    let plan_busy = app.settings.is_busy("settings.part_plan");
    let can_preview = !bn.part_selected.is_empty() && !plan_busy;
    rows.push(
        row()
            .w_full()
            .px(tk.r(1.0))
            .py(tk.r(0.75))
            .justify_end()
            .child(
                btn(
                    tk,
                    "set-split-preview",
                    &t("settings:bookNetwork.preview"),
                    Btn::Outline,
                    can_preview,
                )
                .press_if(can_preview, |a| {
                    a.settings.bn.part_error = None;
                    a.settings.bn.part_saved = false;
                    let inv = inventory(a).cloned();
                    if let Some(inv) = inv {
                        a.settings.bn.part_key_sent = Some(partition_key(a, &inv));
                        let args = partition_args(a);
                        send(a, "ui.networks.partition_plan", args, "settings.part_plan");
                    }
                }),
            ),
    );
    let current = bn
        .part_preview
        .as_ref()
        .filter(|(k, p)| {
            *k == key
                && p["profile_id"] == inv["profile_id"]
                && p["inventory_digest"] == inv["inventory_digest"]
        })
        .map(|(_, p)| p);
    if let Some(p) = current {
        let wallets = inv["wallets"].as_array().cloned().unwrap_or_default();
        for blocker in p["blockers"].as_array().into_iter().flatten() {
            let code = blocker["code"].as_str().unwrap_or("");
            let ids: Vec<String> = blocker["wallet_ids"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_else(|| {
                    blocker["wallet_id"]
                        .as_str()
                        .map(|s| vec![s.to_owned()])
                        .unwrap_or_default()
                });
            let names = ids
                .iter()
                .filter_map(|id| {
                    wallets
                        .iter()
                        .find(|w| w["wallet_id"].as_str() == Some(id))
                        .and_then(|w| w["label"].as_str())
                })
                .collect::<Vec<_>>()
                .join(", ");
            rows.push(para(
                tk,
                format!(
                    "{} {names}",
                    t(&format!("settings:bookNetwork.partition.blockers.{code}"))
                ),
                tk.destructive,
            ));
        }
        if p["can_apply"].as_bool() == Some(true) {
            rows.push(para(
                tk,
                tr(
                    "settings:bookNetwork.partition.counts",
                    &[
                        (
                            "wallets",
                            &p["counts"]["wallets"].as_u64().unwrap_or(0).to_string(),
                        ),
                        (
                            "transactions",
                            &p["counts"]["transactions"]
                                .as_u64()
                                .unwrap_or(0)
                                .to_string(),
                        ),
                    ],
                ),
                tk.fg,
            ));
            rows.push(srow(
                app,
                tk,
                t("settings:bookNetwork.partition.passphrase"),
                None,
                vec![
                    input(
                        tk,
                        &bn.part_pass,
                        "set-split-pass",
                        &t("settings:bookNetwork.partition.passphrase"),
                        "",
                        true,
                        true,
                        tk.r(2.0),
                        false,
                        |a: &mut App| &mut a.settings.bn.part_pass,
                    )
                    .w(tk.r(12.0)),
                ],
            ));
            rows.push(srow(
                app,
                tk,
                t("settings:bookNetwork.partition.confirmPassphrase"),
                None,
                vec![
                    input(
                        tk,
                        &bn.part_confirm,
                        "set-split-confirm",
                        &t("settings:bookNetwork.partition.confirmPassphrase"),
                        "",
                        true,
                        true,
                        tk.r(2.0),
                        false,
                        |a: &mut App| &mut a.settings.bn.part_confirm,
                    )
                    .w(tk.r(12.0)),
                ],
            ));
            // The destination field, and the desktop's native Save dialog that fills it
            // (`saveFile({ defaultPath: "network-partition.kassiber" })`).
            let pick = btn(
                tk,
                "set-split-dest-pick",
                &t("common:actions.save"),
                Btn::Outline,
                true,
            )
            .on_press_tx(|_a: &mut App, tx: &mut rune::Tx| {
                let opts = rune::FileDialogOptions {
                    file_name: Some("network-partition.kassiber".into()),
                    ..Default::default()
                };
                tx.save_file(opts, |a: &mut App, result, _tx: &mut rune::Tx| {
                    if let Ok(Some(path)) = result {
                        a.settings.bn.part_dest =
                            TextEdit::new(path.to_string_lossy().into_owned());
                    }
                });
            });
            rows.push(srow(
                app,
                tk,
                t("settings:bookNetwork.partition.destination"),
                None,
                vec![
                    input(
                        tk,
                        &bn.part_dest,
                        "set-split-dest",
                        &t("settings:bookNetwork.partition.destination"),
                        "",
                        false,
                        true,
                        tk.r(2.0),
                        true,
                        |a: &mut App| &mut a.settings.bn.part_dest,
                    )
                    .w(tk.r(18.0)),
                    pick,
                ],
            ));
            let ok = bn.part_pass.text.chars().count() >= 12
                && bn.part_pass.text == bn.part_confirm.text
                && !bn.part_dest.text.is_empty()
                && !app.settings.is_busy("settings.part_export");
            let plan_id = p["plan_id"].as_str().unwrap_or("").to_owned();
            rows.push(
                row()
                    .w_full()
                    .px(tk.r(1.0))
                    .py(tk.r(0.75))
                    .justify_end()
                    .child(
                        btn(
                            tk,
                            "set-split-export",
                            &t("settings:bookNetwork.partition.export"),
                            Btn::Outline,
                            ok,
                        )
                        .press_if(ok, move |a| {
                            a.settings.bn.part_error = None;
                            a.settings.bn.part_saved = false;
                            let mut args = partition_args(a);
                            args["plan_id"] = json!(plan_id);
                            args["file_path"] = json!(a.settings.bn.part_dest.text);
                            args["backup_passphrase"] = json!(a.settings.bn.part_pass.text);
                            send(
                                a,
                                "ui.networks.partition_export",
                                args,
                                "settings.part_export",
                            );
                        }),
                    ),
            );
        }
    }
    if let Some(e) = &bn.part_error {
        rows.push(
            col()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .child(error_line(tk, e.clone())),
        );
    }
    if bn.part_saved {
        rows.push(para(tk, t("settings:bookNetwork.partition.saved"), tk.fg).role("status"));
        if !bn.part_saved_actions.is_empty() {
            rows.push(
                crate::toastopen::links(
                    tk,
                    &bn.part_saved_actions,
                    |a| format!("set-split-{}", a.id),
                    tk.muted_fg,
                    tk.fg,
                )
                .px(tk.r(1.0))
                .pb(tk.r(0.75)),
            );
        }
    }
    disclosure(
        app,
        tk,
        "set-partition",
        summary_title(tk, t("settings:bookNetwork.partition.title")),
        false,
        rows,
        None,
    )
}
