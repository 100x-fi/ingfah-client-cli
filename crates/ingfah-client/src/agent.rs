//! Agent revisions as full snapshots: building a create body from the newest revision,
//! guarding tool links, diffing, and checking the read-back.
//!
//! A revision stores exactly what it is sent. An omitted tool list means no tools, and
//! publishing replaces the agent's tool links with the revision's, so every body built
//! here carries the complete tool lists and voice.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::Deserialize;
use serde_json::{Map, Value};

/// The agent's channel. Decides which tool lists and voice fields apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentType {
    /// Voice agent.
    Audio,
    /// Text chat agent.
    Text,
}

impl AgentType {
    /// Reads `type` from an agent detail; anything but `text` is audio, the server default.
    pub fn of(agent: &Value) -> Self {
        match agent.get("type").and_then(Value::as_str) {
            Some("text") => Self::Text,
            _ => Self::Audio,
        }
    }
}

/// The three kinds of tool an agent links to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolKind {
    /// Plugin function (`ai_plugin_function_ids`).
    Plugin,
    /// Phone tool (`phone_tools`), audio agents only.
    Phone,
    /// Text chat tool (`text_chat_tools`), text agents only.
    TextChat,
}

impl ToolKind {
    /// Every kind.
    pub const ALL: [Self; 3] = [Self::Plugin, Self::Phone, Self::TextChat];

    /// The id-list key in a create-revision body.
    pub fn body_key(self) -> &'static str {
        match self {
            Self::Plugin => "ai_plugin_function_ids",
            Self::Phone => "phone_tools",
            Self::TextChat => "text_chat_tools",
        }
    }

    /// The object-list key in an agent detail or a created revision.
    pub fn detail_key(self) -> &'static str {
        match self {
            Self::Plugin => "ai_plugin_functions",
            Self::Phone => "phone_tools",
            Self::TextChat => "text_chat_tools",
        }
    }

    /// The key under a flow node's `guidelines` that references this kind.
    pub fn flow_key(self) -> Option<&'static str> {
        match self {
            Self::Plugin => Some("plugin_functions"),
            Self::Phone => Some("phone_tools"),
            Self::TextChat => None,
        }
    }

    /// Short name used on the command line (`plugin:12`).
    pub fn label(self) -> &'static str {
        match self {
            Self::Plugin => "plugin",
            Self::Phone => "phone",
            Self::TextChat => "text",
        }
    }

    /// Whether the server keeps this list for the agent type.
    pub fn applies_to(self, agent_type: AgentType) -> bool {
        match self {
            Self::Plugin => true,
            Self::Phone => agent_type == AgentType::Audio,
            Self::TextChat => agent_type == AgentType::Text,
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "plugin" | "plugin-function" | "plugin_function" => Some(Self::Plugin),
            "phone" | "phone-tool" | "phone_tool" => Some(Self::Phone),
            "text" | "text-chat" | "text-chat-tool" | "text_chat_tool" => Some(Self::TextChat),
            _ => None,
        }
    }
}

/// One tool, as named on the command line: `KIND:ID`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolRef {
    /// Kind.
    pub kind: ToolKind,
    /// Tool id.
    pub id: u64,
}

impl std::str::FromStr for ToolRef {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind, id) = s
            .split_once(':')
            .ok_or_else(|| format!("expected KIND:ID (plugin|phone|text), got `{s}`"))?;
        let kind = ToolKind::parse(kind)
            .ok_or_else(|| format!("unknown tool kind `{kind}`; use plugin, phone or text"))?;
        let id = id
            .parse()
            .map_err(|_| format!("tool id `{id}` is not a number"))?;
        Ok(Self { kind, id })
    }
}

impl fmt::Display for ToolRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind.label(), self.id)
    }
}

/// Tool ids per kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolIds(BTreeMap<ToolKind, BTreeSet<u64>>);

#[derive(Deserialize)]
struct IdOnly {
    #[serde(default)]
    id: u64,
}

impl ToolIds {
    /// Reads `ai_plugin_functions[].id`, `phone_tools[].id` and `text_chat_tools[].id`
    /// from an agent detail or a created revision. Absent lists are empty.
    pub fn from_detail(detail: &Value) -> Self {
        let mut ids = Self::default();
        for kind in ToolKind::ALL {
            let items: Vec<IdOnly> = detail
                .get(kind.detail_key())
                .cloned()
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
            for item in items {
                ids.insert(ToolRef { kind, id: item.id });
            }
        }
        ids
    }

    /// The ids of one kind.
    pub fn get(&self, kind: ToolKind) -> BTreeSet<u64> {
        self.0.get(&kind).cloned().unwrap_or_default()
    }

    /// Adds one id.
    pub fn insert(&mut self, tool: ToolRef) {
        self.0.entry(tool.kind).or_default().insert(tool.id);
    }

    /// Removes one id; returns whether it was there.
    pub fn remove(&mut self, tool: ToolRef) -> bool {
        self.0
            .get_mut(&tool.kind)
            .is_some_and(|set| set.remove(&tool.id))
    }

    /// Every id in either set.
    pub fn union(&self, other: &Self) -> Self {
        let mut out = self.clone();
        for tool in other.iter() {
            out.insert(tool);
        }
        out
    }

    /// Every id, as tool refs.
    pub fn iter(&self) -> impl Iterator<Item = ToolRef> + '_ {
        self.0
            .iter()
            .flat_map(|(&kind, ids)| ids.iter().map(move |&id| ToolRef { kind, id }))
    }

    /// Keeps only the kinds that apply to the agent type.
    pub fn applicable(&self, agent_type: AgentType) -> Self {
        Self(
            self.0
                .iter()
                .filter(|(k, _)| k.applies_to(agent_type))
                .map(|(k, v)| (*k, v.clone()))
                .collect(),
        )
    }

    /// `+kind:id` for ids only in `after`, `-kind:id` for ids only in `self`.
    pub fn changes_to(&self, after: &Self) -> Vec<String> {
        let removed = self
            .iter()
            .filter(|t| !after.get(t.kind).contains(&t.id))
            .map(|t| format!("-{t}"));
        let added = after
            .iter()
            .filter(|t| !self.get(t.kind).contains(&t.id))
            .map(|t| format!("+{t}"));
        removed.chain(added).collect()
    }
}

/// Why the tool guard refused a revision body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardViolation {
    /// A tool list that applies to the agent is absent; publishing would unbind every tool of that kind.
    MissingList(ToolKind),
    /// A tool bound to the agent now is not in the body.
    DroppedTool(ToolRef),
    /// A flow node names a tool that the top-level lists do not link.
    UnboundFlowRef {
        /// Flow node id.
        node: String,
        /// The referenced tool.
        tool: ToolRef,
    },
}

impl fmt::Display for GuardViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingList(kind) => write!(
                f,
                "`{}` is missing; publishing would unbind every {} tool",
                kind.body_key(),
                kind.label()
            ),
            Self::DroppedTool(tool) => {
                write!(
                    f,
                    "currently bound tool {tool} is not in `{}`",
                    tool.kind.body_key()
                )
            }
            Self::UnboundFlowRef { node, tool } => write!(
                f,
                "flow node `{node}` references {tool}, which is not in `{}`",
                tool.kind.body_key()
            ),
        }
    }
}

fn body_ids(body: &Value, kind: ToolKind) -> Option<BTreeSet<u64>> {
    body.get(kind.body_key())?
        .as_array()?
        .iter()
        .map(Value::as_u64)
        .collect()
}

/// Checks a create-revision body against the tools bound to the agent now.
/// `bound` should already exclude any tool the user removed by name.
pub fn check_revision_body(
    agent_type: AgentType,
    bound: &ToolIds,
    body: &Value,
) -> Vec<GuardViolation> {
    let mut violations = Vec::new();
    let mut listed = ToolIds::default();
    for kind in ToolKind::ALL
        .into_iter()
        .filter(|k| k.applies_to(agent_type))
    {
        match body_ids(body, kind) {
            Some(ids) => {
                for id in ids {
                    listed.insert(ToolRef { kind, id });
                }
            }
            None => violations.push(GuardViolation::MissingList(kind)),
        }
    }
    for tool in bound.applicable(agent_type).iter() {
        if body_ids(body, tool.kind).is_some() && !listed.get(tool.kind).contains(&tool.id) {
            violations.push(GuardViolation::DroppedTool(tool));
        }
    }
    let nodes = body
        .get("ai_instruction_flow")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for node in nodes {
        let name = node.get("id").and_then(Value::as_str).unwrap_or("?");
        for kind in ToolKind::ALL {
            let Some(key) = kind.flow_key() else { continue };
            let refs = node
                .pointer(&format!("/guidelines/{key}"))
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            for id in refs
                .iter()
                .filter_map(|r| r.get("id").and_then(Value::as_u64))
            {
                if !listed.get(kind).contains(&id) {
                    violations.push(GuardViolation::UnboundFlowRef {
                        node: name.to_owned(),
                        tool: ToolRef { kind, id },
                    });
                }
            }
        }
    }
    violations
}

/// Whether an editable field takes a plain string or a JSON value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    /// The value is taken verbatim.
    Text,
    /// The value is parsed as JSON.
    Json,
}

/// The two request bodies the field table feeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    /// `POST /client/agents`: the agent and its first draft revision.
    CreateAgent,
    /// `POST /client/agents/{slug}/revisions`: a new draft revision.
    CreateRevision,
}

/// Which bodies accept a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accepts {
    /// Both bodies; the value is stored on the revision.
    Both,
    /// Only the create-agent body; the value is stored on the agent, not the revision.
    CreateAgent,
    /// Only the create-revision body.
    CreateRevision,
}

impl Accepts {
    fn includes(self, body: BodyKind) -> bool {
        match self {
            Self::Both => true,
            Self::CreateAgent => body == BodyKind::CreateAgent,
            Self::CreateRevision => body == BodyKind::CreateRevision,
        }
    }
}

/// A body field and where its current value lives in a revision detail.
#[derive(Debug, Clone, Copy)]
pub struct Field {
    /// Key in the request body.
    pub body_key: &'static str,
    /// Keys to read in a revision detail, first match wins.
    pub detail_keys: &'static [&'static str],
    /// Value kind for `--set`.
    pub kind: FieldKind,
    /// Only sent for audio agents (the server drops it for text agents).
    pub audio_only: bool,
    /// Which bodies accept it.
    pub accepts: Accepts,
}

const fn field(
    body_key: &'static str,
    detail_keys: &'static [&'static str],
    kind: FieldKind,
    audio_only: bool,
    accepts: Accepts,
) -> Field {
    Field {
        body_key,
        detail_keys,
        kind,
        audio_only,
        accepts,
    }
}

/// Every field an agent or revision body carries besides the tool lists, `type` and
/// `visibility`. `voice_id` is read from `voice.id`.
pub const FIELDS: &[Field] = &[
    field("name", &["name"], FieldKind::Text, false, Accepts::Both),
    field(
        "vocal_name",
        &["vocal_name"],
        FieldKind::Text,
        false,
        Accepts::Both,
    ),
    field(
        "ai_greeting_message",
        &["greeting_message", "ai_greeting_message"],
        FieldKind::Text,
        false,
        Accepts::Both,
    ),
    field(
        "ai_instruction_identity",
        &["ai_instruction_identity"],
        FieldKind::Text,
        false,
        Accepts::Both,
    ),
    field(
        "ai_instruction_task",
        &["ai_instruction_task"],
        FieldKind::Text,
        false,
        Accepts::Both,
    ),
    field(
        "ai_instructions",
        &["ai_instructions"],
        FieldKind::Text,
        false,
        Accepts::CreateRevision,
    ),
    field(
        "ai_instruction_flow",
        &["ai_instruction_flow"],
        FieldKind::Json,
        false,
        Accepts::Both,
    ),
    field(
        "ai_instruction_variables",
        &["ai_instruction_variables"],
        FieldKind::Json,
        false,
        Accepts::Both,
    ),
    field("voice_id", &[], FieldKind::Json, true, Accepts::Both),
    field(
        "voice_speed",
        &["voice_speed"],
        FieldKind::Json,
        true,
        Accepts::Both,
    ),
    field(
        "prompt_engine_version",
        &["prompt_engine_version"],
        FieldKind::Text,
        false,
        Accepts::Both,
    ),
    field(
        "profile_url",
        &[],
        FieldKind::Text,
        false,
        Accepts::CreateAgent,
    ),
    field(
        "description",
        &[],
        FieldKind::Text,
        false,
        Accepts::CreateAgent,
    ),
    field(
        "ai_instruction_default_variables",
        &[],
        FieldKind::Json,
        false,
        Accepts::CreateAgent,
    ),
];

/// Fields `POST /client/agents` refuses to go without; `vocal_name` only for audio agents.
const CREATE_REQUIRED: &[&str] = &[
    "name",
    "vocal_name",
    "ai_greeting_message",
    "ai_instruction_identity",
    "ai_instruction_task",
];

/// Required create-agent fields that are absent or blank, in table order. The server trims
/// these, so a blank string counts as missing.
pub fn missing_create_fields(
    body: &Map<String, Value>,
    agent_type: AgentType,
) -> Vec<&'static str> {
    CREATE_REQUIRED
        .iter()
        .copied()
        .filter(|&key| !(key == "vocal_name" && agent_type == AgentType::Text))
        .filter(|&key| {
            body.get(key)
                .and_then(Value::as_str)
                .is_none_or(|s| s.trim().is_empty())
        })
        .collect()
}

/// Looks up an editable field by its body key.
pub fn field_named(name: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|f| f.body_key == name)
}

fn detail_value(detail: &Value, field: &Field) -> Option<Value> {
    if field.body_key == "voice_id" {
        return detail
            .pointer("/voice/id")
            .filter(|v| !v.is_null())
            .cloned();
    }
    field
        .detail_keys
        .iter()
        .find_map(|k| detail.get(*k))
        .filter(|v| !v.is_null())
        .cloned()
}

/// Unwraps `{data: {revision|agent: X}}` or `{data: X}` to `X`.
pub fn unwrap_data<'a>(envelope: &'a Value, inner: &str) -> &'a Value {
    let data = envelope.get("data").unwrap_or(envelope);
    match data.get(inner) {
        Some(v) if v.is_object() => v,
        _ => data,
    }
}

/// One revision in the revisions list.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RevisionSummary {
    /// Revision id.
    #[serde(default)]
    pub id: u64,
    /// `draft`, `published` or `deleted`.
    #[serde(default)]
    pub status: String,
}

/// The newest revision that is not deleted, from a revisions list envelope.
pub fn newest_revision(list: &Value) -> Option<RevisionSummary> {
    let items: Vec<RevisionSummary> = list
        .pointer("/data/revisions")
        .or_else(|| list.get("revisions"))
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    items
        .into_iter()
        .filter(|r| r.status != "deleted")
        .max_by_key(|r| r.id)
}

/// Tools of the agent's current draft, when the draft is `revision_id`.
pub fn draft_tools(agent: &Value, revision_id: u64) -> Option<ToolIds> {
    let draft = agent.get("ai_agent_instruction_revision")?;
    (draft.get("id").and_then(Value::as_u64) == Some(revision_id))
        .then(|| ToolIds::from_detail(draft))
}

/// The tools a new revision must carry: what is bound now, plus the draft's own links
/// when building on that draft.
pub fn tools_to_preserve(agent: &Value, base_revision_id: u64) -> ToolIds {
    let bound = ToolIds::from_detail(agent);
    match draft_tools(agent, base_revision_id) {
        Some(draft) => bound.union(&draft),
        None => bound,
    }
}

/// Builds the full create body from a revision detail and the tool ids to keep.
pub fn base_body(revision: &Value, tools: &ToolIds, agent_type: AgentType) -> Map<String, Value> {
    let mut body = Map::new();
    for f in FIELDS {
        if (f.audio_only && agent_type == AgentType::Text)
            || !f.accepts.includes(BodyKind::CreateRevision)
        {
            continue;
        }
        if let Some(v) = detail_value(revision, f) {
            if f.body_key == "prompt_engine_version" && v.as_str().is_some_and(str::is_empty) {
                continue;
            }
            body.insert(f.body_key.to_owned(), v);
        }
    }
    for kind in ToolKind::ALL
        .into_iter()
        .filter(|k| k.applies_to(agent_type))
    {
        body.insert(kind.body_key().to_owned(), ids_value(&tools.get(kind)));
    }
    body
}

fn ids_value(ids: &BTreeSet<u64>) -> Value {
    Value::Array(ids.iter().map(|&id| Value::from(id)).collect())
}

/// Parses a `--set` value for a field.
pub fn parse_field_value(target: BodyKind, name: &str, raw: &str) -> Result<Value, String> {
    let f = lookup(target, name)?;
    match f.kind {
        FieldKind::Text => Ok(Value::String(raw.to_owned())),
        FieldKind::Json => {
            serde_json::from_str(raw).map_err(|e| format!("`{name}` takes JSON: {e}"))
        }
    }
}

fn lookup(target: BodyKind, name: &str) -> Result<&'static Field, String> {
    let (what, tool_flags) = match target {
        BodyKind::CreateAgent => ("a create-agent field", "--add-tool"),
        BodyKind::CreateRevision => ("a revision field", "--add-tool / --remove-tool"),
    };
    if ToolKind::ALL.iter().any(|k| k.body_key() == name) {
        return Err(format!("`{name}` changes only through {tool_flags}"));
    }
    match field_named(name) {
        Some(f) if f.accepts.includes(target) => Ok(f),
        _ => {
            let known: Vec<&str> = FIELDS
                .iter()
                .filter(|f| f.accepts.includes(target))
                .map(|f| f.body_key)
                .collect();
            Err(format!(
                "`{name}` is not {what}; use one of {}",
                known.join(", ")
            ))
        }
    }
}

/// Sets one field, checking that the body takes it, it applies to the agent, and it has
/// the right kind.
pub fn set_field(
    body: &mut Map<String, Value>,
    target: BodyKind,
    agent_type: AgentType,
    name: &str,
    value: Value,
) -> Result<(), String> {
    let f = lookup(target, name)?;
    if f.audio_only && agent_type == AgentType::Text {
        return Err(format!("`{name}` does not apply to a text agent"));
    }
    if f.kind == FieldKind::Text && !value.is_string() {
        return Err(format!("`{name}` must be a string"));
    }
    body.insert(name.to_owned(), value);
    Ok(())
}

/// Adds and removes tools by name. Returns the tools removed, which the guard then allows.
pub fn apply_tool_edits(
    body: &mut Map<String, Value>,
    agent_type: AgentType,
    add: &[ToolRef],
    remove: &[ToolRef],
) -> Result<Vec<ToolRef>, String> {
    let mut ids = ToolIds::default();
    for kind in ToolKind::ALL {
        if let Some(set) = body_ids(&Value::Object(body.clone()), kind) {
            for id in set {
                ids.insert(ToolRef { kind, id });
            }
        }
    }
    for tool in add.iter().chain(remove) {
        if !tool.kind.applies_to(agent_type) {
            return Err(format!(
                "{tool}: {} tools do not apply to this agent type",
                tool.kind.label()
            ));
        }
    }
    for &tool in add {
        ids.insert(tool);
    }
    let mut removed = Vec::new();
    for &tool in remove {
        if !ids.remove(tool) {
            return Err(format!("{tool} is not linked to this agent"));
        }
        removed.push(tool);
    }
    for kind in ToolKind::ALL
        .into_iter()
        .filter(|k| k.applies_to(agent_type))
    {
        body.insert(kind.body_key().to_owned(), ids_value(&ids.get(kind)));
    }
    Ok(removed)
}

/// One field that differs between two bodies.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldChange {
    /// Body key.
    pub key: String,
    /// Value before, if any.
    pub old: Option<Value>,
    /// Value after, if any.
    pub new: Option<Value>,
}

/// Fields whose values differ, in body-key order.
pub fn diff_bodies(old: &Map<String, Value>, new: &Map<String, Value>) -> Vec<FieldChange> {
    let keys: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    keys.into_iter()
        .filter(|k| old.get(*k).map(normalize_numbers) != new.get(*k).map(normalize_numbers))
        .map(|k| FieldChange {
            key: k.clone(),
            old: old.get(k).cloned(),
            new: new.get(k).cloned(),
        })
        .collect()
}

/// Compares what was sent against what the server stored. `stored` is the revision
/// detail from GET; `created` is the create response, the only place a revision's
/// tool ids are readable. Fields stored on the agent rather than the revision, and keys
/// outside the field table, are not compared.
pub fn readback_mismatches(
    sent: &Map<String, Value>,
    stored: &Value,
    created: &Value,
    agent_type: AgentType,
) -> Vec<String> {
    let mut mismatches = Vec::new();
    for (key, sent_value) in sent {
        if let Some(kind) = ToolKind::ALL.into_iter().find(|k| k.body_key() == key) {
            let want = body_ids(&Value::Object(sent.clone()), kind).unwrap_or_default();
            let got = ToolIds::from_detail(created).get(kind);
            if want != got {
                mismatches.push(format!("{key}: sent {want:?}, stored {got:?}"));
            }
            continue;
        }
        let Some(f) = field_named(key).filter(|f| f.accepts != Accepts::CreateAgent) else {
            continue;
        };
        if key == "ai_instructions" && sent_value.as_str().is_some_and(str::is_empty) {
            continue;
        }
        if key == "prompt_engine_version" && detail_value(stored, f).is_none() {
            continue;
        }
        let got = detail_value(stored, f).unwrap_or(Value::Null);
        let (want, got) = if key == "ai_instruction_flow" {
            (
                normalize_flow(sent_value, agent_type),
                normalize_flow(&got, agent_type),
            )
        } else {
            (normalize_numbers(sent_value), normalize_numbers(&got))
        };
        if want != got {
            mismatches.push(format!("{key}: stored value differs from what was sent"));
        }
    }
    mismatches
}

fn normalize_numbers(v: &Value) -> Value {
    match v {
        Value::Number(n) => n
            .as_f64()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(|| v.clone(), Value::Number),
        Value::Array(items) => Value::Array(items.iter().map(normalize_numbers).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), normalize_numbers(v)))
                .collect(),
        ),
        _ => v.clone(),
    }
}

/// The server fills `end_call_keyword: false` in for audio agents and strips it for text
/// agents, so a stored flow may differ from the sent one only in that key.
fn normalize_flow(flow: &Value, agent_type: AgentType) -> Value {
    let mut flow = normalize_numbers(flow);
    if flow.is_null() {
        flow = Value::Array(Vec::new());
    }
    if let Some(nodes) = flow.as_array_mut() {
        for node in nodes {
            let Some(obj) = node.as_object_mut() else {
                continue;
            };
            let empty = match obj.get_mut("guidelines").and_then(Value::as_object_mut) {
                Some(g) => {
                    let drop = agent_type == AgentType::Text
                        || g.get("end_call_keyword") == Some(&Value::Bool(false));
                    if drop {
                        g.remove("end_call_keyword");
                    }
                    g.is_empty()
                }
                None => false,
            };
            if empty {
                obj.remove("guidelines");
            }
        }
    }
    flow
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn tool(kind: ToolKind, id: u64) -> ToolRef {
        ToolRef { kind, id }
    }

    fn bound(tools: &[ToolRef]) -> ToolIds {
        let mut ids = ToolIds::default();
        for &t in tools {
            ids.insert(t);
        }
        ids
    }

    fn audio_bound() -> ToolIds {
        bound(&[tool(ToolKind::Plugin, 12), tool(ToolKind::Phone, 5)])
    }

    #[test]
    fn guard_passes_body_that_keeps_every_bound_tool() {
        let body = json!({"ai_plugin_function_ids": [12, 13], "phone_tools": [5]});
        assert_eq!(
            check_revision_body(AgentType::Audio, &audio_bound(), &body),
            vec![]
        );
    }

    #[test]
    fn guard_refuses_dropping_a_bound_tool() {
        let body = json!({"ai_plugin_function_ids": [12], "phone_tools": []});
        assert_eq!(
            check_revision_body(AgentType::Audio, &audio_bound(), &body),
            vec![GuardViolation::DroppedTool(tool(ToolKind::Phone, 5))]
        );
    }

    #[test]
    fn guard_refuses_missing_list_key() {
        let body = json!({"ai_plugin_function_ids": [12]});
        assert_eq!(
            check_revision_body(AgentType::Audio, &audio_bound(), &body),
            vec![GuardViolation::MissingList(ToolKind::Phone)]
        );
    }

    #[test]
    fn guard_refuses_flow_reference_to_unlinked_tool() {
        let body = json!({
            "ai_plugin_function_ids": [12],
            "phone_tools": [5],
            "ai_instruction_flow": [
                {"id": "lookup", "guidelines": {"plugin_functions": [{"id": 99, "instruction": "x"}]}}
            ]
        });
        assert_eq!(
            check_revision_body(AgentType::Audio, &audio_bound(), &body),
            vec![GuardViolation::UnboundFlowRef {
                node: "lookup".to_owned(),
                tool: tool(ToolKind::Plugin, 99)
            }]
        );
    }

    #[test]
    fn guard_skips_text_chat_list_for_audio_agent() {
        let body = json!({"ai_plugin_function_ids": [12], "phone_tools": [5]});
        assert!(check_revision_body(AgentType::Audio, &audio_bound(), &body).is_empty());
    }

    #[test]
    fn guard_skips_phone_list_for_text_agent() {
        let text_bound = bound(&[tool(ToolKind::TextChat, 1)]);
        let body = json!({"ai_plugin_function_ids": [], "text_chat_tools": [1]});
        assert!(check_revision_body(AgentType::Text, &text_bound, &body).is_empty());
    }

    #[test]
    fn guard_requires_text_chat_list_for_text_agent() {
        let text_bound = bound(&[tool(ToolKind::TextChat, 1)]);
        let body = json!({"ai_plugin_function_ids": []});
        assert_eq!(
            check_revision_body(AgentType::Text, &text_bound, &body),
            vec![GuardViolation::MissingList(ToolKind::TextChat)]
        );
    }

    fn revision_detail() -> Value {
        json!({
            "id": 17,
            "prompt_engine_version": "v3",
            "name": "Mali",
            "vocal_name": "มะลิ",
            "greeting_message": "สวัสดีค่ะ",
            "ai_instruction_identity": "You are Mali",
            "ai_instruction_task": "Help",
            "ai_instruction_flow": [],
            "ai_instructions": "",
            "ai_instruction_variables": {"company": "Ingfah"},
            "voice": {"id": 3, "name": "Nara"},
            "voice_speed": 1,
            "created_at": "2026-05-25T11:05:00Z"
        })
    }

    #[test]
    fn base_body_maps_greeting_and_voice_to_create_names() {
        let body = base_body(&revision_detail(), &audio_bound(), AgentType::Audio);
        assert_eq!(
            (body.get("ai_greeting_message"), body.get("voice_id")),
            (Some(&json!("สวัสดีค่ะ")), Some(&json!(3)))
        );
    }

    #[test]
    fn base_body_carries_bound_tool_ids() {
        let body = base_body(&revision_detail(), &audio_bound(), AgentType::Audio);
        assert_eq!(
            (body.get("ai_plugin_function_ids"), body.get("phone_tools")),
            (Some(&json!([12])), Some(&json!([5])))
        );
    }

    #[test]
    fn base_body_omits_voice_and_phone_for_text_agent() {
        let body = base_body(&revision_detail(), &ToolIds::default(), AgentType::Text);
        let keys: Vec<&str> = body.keys().map(String::as_str).collect();
        assert!(
            !keys
                .iter()
                .any(|k| ["voice_id", "voice_speed", "phone_tools"].contains(k))
        );
    }

    #[test]
    fn base_body_drops_read_only_fields() {
        let body = base_body(&revision_detail(), &audio_bound(), AgentType::Audio);
        assert!(!body.contains_key("created_at") && !body.contains_key("voice"));
    }

    #[test]
    fn tools_to_preserve_unions_draft_links_when_building_on_draft() {
        let agent = json!({
            "phone_tools": [{"id": 5}],
            "ai_agent_instruction_revision": {"id": 18, "ai_plugin_functions": [{"id": 12}]}
        });
        assert_eq!(
            tools_to_preserve(&agent, 18),
            bound(&[tool(ToolKind::Phone, 5), tool(ToolKind::Plugin, 12)])
        );
    }

    #[test]
    fn newest_revision_skips_deleted() {
        let list = json!({"data": {"revisions": [
            {"id": 19, "status": "deleted"},
            {"id": 18, "status": "draft"},
            {"id": 17, "status": "published"}
        ]}});
        assert_eq!(
            newest_revision(&list),
            Some(RevisionSummary {
                id: 18,
                status: "draft".to_owned()
            })
        );
    }

    #[test]
    fn set_field_refuses_tool_lists() {
        let mut body = Map::new();
        let err = set_field(
            &mut body,
            BodyKind::CreateRevision,
            AgentType::Audio,
            "phone_tools",
            json!([1]),
        );
        assert!(err.is_err_and(|e| e.contains("--add-tool")));
    }

    #[test]
    fn parse_field_value_reads_json_for_voice_id() {
        assert_eq!(
            parse_field_value(BodyKind::CreateRevision, "voice_id", "4"),
            Ok(json!(4))
        );
    }

    #[test]
    fn parse_field_value_keeps_text_verbatim() {
        assert_eq!(
            parse_field_value(BodyKind::CreateRevision, "name", "4"),
            Ok(json!("4"))
        );
    }

    #[test]
    fn set_field_refuses_agent_only_field_on_a_revision() {
        let mut body = Map::new();
        let err = set_field(
            &mut body,
            BodyKind::CreateRevision,
            AgentType::Audio,
            "description",
            json!("x"),
        );
        assert!(err.is_err_and(|e| e.contains("not a revision field")));
    }

    #[test]
    fn set_field_refuses_ai_instructions_on_create() {
        let mut body = Map::new();
        let err = set_field(
            &mut body,
            BodyKind::CreateAgent,
            AgentType::Audio,
            "ai_instructions",
            json!("x"),
        );
        assert!(err.is_err_and(|e| e.contains("not a create-agent field")));
    }

    #[test]
    fn missing_create_fields_lists_every_blank_required_field() {
        let mut body = Map::new();
        body.insert("name".to_owned(), json!("Mali"));
        body.insert("ai_instruction_task".to_owned(), json!("  "));
        assert_eq!(
            missing_create_fields(&body, AgentType::Audio),
            vec![
                "vocal_name",
                "ai_greeting_message",
                "ai_instruction_identity",
                "ai_instruction_task"
            ]
        );
    }

    #[test]
    fn missing_create_fields_does_not_require_vocal_name_for_text() {
        assert!(!missing_create_fields(&Map::new(), AgentType::Text).contains(&"vocal_name"));
    }

    #[test]
    fn base_body_leaves_out_agent_only_fields() {
        let mut revision = revision_detail();
        revision["description"] = json!("note");
        let body = base_body(&revision, &audio_bound(), AgentType::Audio);
        assert!(!body.contains_key("description"));
    }

    #[test]
    fn readback_skips_fields_stored_on_the_agent() {
        let mut sent = Map::new();
        sent.insert("description".to_owned(), json!("note"));
        sent.insert("visibility".to_owned(), json!("public"));
        assert!(readback_mismatches(&sent, &json!({}), &json!({}), AgentType::Audio).is_empty());
    }

    #[test]
    fn tool_edits_add_and_remove_by_name() {
        let mut body = base_body(&revision_detail(), &audio_bound(), AgentType::Audio);
        let removed = apply_tool_edits(
            &mut body,
            AgentType::Audio,
            &[tool(ToolKind::Plugin, 40)],
            &[tool(ToolKind::Phone, 5)],
        );
        assert_eq!(
            (
                removed,
                body.get("ai_plugin_function_ids"),
                body.get("phone_tools")
            ),
            (
                Ok(vec![tool(ToolKind::Phone, 5)]),
                Some(&json!([12, 40])),
                Some(&json!([]))
            )
        );
    }

    #[test]
    fn tool_edits_refuse_kind_that_does_not_apply() {
        let mut body = Map::new();
        let result = apply_tool_edits(
            &mut body,
            AgentType::Audio,
            &[tool(ToolKind::TextChat, 1)],
            &[],
        );
        assert!(result.is_err());
    }

    #[test]
    fn tool_ref_parses_kind_and_id() {
        assert_eq!("phone:5".parse(), Ok(tool(ToolKind::Phone, 5)));
    }

    #[test]
    fn diff_ignores_integer_versus_float() {
        let mut old = Map::new();
        old.insert("voice_speed".to_owned(), json!(1));
        let mut new = Map::new();
        new.insert("voice_speed".to_owned(), json!(1.0));
        assert!(diff_bodies(&old, &new).is_empty());
    }

    #[test]
    fn readback_accepts_server_filled_end_call_keyword() {
        let mut sent = Map::new();
        sent.insert(
            "ai_instruction_flow".to_owned(),
            json!([{"id": "a", "transitions": []}]),
        );
        let stored = json!({"ai_instruction_flow": [
            {"id": "a", "transitions": [], "guidelines": {"end_call_keyword": false}}
        ]});
        assert!(readback_mismatches(&sent, &stored, &json!({}), AgentType::Audio).is_empty());
    }

    #[test]
    fn readback_reports_changed_task() {
        let mut sent = Map::new();
        sent.insert("ai_instruction_task".to_owned(), json!("new"));
        let stored = json!({"ai_instruction_task": "old"});
        assert_eq!(
            readback_mismatches(&sent, &stored, &json!({}), AgentType::Audio).len(),
            1
        );
    }

    #[test]
    fn readback_compares_tool_ids_from_create_response() {
        let mut sent = Map::new();
        sent.insert("phone_tools".to_owned(), json!([5]));
        let created = json!({"phone_tools": []});
        assert_eq!(
            readback_mismatches(&sent, &json!({}), &created, AgentType::Audio).len(),
            1
        );
    }

    #[test]
    fn readback_maps_voice_id_to_nested_voice() {
        let mut sent = Map::new();
        sent.insert("voice_id".to_owned(), json!(3));
        let stored = json!({"voice": {"id": 3}});
        assert!(readback_mismatches(&sent, &stored, &json!({}), AgentType::Audio).is_empty());
    }
}
