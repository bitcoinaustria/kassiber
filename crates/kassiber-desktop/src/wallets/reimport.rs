//! Re-import feedback (docs/reference/imports.md#re-importing-a-newer-export): an import's
//! counts (new, already in the book, updated) and the values the book kept, and the refusal for
//! rows that look like a stored row with its two answers.
//!
//! The daemon decides; this module only reads its results and draws them. Tags: none of its own
//! (the dialog and the exchange card send the requests).

use rune::{El, col, row, text};
use serde_json::Value;

use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;
use crate::wallets::add::btcpay::form::{Variant, button, xs};

/// An import's or sync's counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Rows imported.
    pub new: u64,
    /// Rows the book already had and left as they were.
    pub known: u64,
    /// Rows the book had and the import enriched.
    pub updated: u64,
}

/// `updated`, or the length of `updated_records` for a result that does not name it; `None`
/// when the result says neither (not an import result).
pub fn updated(r: &Value) -> Option<u64> {
    r.get("updated").and_then(Value::as_u64).or_else(|| {
        r.get("updated_records")
            .and_then(Value::as_array)
            .map(|a| a.len() as u64)
    })
}

impl Counts {
    pub fn of(r: &Value) -> Counts {
        let n = |k: &str| r.get(k).and_then(Value::as_u64).unwrap_or(0);
        Counts {
            new: n("imported"),
            known: n("unchanged"),
            updated: updated(r).unwrap_or(0),
        }
    }

    /// "3 new · 120 already in the book · 2 updated" (the last two only when above zero).
    pub fn text(&self) -> String {
        let mut parts = vec![tc("connections:reimport.new", self.new as usize, &[])];
        if self.known > 0 {
            parts.push(tc("connections:reimport.known", self.known as usize, &[]));
        }
        if self.updated > 0 {
            parts.push(tc(
                "connections:reimport.updated",
                self.updated as usize,
                &[],
            ));
        }
        parts.join(" · ")
    }
}

/// The values an import found different from the book and did not write, as one line.
pub fn corrections_line(r: &Value) -> Option<String> {
    let n = r
        .get("corrections_ignored")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    (n > 0).then(|| tc("connections:reimport.corrections", n, &[]))
}

/// What an import that met changed rows did with them, as one line.
pub fn changed_line(r: &Value) -> Option<String> {
    let c = r.get("changed_rows").filter(|c| c.is_object())?;
    let n = c["reidentified"].as_u64().unwrap_or(0) + c["corrected"].as_u64().unwrap_or(0);
    if n == 0 {
        return None;
    }
    let key = match c["action"].as_str()? {
        "skip" => "connections:reimport.keptRows",
        "import" => "connections:reimport.importedRows",
        _ => return None,
    };
    Some(tc(key, n as usize, &[]))
}

/// The user's answer to a refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Skip,
    Import,
}

impl Answer {
    pub fn as_str(self) -> &'static str {
        match self {
            Answer::Skip => "skip",
            Answer::Import => "import",
        }
    }
}

/// An `import_changed_rows` refusal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Refusal {
    pub reidentified: u64,
    pub corrected: u64,
}

impl Refusal {
    /// The refusal in a failed request's error object, if it is one.
    pub fn of(error: &Value) -> Option<Refusal> {
        if error["code"].as_str() != Some("import_changed_rows") {
            return None;
        }
        let n = |k: &str| error["details"][k].as_u64().unwrap_or(0);
        Some(Refusal {
            reidentified: n("reidentified"),
            corrected: n("corrected"),
        })
    }

    /// The request again, with the answer.
    pub fn answered(args: &Value, answer: Answer) -> Value {
        let mut args = args.clone();
        if let Some(map) = args.as_object_mut() {
            map.insert("changed_rows".into(), Value::from(answer.as_str()));
        }
        args
    }

    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![];
        if self.reidentified > 0 {
            out.push(tc(
                "connections:reimport.refusal.reidentified",
                self.reidentified as usize,
                &[],
            ));
        }
        if self.corrected > 0 {
            out.push(tc(
                "connections:reimport.refusal.corrected",
                self.corrected as usize,
                &[],
            ));
        }
        out
    }
}

/// The refusal with its two answers; `answer` re-sends the request. `sync`: an exchange sync
/// rather than a file import (the rollback advice names the sync).
pub fn refusal_view(
    tk: &Tk,
    key: &str,
    r: &Refusal,
    sync: bool,
    pending: bool,
    answer: impl Fn(&mut App, Answer) + Clone + 'static,
) -> El {
    let mut c = col()
        .key(key.to_owned())
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, tk.border.alpha(tk.border.a * 0.6))
        .border_layout()
        .role("alert")
        .label(t("connections:reimport.refusal.title"))
        .child(
            row()
                .gap(tk.r(0.375))
                .center_y()
                .child(ic(&I::COINS, tk.r(0.875), tk.fg))
                .child(
                    text(t("connections:reimport.refusal.title"))
                        .font(tk.txs(500.0))
                        .color(tk.fg)
                        .wrap()
                        .min_w(0.0),
                ),
        );
    for line in r.lines() {
        c = c.child(xs(tk, line, tk.fg));
    }
    c = c.child(xs(
        tk,
        t(if sync {
            "connections:reimport.refusal.bodySync"
        } else {
            "connections:reimport.refusal.body"
        }),
        tk.muted_fg,
    ));
    let mut buttons = row().w_full().gap(tk.r(0.5)).wrap();
    for (answer_key, which, variant) in [
        ("skip", Answer::Skip, Variant::Secondary),
        ("import", Answer::Import, Variant::Outline),
    ] {
        let mut b = button(
            tk,
            &format!("{key}-{answer_key}"),
            variant,
            true,
            None,
            &t(&format!("connections:reimport.refusal.{answer_key}")),
            !pending,
        );
        if !pending {
            let answer = answer.clone();
            b = b.on_press(move |a: &mut App| answer(a, which));
        }
        buttons = buttons.child(b);
    }
    c.child(buttons)
}

/// A toast body: "<label>: 3 new · 120 already in the book", then the kept values.
pub fn toast_body(label: &str, r: &Value) -> String {
    let mut body = tr(
        "connections:reimport.toast",
        &[("label", label), ("counts", &Counts::of(r).text())],
    );
    let lines: Vec<String> = [changed_line(r), corrections_line(r)]
        .into_iter()
        .flatten()
        .collect();
    if !lines.is_empty() {
        body.push_str(". ");
        body.push_str(&lines.join(" "));
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_missing_updated_count_is_the_updated_records_not_zero() {
        let r = json!({"imported": 2, "unchanged": 5, "updated_records": [{}, {}, {}]});
        assert_eq!(updated(&r), Some(3));
        assert_eq!(
            Counts::of(&r),
            Counts {
                new: 2,
                known: 5,
                updated: 3
            }
        );
        assert_eq!(
            updated(&json!({"updated": 1, "updated_records": []})),
            Some(1)
        );
        assert_eq!(updated(&json!({"imported": 1})), None);
    }

    #[test]
    fn counts_name_new_rows_and_only_nonzero_others() {
        assert_eq!(
            Counts {
                new: 0,
                known: 0,
                updated: 0
            }
            .text(),
            "0 new"
        );
        assert_eq!(
            Counts {
                new: 3,
                known: 120,
                updated: 2
            }
            .text(),
            "3 new · 120 already in the book · 2 updated"
        );
        assert_eq!(
            Counts {
                new: 1,
                known: 0,
                updated: 1
            }
            .text(),
            "1 new · 1 updated"
        );
    }

    #[test]
    fn kept_values_and_answered_rows_are_one_line_each() {
        let r = json!({"corrections_ignored": [{}, {}], "changed_rows": {"action": "skip", "reidentified": 3, "corrected": 1}});
        assert_eq!(
            corrections_line(&r).unwrap(),
            "2 values in the export differ from the book; the book's values were kept."
        );
        assert_eq!(
            changed_line(&r).unwrap(),
            "4 rows kept as the book has them."
        );
        assert_eq!(corrections_line(&json!({})), None);
        assert_eq!(
            changed_line(
                &json!({"changed_rows": {"action": "skip", "reidentified": 0, "corrected": 0}})
            ),
            None
        );
        assert_eq!(
            toast_body(
                "Bitpanda",
                &json!({"imported": 1, "unchanged": 4, "updated_records": []})
            ),
            "Bitpanda: 1 new · 4 already in the book"
        );
    }

    #[test]
    fn only_the_changed_rows_refusal_is_offered_answers() {
        let e =
            json!({"code": "import_changed_rows", "details": {"reidentified": 12, "corrected": 1}});
        let r = Refusal::of(&e).unwrap();
        assert_eq!(
            r,
            Refusal {
                reidentified: 12,
                corrected: 1
            }
        );
        assert_eq!(
            r.lines(),
            [
                "This export lists 12 rows the book already has under another ID.",
                "1 row has the ID of a row the book has, with another amount or fee.",
            ]
        );
        assert_eq!(Refusal::of(&json!({"code": "validation"})), None);
        let args = Refusal::answered(&json!({"wallet": "w"}), Answer::Skip);
        assert_eq!(args, json!({"wallet": "w", "changed_rows": "skip"}));
    }
}
