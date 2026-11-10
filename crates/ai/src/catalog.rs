//! `TOOL_CATALOG` (`kassiber/ai/tools.py`): every definition the in-product
//! assistant can be offered, embedded from `data/catalog.json`, which
//! `scripts/gen_catalog.py` writes from Python. Schemas keep their literal
//! key order, because the provider sees them as written.

use std::collections::HashMap;
use std::sync::OnceLock;

use kassiber_envelope::{dict, get, s};
use kassiber_pyjson::{Key, PyStr, Value};

const CATALOG_JSON: &str = include_str!("data/catalog.json");

/// `ToolKindClass`: does the tool change the book?
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KindClass {
    ReadOnly,
    Mutating,
}

impl KindClass {
    pub fn as_str(self) -> &'static str {
        match self {
            KindClass::ReadOnly => "read_only",
            KindClass::Mutating => "mutating",
        }
    }
}

/// `ToolEntry`.
#[derive(Clone, Debug)]
pub struct ToolEntry {
    pub name: String,
    pub wire_name: Option<String>,
    pub description: String,
    pub parameters: Value,
    pub kind_class: KindClass,
    pub daemon_kind: Option<String>,
    pub summary_template: Option<String>,
    /// Independent of `kind_class`: does the tool leave the machine?
    pub egresses: bool,
    /// `REVERSIBLE_TOOLS`: another AI tool can take the change back, so the
    /// `auto` permission mode may approve it without asking.
    pub reversible: bool,
}

impl ToolEntry {
    /// `provider_name`: the name the model sees.
    pub fn provider_name(&self) -> &str {
        self.wire_name.as_deref().unwrap_or(&self.name)
    }

    /// `requires_consent`: it mutates or egresses.
    pub fn requires_consent(&self) -> bool {
        self.kind_class == KindClass::Mutating || self.egresses
    }

    /// `to_responses_tool()`.
    pub fn to_responses_tool(&self) -> Value {
        dict([
            ("type", s("function")),
            ("name", s(self.provider_name())),
            ("description", s(self.description.as_str())),
            ("parameters", self.parameters.clone()),
            // Kassiber's optional-argument semantics: an omitted strict flag
            // may be resolved as strict by the Responses API.
            ("strict", Value::Bool(false)),
        ])
    }
}

struct Catalog {
    tools: Vec<ToolEntry>,
    /// `LOAD_TOOLS_ENTRY`: resolvable by name, never in `TOOL_CATALOG`.
    load_tools: ToolEntry,
    by_name: HashMap<String, usize>,
    skill_reference_index: String,
}

fn text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Str(text)) => Some(text.as_str().expect("catalog text is UTF-8").to_owned()),
        _ => None,
    }
}

fn entry(entry: &Value) -> ToolEntry {
    ToolEntry {
        name: text(get(entry, "name")).expect("tool name"),
        wire_name: text(get(entry, "wire_name")),
        description: text(get(entry, "description")).expect("tool description"),
        parameters: get(entry, "parameters").cloned().expect("tool parameters"),
        kind_class: match text(get(entry, "kind_class")).as_deref() {
            Some("read_only") => KindClass::ReadOnly,
            Some("mutating") => KindClass::Mutating,
            other => panic!("unknown kind_class {other:?}"),
        },
        daemon_kind: text(get(entry, "daemon_kind")),
        summary_template: text(get(entry, "summary_template")),
        egresses: matches!(get(entry, "egresses"), Some(Value::Bool(true))),
        reversible: matches!(get(entry, "reversible"), Some(Value::Bool(true))),
    }
}

/// The index `TOOL_BY_NAME` gives `LOAD_TOOLS_ENTRY`.
const LOAD_TOOLS_INDEX: usize = usize::MAX;

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let document = kassiber_pytext::loads(CATALOG_JSON).expect("embedded catalog parses");
        let Some(Value::List(entries)) = get(&document, "tools") else {
            panic!("embedded catalog has no tools");
        };
        let tools: Vec<ToolEntry> = entries.iter().map(entry).collect();
        let load_tools = entry(get(&document, "load_tools").expect("load_tools entry"));
        // `TOOL_BY_NAME`: name, then provider name, in catalog order (a
        // later entry wins), then `load_tools`, then the legacy blockers alias.
        let mut by_name = HashMap::new();
        for (index, tool) in tools.iter().enumerate() {
            by_name.insert(tool.name.clone(), index);
            by_name.insert(tool.provider_name().to_owned(), index);
        }
        by_name.insert(load_tools.name.clone(), LOAD_TOOLS_INDEX);
        if let Some(Value::Dict(aliases)) = get(&document, "aliases") {
            for (alias, target) in aliases {
                let (Key::Str(alias), Value::Str(target)) = (alias, target) else {
                    panic!("catalog alias is not text");
                };
                let index = by_name[target.as_str().expect("alias target")];
                by_name.insert(alias.as_str().expect("alias").to_owned(), index);
            }
        }
        Catalog {
            tools,
            load_tools,
            by_name,
            skill_reference_index: text(get(&document, "skill_reference_index"))
                .expect("skill reference index"),
        }
    })
}

/// `TOOL_CATALOG`, in order.
pub fn tools() -> &'static [ToolEntry] {
    &catalog().tools
}

/// `get_tool(name)`: by name, provider name, or the one legacy alias.
pub fn get_tool(name: &str) -> Option<&'static ToolEntry> {
    let catalog = catalog();
    catalog.by_name.get(name).map(|&index| match index {
        LOAD_TOOLS_INDEX => &catalog.load_tools,
        index => &catalog.tools[index],
    })
}

/// `LOAD_TOOLS_ENTRY`.
pub fn load_tools_entry() -> &'static ToolEntry {
    &catalog().load_tools
}

/// `get_tool` for a model-supplied name that may hold surrogates (no tool
/// name does).
pub fn get_tool_py(name: &PyStr) -> Option<&'static ToolEntry> {
    name.as_str().and_then(get_tool)
}

/// `SKILL_REFERENCE_INDEX`.
pub(crate) fn skill_reference_index() -> &'static str {
    &catalog().skill_reference_index
}

/// The embedded document, for drift tests.
#[doc(hidden)]
pub fn catalog_document() -> Value {
    kassiber_pytext::loads(CATALOG_JSON).expect("embedded catalog parses")
}
