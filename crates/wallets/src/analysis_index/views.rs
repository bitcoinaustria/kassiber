use super::{field, fields, list, AnalysisIndex, Text};
use crate::database_error;
use kassiber_envelope::{dict, s, set, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{
    params_from_iter, types::Value as Sql, Connection, OptionalExtension,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

const NODE_LOOKAHEAD: usize = 2001;
const EDGE_LOOKAHEAD: usize = 6002;
const LABEL_LIMIT: usize = 2000;

pub trait GraphRead {
    fn public_visibility(&self) -> bool;
    fn get(&self, map: &str, key: &str) -> Result<Value, AppError>;
    fn seeds(&self, query: &Value) -> Result<Vec<String>, AppError>;
    fn findings_for_nodes(&self, nodes: &BTreeSet<String>) -> Result<Vec<Value>, AppError>;
    fn coverage(&self) -> Result<Value, AppError>;
    fn snapshot_id(&self) -> Value;
    fn has_spend(&self, id: &str) -> Result<bool, AppError> {
        for edge in list(&self.get("outgoing", id)?) {
            if field(
                &self.get("edges", edge.as_str().ok_or_else(super::invalid)?)?,
                "kind",
            ) == &s("spends")
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl GraphRead for AnalysisIndex {
    fn public_visibility(&self) -> bool {
        field(field(self.snapshot(), "coverage"), "observer_knowledge") == &s("public_chain_facts")
    }
    fn get(&self, map: &str, key: &str) -> Result<Value, AppError> {
        Ok(self.lookup(map, key).cloned().unwrap_or(Value::Null))
    }
    fn seeds(&self, query: &Value) -> Result<Vec<String>, AppError> {
        Ok(list(field(self.snapshot(), "profile_seeds"))
            .iter()
            .filter_map(Value::as_str)
            .filter(|id| {
                self.lookup("nodes", id)
                    .is_some_and(|n| super::traversal::domain(n, query))
            })
            .map(str::to_owned)
            .collect())
    }
    fn findings_for_nodes(&self, nodes: &BTreeSet<String>) -> Result<Vec<Value>, AppError> {
        Ok(list(field(self.snapshot(), "findings"))
            .iter()
            .filter(|f| {
                !truthy(field(f, "node_ids"))
                    || list(field(f, "node_ids"))
                        .iter()
                        .any(|id| id.as_str().is_some_and(|id| nodes.contains(id)))
            })
            .cloned()
            .collect())
    }
    fn coverage(&self) -> Result<Value, AppError> {
        Ok(field(self.snapshot(), "coverage").clone())
    }
    fn snapshot_id(&self) -> Value {
        field(self.snapshot(), "snapshot_id").clone()
    }
}

pub struct SqlView<'a> {
    conn: &'a Connection,
    profile: &'a str,
    state: Value,
    public: bool,
    confirmations: RefCell<super::confirmations::ConfirmationOverlay<'a>>,
}

pub fn with_read<R>(
    conn: &Connection,
    profile: &str,
    observer: &str,
    read: impl for<'v> FnOnce(&SqlView<'v>) -> Result<R, AppError>,
) -> Result<R, AppError> {
    let outermost = conn.is_autocommit();
    conn.execute_batch("SAVEPOINT chain_analysis_view_read")
        .map_err(database_error)?;
    let result = (|| {
        let state = crate::publication::row::rows(
            conn,
            "SELECT * FROM chain_index_state WHERE profile_id=?",
            [profile],
        )?
        .into_iter()
        .next()
        .ok_or_else(|| {
            AppError::new(
                "chain_analysis_index_unavailable",
                "Synchronize the local analysis index before reading it",
            )
        })?;
        if field(&state, "version") != &Value::Int(super::durable::VERSION.into()) {
            return Err(AppError::new(
                "chain_analysis_index_unavailable",
                "Rebuild the local analysis index before reading it",
            ));
        }
        let view = SqlView::new(conn, profile, state, observer == "public")?;
        read(&view)
    })();
    kassiber_store::debug_assert_no_write(conn, outermost, "chain_analysis_view_read");
    let released = conn
        .execute_batch("RELEASE chain_analysis_view_read")
        .map_err(database_error);
    match result {
        Ok(v) => {
            released?;
            Ok(v)
        }
        Err(e) => {
            released?;
            Err(e)
        }
    }
}

impl<'a> SqlView<'a> {
    /// The synchronized `chain_index_state` row this view reads.
    pub fn state(&self) -> &Value {
        &self.state
    }
    pub(super) fn book_scope(&self) -> Result<super::handles::BookScope, AppError> {
        super::handles::BookScope::read(self.conn, self.profile)
    }
    fn new(
        conn: &'a Connection,
        profile: &'a str,
        state: Value,
        public: bool,
    ) -> Result<Self, AppError> {
        Ok(Self {
            conn,
            profile,
            state,
            public,
            confirmations: RefCell::new(super::confirmations::ConfirmationOverlay::read(
                conn, profile,
            )?),
        })
    }
    pub fn for_observer(&self, observer: &str) -> Result<SqlView<'_>, AppError> {
        SqlView::new(
            self.conn,
            self.profile,
            self.state.clone(),
            self.public || observer == "public",
        )
    }
    fn column(&self) -> &'static str {
        if self.public {
            "public_json"
        } else {
            "payload_json"
        }
    }
    fn visibility(&self) -> &'static str {
        if self.public {
            "public"
        } else {
            "owner"
        }
    }
    fn label_visibility(&self) -> &'static str {
        if self.public {
            " AND json_extract(payload_json,'$.visibility')='public' AND json_extract(payload_json,'$.dataset_id') IS NOT NULL"
        } else {
            ""
        }
    }
    fn strings(
        &self,
        sql: &str,
        params: impl kassiber_store::rusqlite::Params,
    ) -> Result<Vec<String>, AppError> {
        self.conn
            .prepare(sql)
            .map_err(database_error)?
            .query_map(params, |r| r.get(0))
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)
    }
    fn count(&self, sql: &str) -> Result<usize, AppError> {
        self.conn
            .query_row(sql, [self.profile], |r| r.get::<_, i64>(0))
            .map(|v| v as usize)
            .map_err(database_error)
    }
    pub fn has_subject(&self, subject: &str) -> Result<bool, AppError> {
        let alias = self.conn.query_row("SELECT 1 FROM chain_index_aliases WHERE profile_id=? AND observer=? AND alias=? LIMIT 1",[self.profile,self.visibility(),subject], |_| Ok(())).optional().map_err(database_error)?.is_some();
        if alias {
            return Ok(true);
        }
        self.conn.query_row(&format!("SELECT 1 FROM chain_index_labels WHERE profile_id=? AND json_extract(payload_json,'$.subject')=?{} LIMIT 1",self.label_visibility()),[self.profile,subject], |_| Ok(())).optional().map(|v|v.is_some()).map_err(database_error)
    }
    fn related(&self, findings: bool, nodes: &BTreeSet<String>) -> Result<Vec<Value>, AppError> {
        if nodes.len() > 2000 {
            return Err(super::invalid());
        }
        let (table, link, key, column) = if findings {
            (
                "chain_index_findings",
                "chain_index_finding_nodes",
                "finding_id",
                self.column(),
            )
        } else {
            (
                "chain_index_labels",
                "chain_index_label_nodes",
                "label_id",
                "payload_json",
            )
        };
        let mut found = BTreeMap::new();
        let ids: Vec<_> = nodes.iter().collect();
        for part in ids.chunks(200) {
            let marks = vec!["?"; part.len()].join(",");
            let visibility = if !findings && self.public {
                " AND json_extract(f.payload_json,'$.visibility')='public' AND json_extract(f.payload_json,'$.dataset_id') IS NOT NULL"
            } else {
                ""
            };
            let limit = if findings {
                " LIMIT 10001".to_owned()
            } else {
                format!(" LIMIT {LABEL_LIMIT}")
            };
            let sql = format!("SELECT DISTINCT f.id,f.{column} FROM {table} f JOIN {link} n ON n.profile_id=f.profile_id AND n.{key}=f.id WHERE f.profile_id=? AND f.{column} IS NOT NULL AND n.node_id IN ({marks}){visibility} ORDER BY f.id{limit}");
            let params = std::iter::once(Sql::Text(self.profile.to_owned()))
                .chain(part.iter().map(|id| Sql::Text((***id).to_owned())));
            let mut stmt = self.conn.prepare(&sql).map_err(database_error)?;
            for row in stmt
                .query_map(params_from_iter(params), |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(database_error)?
            {
                let (id, payload) = row.map_err(database_error)?;
                found.insert(id, parse(&payload)?);
                while !findings && found.len() > LABEL_LIMIT {
                    found.pop_last();
                }
                if findings && found.len() > 10000 {
                    return Err(AppError::new(
                        "chain_analysis_budget",
                        "Findings exceed the local analysis lookup limit",
                    ));
                }
            }
        }
        if findings {
            let sql = format!("SELECT id,{} FROM chain_index_findings WHERE profile_id=? AND {} IS NOT NULL AND json_array_length(json_extract(payload_json,'$.node_ids'))=0 ORDER BY id LIMIT 10001",self.column(),self.column());
            let mut stmt = self.conn.prepare(&sql).map_err(database_error)?;
            for row in stmt
                .query_map([self.profile], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .map_err(database_error)?
            {
                let (id, payload) = row.map_err(database_error)?;
                found.insert(id, parse(&payload)?);
            }
        }
        if findings && found.len() > 10000 {
            return Err(AppError::new(
                "chain_analysis_budget",
                "Findings exceed the local analysis lookup limit",
            ));
        }
        let values = found.into_values();
        Ok(if findings {
            values.collect()
        } else {
            values.take(LABEL_LIMIT).collect()
        })
    }
    pub fn label_count(&self) -> Result<usize, AppError> {
        self.count(&format!(
            "SELECT count(*) FROM chain_index_labels WHERE profile_id=?{}",
            self.label_visibility()
        ))
    }
    pub fn available_transaction_count(&self) -> Result<usize, AppError> {
        self.count(&format!("SELECT count(*) FROM chain_index_nodes WHERE profile_id=? AND {} IS NOT NULL AND kind='transaction' AND status!='missing'",self.column()))
    }
    pub fn owned_output_count(&self) -> Result<usize, AppError> {
        if self.public {
            return Ok(0);
        }
        self.count(&format!(
            "SELECT count(*) FROM chain_index_nodes WHERE profile_id=? AND {OWNED}"
        ))
    }
    pub fn owned_domain_count(&self) -> Result<usize, AppError> {
        if self.public {
            return Ok(0);
        }
        self.count(&format!("SELECT count(*) FROM (SELECT DISTINCT json_extract(payload_json,'$.chain'),json_extract(payload_json,'$.network') FROM chain_index_nodes WHERE profile_id=? AND public_json IS NOT NULL AND {OWNED})"))
    }
    pub fn owned_outputs_for_nodes(
        &self,
        nodes: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>, AppError> {
        if self.public {
            return Ok(BTreeSet::new());
        }
        let mut found = BTreeSet::new();
        let ids: Vec<_> = nodes.iter().collect();
        for part in ids.chunks(200) {
            let marks = vec!["?"; part.len()].join(",");
            let params = std::iter::once(Sql::Text(self.profile.to_owned()))
                .chain(part.iter().map(|id| Sql::Text((***id).to_owned())));
            found.extend(self.strings(&format!("SELECT id FROM chain_index_nodes WHERE profile_id=? AND id IN ({marks}) AND {OWNED}"),params_from_iter(params))?);
        }
        Ok(found)
    }
    pub fn selected_index(
        &self,
        nodes: &BTreeSet<String>,
        edge_ids: &BTreeSet<String>,
    ) -> Result<AnalysisIndex, AppError> {
        if nodes.len() > 2000 || edge_ids.len() > 6000 {
            return Err(super::invalid());
        }
        let mut label_nodes = BTreeSet::new();
        for id in nodes {
            let node = self.get("nodes", id)?;
            let chain = field(&node, "chain").as_str().unwrap_or("");
            let network = field(&node, "network").as_str().unwrap_or("");
            let domain = if chain == "lightning" {
                ["main", "test", "signet", "regtest"].contains(&network)
            } else {
                ["bitcoin", "liquid"].contains(&chain)
                    && crate::descriptors::normalize_network(chain, network)
                        .ok()
                        .as_deref()
                        == Some(network)
            };
            if domain
                && !["stale", "conflicting", "retracted"]
                    .contains(&field(&node, "status").as_str().unwrap_or(""))
            {
                label_nodes.insert(id.clone());
            }
        }
        let mut result = dict([
            ("snapshot_id", self.snapshot_id()),
            ("coverage", self.coverage()?),
            ("findings", Value::List(self.findings_for_nodes(nodes)?)),
            ("labels", Value::List(self.related(false, &label_nodes)?)),
            (
                "profile_seeds",
                Value::List(
                    self.seeds(&dict([] as [(&str, Value); 0]))?
                        .into_iter()
                        .map(s)
                        .collect(),
                ),
            ),
        ]);
        for map in [
            "nodes",
            "transaction_facts",
            "output_facts",
            "outgoing",
            "incoming",
            "subjects",
            "edges",
        ] {
            set(&mut result, map, dict([] as [(&str, Value); 0]));
        }
        for id in nodes {
            for map in ["nodes", "transaction_facts", "output_facts"] {
                let value = self.get(map, id)?;
                if value != Value::Null {
                    let mut mapping = field(&result, map).clone();
                    set(&mut mapping, id, value);
                    set(&mut result, map, mapping);
                }
            }
        }
        for id in edge_ids {
            let value = self.get("edges", id)?;
            if value == Value::Null
                || !nodes.contains(field(&value, "source").as_str().unwrap_or(""))
                || !nodes.contains(field(&value, "target").as_str().unwrap_or(""))
            {
                continue;
            }
            let mut edges = field(&result, "edges").clone();
            set(&mut edges, id, value.clone());
            set(&mut result, "edges", edges);
            for (map, key) in [("outgoing", "source"), ("incoming", "target")] {
                let key = field(&value, key).as_str().ok_or_else(super::invalid)?;
                let mut mapping = field(&result, map).clone();
                let mut ids = list(field(&mapping, key)).to_vec();
                ids.push(s(id.as_str()));
                set(&mut mapping, key, Value::List(ids));
                set(&mut result, map, mapping);
            }
        }
        AnalysisIndex::from_snapshot(result)
    }
}
const OWNED: &str = "json_extract(output_json,'$.ownership_known')=1 AND coalesce(json_extract(output_json,'$.ownership_ambiguous'),0)=0 AND json_array_length(json_extract(payload_json,'$.wallet_ids'))=1 AND status NOT IN ('missing','stale','conflicting')";
fn parse(text: &str) -> Result<Value, AppError> {
    kassiber_pytext::loads(text).map_err(|_| super::invalid())
}

impl GraphRead for SqlView<'_> {
    fn public_visibility(&self) -> bool {
        self.public
    }
    fn get(&self, map: &str, key: &str) -> Result<Value, AppError> {
        if map == "subjects" {
            let ids=self.strings(&format!("SELECT node_id FROM chain_index_aliases WHERE profile_id=? AND observer=? AND alias=? ORDER BY node_id LIMIT {}",NODE_LOOKAHEAD+1),[self.profile,self.visibility(),key])?;
            if ids.len() > NODE_LOOKAHEAD {
                return Err(AppError::new(
                    "chain_analysis_budget",
                    "Subject exceeds the local analysis lookup limit",
                ));
            }
            return Ok(Value::List(ids.into_iter().map(s).collect()));
        }
        if ["outgoing", "incoming"].contains(&map) {
            let direction = if map == "outgoing" {
                "source"
            } else {
                "target"
            };
            let ids=self.strings(&format!("SELECT id FROM chain_index_edges WHERE profile_id=? AND {direction}=? AND {} IS NOT NULL ORDER BY id LIMIT {EDGE_LOOKAHEAD}",self.column()),[self.profile,key])?;
            return Ok(Value::List(ids.into_iter().map(s).collect()));
        }
        let (table, column) = match map {
            "nodes" => ("chain_index_nodes", self.column()),
            "edges" => ("chain_index_edges", self.column()),
            "transaction_facts" => (
                "chain_index_nodes",
                if self.public {
                    "public_tx_json"
                } else {
                    "tx_json"
                },
            ),
            "output_facts" => (
                "chain_index_nodes",
                if self.public {
                    "public_output_json"
                } else {
                    "output_json"
                },
            ),
            _ => return Err(super::invalid()),
        };
        let payload = self
            .conn
            .query_row(
                &format!("SELECT {column} FROM {table} WHERE profile_id=? AND id=?"),
                [self.profile, key],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(database_error)?
            .flatten();
        let Some(payload) = payload else {
            return Ok(Value::Null);
        };
        let value = parse(&payload)?;
        if map == "nodes" {
            self.confirmations.borrow_mut().apply(&value)
        } else {
            Ok(value)
        }
    }
    fn seeds(&self, query: &Value) -> Result<Vec<String>, AppError> {
        let sql=format!("SELECT id,json_extract(payload_json,'$.chain'),json_extract(payload_json,'$.network') FROM chain_index_nodes WHERE profile_id=? AND seed=1 AND {} IS NOT NULL ORDER BY id",self.column());
        let mut stmt = self.conn.prepare(&sql).map_err(database_error)?;
        let mut result = Vec::new();
        for row in stmt
            .query_map([self.profile], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(database_error)?
        {
            let (id, chain, network) = row.map_err(database_error)?;
            if super::traversal::domain(
                &dict([("chain", s(chain)), ("network", s(network))]),
                query,
            ) {
                result.push(id);
                if result.len() >= NODE_LOOKAHEAD {
                    break;
                }
            }
        }
        Ok(result)
    }
    fn findings_for_nodes(&self, nodes: &BTreeSet<String>) -> Result<Vec<Value>, AppError> {
        self.related(true, nodes)
    }
    fn coverage(&self) -> Result<Value, AppError> {
        let mut value = parse(
            field(&self.state, "coverage_json")
                .as_str()
                .ok_or_else(super::invalid)?,
        )?;
        if self.public {
            let mut datasets = field(&value, "datasets").clone();
            if matches!(datasets, Value::Dict(_)) {
                let partitions = field(&datasets, "visibility_coverage").clone();
                super::remove(&mut datasets, "visibility_coverage");
                for (key, v) in fields(field(&partitions, "public")) {
                    set(&mut datasets, key, v.clone());
                }
                set(&mut value, "datasets", datasets);
            }
            for (key,v) in [("observer_knowledge",s("public_chain_facts")),("hidden_private_node_count",Value::Int(self.count("SELECT count(*) FROM chain_index_nodes WHERE profile_id=? AND public_json IS NULL")?.into())),("hidden_private_relation_count",Value::Int(self.count("SELECT count(*) FROM chain_index_edges WHERE profile_id=? AND public_json IS NULL")?.into())),("public_liquid_value_policy",s("explicit_public_values_only")),("private_labels_withheld",Value::Bool(true))] {set(&mut value,key,v);}
        }
        Ok(value)
    }
    fn snapshot_id(&self) -> Value {
        field(&self.state, "snapshot_id").clone()
    }
    fn has_spend(&self, id: &str) -> Result<bool, AppError> {
        self.conn.query_row(&format!("SELECT 1 FROM chain_index_edges WHERE profile_id=? AND source=? AND {} IS NOT NULL AND json_extract(payload_json,'$.kind')='spends' LIMIT 1",self.column()),[self.profile,id],|_|Ok(())).optional().map(|v|v.is_some()).map_err(database_error)
    }
}

#[cfg(any(test, feature = "test-harness"))]
pub fn evaluate_fixture(case: &Value, conn: &Connection) -> Result<Value, AppError> {
    use super::traversal::query_read;
    let profile = field(case, "profile").as_str().unwrap_or("p");
    with_read(
        conn,
        profile,
        field(case, "observer").as_str().unwrap_or("owner"),
        |initial| {
            let view = initial.for_observer(if truthy(field(case, "upgrade")) {
                "owner"
            } else {
                field(case, "observer").as_str().unwrap_or("owner")
            })?;
            if kassiber_envelope::get(case, "query").is_some() {
                return Ok(query_read(&view, field(case, "query")).unwrap_or_else(|e| {
                    dict([(
                        "error",
                        dict([
                            ("code", s(e.code)),
                            ("message", s(e.message)),
                            ("details", e.details.unwrap_or(Value::Null)),
                            ("retryable", Value::Bool(e.retryable)),
                        ]),
                    )])
                }));
            }
            let mut result = dict([
                ("snapshot_id", view.snapshot_id()),
                ("coverage", view.coverage()?),
                (
                    "seeds",
                    Value::List(
                        view.seeds(field(case, "seed_query"))?
                            .into_iter()
                            .map(s)
                            .collect(),
                    ),
                ),
            ]);
            let ids=view.strings(&format!("SELECT id FROM chain_index_nodes WHERE profile_id=? AND {} IS NOT NULL ORDER BY id",view.column()),[profile])?;
            let nodes = ids.into_iter().collect::<BTreeSet<_>>();
            for (map, key, table, column) in [
                ("nodes", "nodes", "chain_index_nodes", view.column()),
                ("edges", "edges", "chain_index_edges", view.column()),
                (
                    "facts",
                    "transaction_facts",
                    "chain_index_nodes",
                    if view.public {
                        "public_tx_json"
                    } else {
                        "tx_json"
                    },
                ),
                (
                    "outputs",
                    "output_facts",
                    "chain_index_nodes",
                    if view.public {
                        "public_output_json"
                    } else {
                        "output_json"
                    },
                ),
            ] {
                let ids=view.strings(&format!("SELECT id FROM {table} WHERE profile_id=? AND {column} IS NOT NULL ORDER BY id"),[profile])?;
                let mut mapping = dict([] as [(&str, Value); 0]);
                for id in ids {
                    set(&mut mapping, &id, view.get(key, &id)?);
                }
                set(&mut result, map, mapping);
            }
            let edge_ids=view.strings(&format!("SELECT id FROM chain_index_edges WHERE profile_id=? AND {} IS NOT NULL ORDER BY id",view.column()),[profile])?.into_iter().collect();
            let captured = view.selected_index(&nodes, &edge_ids)?;
            let query = dict([
                (
                    "observer",
                    s(field(case, "observer").as_str().unwrap_or("owner")),
                ),
                ("include_hypotheses", Value::Bool(true)),
                (
                    "_subject_node_ids",
                    Value::List(
                        view.seeds(&dict([] as [(&str, Value); 0]))?
                            .into_iter()
                            .map(s)
                            .collect(),
                    ),
                ),
                (
                    "_selected_edge_ids",
                    Value::List(edge_ids.iter().map(|v: &String| s(v.as_str())).collect()),
                ),
            ]);
            set(
                &mut result,
                "analytics",
                super::analytics::analyze_index(&captured, &query, &nodes)?,
            );
            let mut subjects = dict([] as [(&str, Value); 0]);
            for alias in view.strings("SELECT DISTINCT alias FROM chain_index_aliases WHERE profile_id=? AND observer=? ORDER BY alias",[profile,view.visibility()])? {set(&mut subjects,&alias,view.get("subjects",&alias)?);}
            set(&mut result, "subjects", subjects);
            set(
                &mut result,
                "findings",
                Value::List(view.findings_for_nodes(&nodes)?),
            );
            set(
                &mut result,
                "labels",
                Value::List(view.related(false, &nodes)?),
            );
            set(
                &mut result,
                "label_count",
                Value::Int(view.label_count()?.into()),
            );
            set(
                &mut result,
                "available",
                Value::Int(view.available_transaction_count()?.into()),
            );
            set(
                &mut result,
                "owned_count",
                Value::Int(view.owned_output_count()?.into()),
            );
            set(
                &mut result,
                "domain_count",
                Value::Int(view.owned_domain_count()?.into()),
            );
            set(
                &mut result,
                "owned",
                Value::List(
                    view.owned_outputs_for_nodes(&nodes)?
                        .into_iter()
                        .map(s)
                        .collect(),
                ),
            );
            let mut subjects = vec![
                s("missing"),
                s("a".repeat(64)),
                s(format!("script:0014{}", "22".repeat(20))),
            ];
            subjects.extend(list(field(case, "subjects")).iter().cloned());
            let mut found = dict([] as [(&str, Value); 0]);
            for subject in subjects {
                let subject = subject.as_str().unwrap();
                set(&mut found, subject, Value::Bool(view.has_subject(subject)?));
            }
            set(&mut result, "has_subject", found);
            Ok(result)
        },
    )
}
