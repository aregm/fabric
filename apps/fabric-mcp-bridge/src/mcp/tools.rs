//! The ten tools, made from the contract's catalog and argument schemas (W06 3.5), and the
//! filling in of nulls before a call is read by the contract (W06 3.6 step 3).

use std::sync::OnceLock;

use fabric_desktop_contracts::common::ToolName;
use fabric_desktop_contracts::parse_strict;
use serde_json::{Map, Value, json};

const CATALOG: &str = include_str!("../../../../contracts/desktop/v1/data/tool-catalog.json");
const AGENT_SCHEMA: &str =
    include_str!("../../../../contracts/desktop/v1/schemas/agent-channel.schema.json");
const COMMON_SCHEMA: &str =
    include_str!("../../../../contracts/desktop/v1/schemas/common.schema.json");

const AGENT_FILE: &str = "agent-channel.schema.json";
const COMMON_FILE: &str = "common.schema.json";

/// The sentence rule 2 adds to a member that may be null.
pub const LEAVE_OUT: &str = "Leave this member out for none.";

/// The examples of rule 4: a UTC instant, a local time with its offset, a zone.
pub const TIMESTAMP_EXAMPLE: &str = "2026-11-20T16:00:00.000Z";
pub const OFFSET_DATE_TIME_EXAMPLE: &str = "2026-11-20T17:00:00+01:00";
pub const ZONE_EXAMPLE: &str = "Europe/Berlin";

/// One tool as `tools/list` shows it, with the contract's own schema kept for the nulls.
#[derive(Clone, Debug)]
pub struct Tool {
    pub name: ToolName,
    pub title: &'static str,
    pub description: String,
    /// The schema after the four rules of W06 3.5.
    pub input_schema: Value,
    /// The contract's argument schema with its references inlined and nothing else changed.
    pub contract_schema: Value,
}

impl Tool {
    /// The tool as an entry of `tools/list`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name.as_str(),
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input_schema,
            "annotations": annotations(self.name),
        })
    }
}

/// The bridge's own short name of a tool, for a person.
#[must_use]
pub const fn title(tool: ToolName) -> &'static str {
    match tool {
        ToolName::CalendarList => "List calendars",
        ToolName::CalendarAvailability => "Check availability",
        ToolName::CalendarRead => "Read events",
        ToolName::ProposeCreate => "Propose a new event",
        ToolName::ProposeUpdate => "Propose changes to an event",
        ToolName::ProposeInvitation => "Propose an attendee list",
        ToolName::ProposeResponse => "Propose an answer to an invitation",
        ToolName::ProposeDelete => "Propose deleting an event",
        ToolName::RequestStatus => "Show a request's state",
        ToolName::RequestResult => "Get a request's result",
    }
}

/// The hints of W06 3.5. They carry no authority in Fabric Calendar.
#[must_use]
pub fn annotations(tool: ToolName) -> Value {
    match tool {
        ToolName::CalendarList
        | ToolName::CalendarAvailability
        | ToolName::CalendarRead
        | ToolName::RequestStatus => json!({"readOnlyHint": true}),
        ToolName::RequestResult | ToolName::ProposeCreate => {
            json!({"readOnlyHint": false, "destructiveHint": false})
        }
        ToolName::ProposeUpdate
        | ToolName::ProposeInvitation
        | ToolName::ProposeResponse
        | ToolName::ProposeDelete => json!({"readOnlyHint": false, "destructiveHint": true}),
    }
}

/// The ten tools in the catalog's order.
///
/// # Panics
///
/// Only if the contract files compiled into the binary do not have the shape the contract
/// gives them, which the tests rule out.
#[must_use]
pub fn tools() -> &'static [Tool] {
    static TOOLS: OnceLock<Vec<Tool>> = OnceLock::new();
    TOOLS.get_or_init(|| {
        build().unwrap_or_else(|_| unreachable!("the compiled-in contract files make ten tools"))
    })
}

/// The tool named `name`, if it is one of the ten.
#[must_use]
pub fn find(name: &str) -> Option<&'static Tool> {
    tools().iter().find(|t| t.name.as_str() == name)
}

/// The tool `name`.
#[must_use]
pub fn tool(name: ToolName) -> &'static Tool {
    // Every name is in the list: `build` makes one tool per catalog entry, and t01 checks
    // that the catalog holds exactly `ToolName::ALL`.
    tools()
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| unreachable!("every tool name is listed"))
}

/// `tools/list`'s `tools` array.
#[must_use]
pub fn list_json() -> Value {
    Value::Array(tools().iter().map(Tool::to_json).collect())
}

struct Files {
    agent: Value,
    common: Value,
}

impl Files {
    fn definition(&self, file: &str, name: &str) -> &Value {
        let root = if file == COMMON_FILE {
            &self.common
        } else {
            &self.agent
        };
        &root["$defs"][name]
    }
}

fn parse(text: &str) -> Value {
    parse_strict(text.as_bytes()).unwrap_or_else(|_| unreachable!("the contract files are JSON"))
}

/// Why the tools could not be made from the contract's files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaError {
    /// A catalog entry lacks its name, a known tool name, its arguments or its description.
    Catalog,
    /// Rule 3: two branches of a `kind` union give the member of this name different schemas.
    Conflict(String),
}

/// Makes the ten tools from the compiled-in catalog and schemas.
pub fn build() -> Result<Vec<Tool>, SchemaError> {
    let files = Files {
        agent: parse(AGENT_SCHEMA),
        common: parse(COMMON_SCHEMA),
    };
    let catalog = parse(CATALOG);
    let entries = catalog["tools"].as_array().cloned().unwrap_or_default();
    entries
        .iter()
        .map(|entry| {
            let name = entry["name"]
                .as_str()
                .and_then(ToolName::from_name)
                .ok_or(SchemaError::Catalog)?;
            let reference = entry["arguments"].as_str().ok_or(SchemaError::Catalog)?;
            let description = entry["description"].as_str().ok_or(SchemaError::Catalog)?;
            let (file, definition) = split_reference(AGENT_FILE, reference);
            let source = files.definition(&file, &definition).clone();
            let contract_schema = inline(&files, &file, &source, false);
            let input_schema = transform(&inline(&files, &file, &source, true))?;
            Ok(Tool {
                name,
                title: title(name),
                description: description.to_owned(),
                input_schema,
                contract_schema,
            })
        })
        .collect()
}

/// Splits a reference into the file it names (or `base` for `#...`) and the definition.
fn split_reference(base: &str, reference: &str) -> (String, String) {
    let (file, fragment) = reference.split_once('#').unwrap_or((reference, ""));
    let file = if file.is_empty() { base } else { file };
    let definition = fragment.rsplit('/').next().unwrap_or_default();
    (file.to_owned(), definition.to_owned())
}

/// Rule 1: every `$ref` replaced by its definition, resolved against the file that holds it.
/// With `examples`, rule 4 is applied to the three time formats on the way.
fn inline(files: &Files, file: &str, node: &Value, examples: bool) -> Value {
    match node {
        Value::Object(members) => {
            if let Some(Value::String(reference)) = members.get("$ref") {
                let (target_file, definition) = split_reference(file, reference);
                let mut resolved = inline(
                    files,
                    &target_file,
                    files.definition(&target_file, &definition),
                    examples,
                );
                if let Value::Object(out) = &mut resolved {
                    for (k, v) in members {
                        if k != "$ref" {
                            out.insert(k.clone(), inline(files, file, v, examples));
                        }
                    }
                }
                // Rule 4 after the siblings are merged in, so a reference with a description of
                // its own keeps the example.
                if examples && target_file == COMMON_FILE {
                    add_example(&mut resolved, &definition);
                }
                resolved
            } else {
                Value::Object(
                    members
                        .iter()
                        .map(|(k, v)| (k.clone(), inline(files, file, v, examples)))
                        .collect(),
                )
            }
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| inline(files, file, v, examples))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Rule 4: the descriptions of the three time formats gain an example.
fn add_example(schema: &mut Value, definition: &str) {
    let example = match definition {
        "timestamp" => TIMESTAMP_EXAMPLE,
        "offset_date_time" => OFFSET_DATE_TIME_EXAMPLE,
        "iana_zone" => ZONE_EXAMPLE,
        _ => return,
    };
    append_description(schema, &format!("Example: {example}"));
}

fn append_description(schema: &mut Value, sentence: &str) {
    if let Value::Object(members) = schema {
        let text = match members.get("description") {
            Some(Value::String(d)) if !d.is_empty() => format!("{d} {sentence}"),
            _ => sentence.to_owned(),
        };
        members.insert("description".to_owned(), Value::String(text));
    }
}

/// The schema X of a member whose schema is `oneOf` of `null` and X (rule 2).
fn nullable_inner(schema: &Value) -> Option<&Value> {
    let branches = schema.get("oneOf")?.as_array()?;
    if branches.len() != 2 {
        return None;
    }
    let is_null = |b: &Value| b.get("type") == Some(&json!("null"));
    match (is_null(&branches[0]), is_null(&branches[1])) {
        (true, false) => Some(&branches[1]),
        (false, true) => Some(&branches[0]),
        _ => None,
    }
}

/// The `kind` constant of each branch, when the schema is a union of objects told apart by
/// `kind` (rule 3).
fn kind_union(schema: &Value) -> Option<Vec<(&str, &Value)>> {
    let branches = schema.get("oneOf")?.as_array()?;
    branches
        .iter()
        .map(|b| Some((b.get("properties")?.get("kind")?.get("const")?.as_str()?, b)))
        .collect()
}

/// Rules 2 and 3 on an inlined schema.
fn transform(schema: &Value) -> Result<Value, SchemaError> {
    if let Some(branches) = kind_union(schema) {
        let mut properties = Map::new();
        properties.insert(
            "kind".to_owned(),
            json!({"enum": branches.iter().map(|(k, _)| *k).collect::<Vec<_>>()}),
        );
        for (_, branch) in &branches {
            if let Some(Value::Object(members)) = branch.get("properties") {
                for (name, member) in members {
                    if name == "kind" {
                        continue;
                    }
                    match properties.get(name) {
                        None => {
                            properties.insert(name.clone(), member.clone());
                        }
                        Some(seen) if seen == member => {}
                        Some(_) => return Err(SchemaError::Conflict(name.clone())),
                    }
                }
            }
        }
        let mut merged = Map::new();
        if let Some(description) = schema.get("description") {
            merged.insert("description".to_owned(), description.clone());
        }
        merged.insert("type".to_owned(), json!("object"));
        merged.insert("properties".to_owned(), Value::Object(properties));
        merged.insert("required".to_owned(), json!(["kind"]));
        merged.insert("additionalProperties".to_owned(), json!(false));
        return transform(&Value::Object(merged));
    }
    let Value::Object(members) = schema else {
        return Ok(schema.clone());
    };
    let mut out = members.clone();
    let mut optional = Vec::new();
    if let Some(Value::Object(properties)) = members.get("properties") {
        let mut changed = Map::new();
        for (name, member) in properties {
            if let Some(inner) = nullable_inner(member) {
                let mut written = transform(inner)?;
                // What sits beside the `oneOf`, such as a description, is kept.
                if let (Value::Object(out), Value::Object(beside)) = (&mut written, member) {
                    for (k, v) in beside {
                        if k != "oneOf" {
                            out.insert(k.clone(), v.clone());
                        }
                    }
                }
                append_description(&mut written, LEAVE_OUT);
                changed.insert(name.clone(), written);
                optional.push(name.clone());
            } else {
                changed.insert(name.clone(), transform(member)?);
            }
        }
        out.insert("properties".to_owned(), Value::Object(changed));
    }
    if let Some(Value::Array(required)) = members.get("required") {
        let kept: Vec<Value> = required
            .iter()
            .filter(|r| !optional.iter().any(|o| r.as_str() == Some(o)))
            .cloned()
            .collect();
        out.insert("required".to_owned(), Value::Array(kept));
    }
    if let Some(items) = members.get("items") {
        out.insert("items".to_owned(), transform(items)?);
    }
    Ok(Value::Object(out))
}

/// Writes `null` for every absent member that the contract lets be null, at every depth,
/// beside the contract's own schema (rule 2 backwards). A present member, null or not, is
/// left as it is, and nothing else is added (W06 3.6 step 3).
pub fn fill_nulls(tool: ToolName, arguments: &mut Value) {
    fill(&self::tool(tool).contract_schema, arguments);
}

fn fill(schema: &Value, value: &mut Value) {
    if let Some(inner) = nullable_inner(schema) {
        if !value.is_null() {
            fill(inner, value);
        }
        return;
    }
    if let Some(branches) = kind_union(schema) {
        let kind = value.get("kind").and_then(Value::as_str).map(str::to_owned);
        if let Some((_, branch)) = branches.iter().find(|(k, _)| Some(*k) == kind.as_deref()) {
            fill(branch, value);
        }
        return;
    }
    match value {
        Value::Object(members) => {
            if let Some(Value::Object(properties)) = schema.get("properties") {
                for (name, member_schema) in properties {
                    match members.get_mut(name) {
                        Some(member) => fill(member_schema, member),
                        None => {
                            if nullable_inner(member_schema).is_some() {
                                members.insert(name.clone(), Value::Null);
                            }
                        }
                    }
                }
            }
        }
        Value::Array(items) => {
            if let Some(item_schema) = schema.get("items") {
                for item in items {
                    fill(item_schema, item);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric_desktop_contracts::scalar::{IanaZone, OffsetDateTime, Timestamp};

    fn walk<'a>(value: &'a Value, path: String, out: &mut Vec<(String, &'a Value)>) {
        out.push((path.clone(), value));
        match value {
            Value::Object(m) => {
                for (k, v) in m {
                    walk(v, format!("{path}/{k}"), out);
                }
            }
            Value::Array(items) => {
                for (i, v) in items.iter().enumerate() {
                    walk(v, format!("{path}/{i}"), out);
                }
            }
            _ => {}
        }
    }

    // t01: the ten names are `ToolName::ALL` and the catalog, in order, with the catalog's
    // descriptions.
    #[test]
    fn t01_names_and_descriptions() {
        let catalog = parse(CATALOG);
        let entries = catalog["tools"].as_array().unwrap();
        assert_eq!(entries.len(), 10);
        assert_eq!(tools().len(), 10);
        for ((tool, name), entry) in tools().iter().zip(ToolName::ALL).zip(entries) {
            assert_eq!(tool.name, name);
            assert_eq!(entry["name"], json!(name.as_str()));
            assert_eq!(entry["description"], json!(tool.description));
            assert_eq!(
                entry["arguments"],
                json!(format!(
                    "agent-channel.schema.json#/$defs/{}",
                    name.arguments_definition()
                ))
            );
        }
        let listed = list_json();
        let names: Vec<&str> = listed
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        let expected: Vec<&str> = ToolName::ALL.iter().map(|t| t.as_str()).collect();
        assert_eq!(names, expected);
        for t in listed.as_array().unwrap() {
            let keys: Vec<&String> = t.as_object().unwrap().keys().collect();
            assert_eq!(
                keys.len(),
                5,
                "name, title, description, inputSchema, annotations"
            );
            assert!(t.get("outputSchema").is_none());
        }
    }

    // t02: no inputSchema holds $ref, oneOf, anyOf, allOf, not, if, then, else, const or a
    // null (as a type or in an enum); each root is an object schema; each member that may be
    // null is absent from required. The generator fails loudly, never silently.
    #[test]
    fn t02_schemas_are_plain() {
        let built = build();
        assert!(built.is_ok(), "the tools are made without error: {built:?}");
        let mut nullable_seen = 0;
        for tool in tools() {
            let schema = &tool.input_schema;
            assert_eq!(schema["type"], json!("object"), "{}", tool.name.as_str());
            assert!(schema["properties"].is_object());
            let mut nodes = Vec::new();
            walk(schema, String::new(), &mut nodes);
            for (path, node) in nodes {
                if let Value::Object(m) = node {
                    for bad in [
                        "$ref", "oneOf", "anyOf", "allOf", "not", "if", "then", "else", "const",
                    ] {
                        // `properties` may name a member "not" or "if"; only keywords count.
                        if path.ends_with("/properties") {
                            continue;
                        }
                        assert!(!m.contains_key(bad), "{} {path} {bad}", tool.name.as_str());
                    }
                    if let Some(Value::Array(values)) = m.get("enum") {
                        assert!(!values.contains(&Value::Null), "{path}");
                    }
                    assert_ne!(m.get("type"), Some(&json!("null")), "{path}");
                    if let Some(Value::Array(types)) = m.get("type") {
                        assert!(!types.contains(&json!("null")), "{path}");
                    }
                }
            }
            // Every nullable member of the contract schema is optional in the input schema.
            let mut contract_nodes = Vec::new();
            walk(&tool.contract_schema, String::new(), &mut contract_nodes);
            for (path, node) in contract_nodes {
                let Some(Value::Object(props)) = node.get("properties") else {
                    continue;
                };
                for (name, member) in props {
                    if nullable_inner(member).is_some() {
                        nullable_seen += 1;
                        let written = lookup(schema, &path).expect("same path in input schema");
                        let required = written["required"].as_array().unwrap();
                        assert!(!required.contains(&json!(name)), "{path} {name}");
                        let description =
                            written["properties"][name]["description"].as_str().unwrap();
                        assert!(description.ends_with(LEAVE_OUT));
                    }
                }
            }
        }
        // new_event: description and location; event_changes: all five.
        assert_eq!(nullable_seen, 7);
        let response = &tool(ToolName::ProposeResponse).input_schema["properties"]["target"];
        assert_eq!(response["required"], json!(["kind"]));
        assert_eq!(
            response["properties"]["kind"]["enum"],
            json!(["event", "occurrence"])
        );
    }

    // t02, the rules on schemas the contract does not have yet: a description beside a
    // nullable `oneOf` is kept (rule 2); a reference with its own description keeps the
    // example (rule 4); two branches that disagree about a member fail (rule 3).
    #[test]
    fn t02_rules_lose_nothing() {
        let nullable = json!({
            "type": "object",
            "properties": {"note": {
                "description": "A note.",
                "oneOf": [{"type": "null"}, {"type": "string", "description": "Inner."}],
            }},
            "required": ["note"],
        });
        let written = transform(&nullable).unwrap();
        assert_eq!(
            written["properties"]["note"],
            json!({"type": "string", "description": format!("A note. {LEAVE_OUT}")})
        );
        assert_eq!(written["required"], json!([]));

        let files = Files {
            agent: json!({"$defs": {"x": {"type": "object", "properties": {
                "at": {"$ref": "common.schema.json#/$defs/timestamp", "description": "When."},
            }}}}),
            common: json!({"$defs": {"timestamp": {"type": "string", "description": "Inner."}}}),
        };
        let inlined = inline(&files, AGENT_FILE, files.definition(AGENT_FILE, "x"), true);
        assert_eq!(
            inlined["properties"]["at"]["description"],
            json!(format!("When. Example: {TIMESTAMP_EXAMPLE}"))
        );

        let conflict = json!({"oneOf": [
            {"type": "object", "properties": {"kind": {"const": "a"}, "x": {"type": "string"}}},
            {"type": "object", "properties": {"kind": {"const": "b"}, "x": {"type": "integer"}}},
        ]});
        assert_eq!(
            transform(&conflict),
            Err(SchemaError::Conflict("x".to_owned()))
        );
        let same = json!({"oneOf": [
            {"type": "object", "properties": {"kind": {"const": "a"}, "x": {"type": "string"}}},
            {"type": "object", "properties": {"kind": {"const": "b"}, "x": {"type": "string"}}},
        ]});
        assert!(transform(&same).is_ok());
    }

    /// The node at a path of the contract schema, in the input schema. Only object paths
    /// through `properties` occur in the ten schemas.
    fn lookup<'a>(schema: &'a Value, path: &str) -> Option<&'a Value> {
        let mut node = schema;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            node = node.get(part)?;
        }
        Some(node)
    }

    // t03: the hints of 3.5, tool by tool.
    #[test]
    fn t03_hints() {
        let read_only = || json!({"readOnlyHint": true});
        let expected = [
            (ToolName::CalendarList, read_only()),
            (ToolName::CalendarAvailability, read_only()),
            (ToolName::CalendarRead, read_only()),
            (
                ToolName::ProposeCreate,
                json!({"readOnlyHint": false, "destructiveHint": false}),
            ),
            (
                ToolName::ProposeUpdate,
                json!({"readOnlyHint": false, "destructiveHint": true}),
            ),
            (
                ToolName::ProposeInvitation,
                json!({"readOnlyHint": false, "destructiveHint": true}),
            ),
            (
                ToolName::ProposeResponse,
                json!({"readOnlyHint": false, "destructiveHint": true}),
            ),
            (
                ToolName::ProposeDelete,
                json!({"readOnlyHint": false, "destructiveHint": true}),
            ),
            (ToolName::RequestStatus, read_only()),
            (
                ToolName::RequestResult,
                json!({"readOnlyHint": false, "destructiveHint": false}),
            ),
        ];
        for (name, hints) in expected {
            assert_eq!(
                tool(name).to_json()["annotations"],
                hints,
                "{}",
                name.as_str()
            );
        }
    }

    // t04: each example of rule 4 is accepted by the contract's own type for that format, and
    // appears in the schemas.
    #[test]
    fn t04_examples() {
        assert!(Timestamp::new(TIMESTAMP_EXAMPLE).is_ok());
        assert!(OffsetDateTime::new(OFFSET_DATE_TIME_EXAMPLE).is_ok());
        assert!(IanaZone::new(ZONE_EXAMPLE).is_ok());
        let window = &tool(ToolName::CalendarRead).input_schema["properties"]["window"];
        assert!(
            window["properties"]["start"]["description"]
                .as_str()
                .unwrap()
                .contains(TIMESTAMP_EXAMPLE)
        );
        let start = &tool(ToolName::ProposeCreate).input_schema["properties"]["event"]["properties"]
            ["start"]["properties"];
        assert!(
            start["date_time"]["description"]
                .as_str()
                .unwrap()
                .contains(OFFSET_DATE_TIME_EXAMPLE)
        );
        assert!(
            start["zone"]["description"]
                .as_str()
                .unwrap()
                .contains(ZONE_EXAMPLE)
        );
        // The contract schema kept for the nulls is not changed by rule 4.
        let contract = tool(ToolName::CalendarRead).contract_schema.to_string();
        assert!(!contract.contains(TIMESTAMP_EXAMPLE));
    }

    // t05: filling in: an absent member that may be null becomes null at every depth; a
    // present one, null or not, is untouched; nothing else is added.
    #[test]
    fn t05_fill_nulls() {
        let mut create = json!({
            "account_id": "a",
            "event": {"title": "t", "location": "here"},
        });
        fill_nulls(ToolName::ProposeCreate, &mut create);
        assert_eq!(
            create,
            json!({
                "account_id": "a",
                "event": {"title": "t", "location": "here", "description": null},
            })
        );

        let mut update = json!({"changes": {"title": null, "start": {"date_time": "x"}}});
        fill_nulls(ToolName::ProposeUpdate, &mut update);
        assert_eq!(
            update,
            json!({"changes": {
                "title": null,
                "start": {"date_time": "x"},
                "end": null,
                "description": null,
                "location": null,
            }})
        );

        let mut response = json!({"target": {"kind": "event", "event": {"event_id": "e"}}});
        let before = response.clone();
        fill_nulls(ToolName::ProposeResponse, &mut response);
        assert_eq!(response, before);

        let mut empty = json!({});
        fill_nulls(ToolName::ProposeCreate, &mut empty);
        assert_eq!(empty, json!({}));

        let mut status = json!({"request_id": "r", "extra": 1});
        fill_nulls(ToolName::RequestStatus, &mut status);
        assert_eq!(status, json!({"request_id": "r", "extra": 1}));

        // A member inside an array and inside a branch of a `kind` union. The ten schemas have
        // neither nullable case, so the schema here is made for the test.
        let schema = json!({
            "type": "object",
            "properties": {
                "items": {"type": "array", "items": {
                    "type": "object",
                    "properties": {"a": {"type": "string"}, "n": {"oneOf": [{"type": "null"}, {"type": "string"}]}},
                }},
                "target": {"oneOf": [
                    {"type": "object", "properties": {
                        "kind": {"const": "one"},
                        "m": {"oneOf": [{"type": "null"}, {"type": "string"}]},
                    }},
                    {"type": "object", "properties": {
                        "kind": {"const": "two"},
                        "k": {"oneOf": [{"type": "null"}, {"type": "string"}]},
                    }},
                ]},
            },
        });
        let mut value = json!({
            "items": [{"a": "x"}, {"a": "y", "n": "kept"}, {"n": null}],
            "target": {"kind": "two"},
        });
        fill(&schema, &mut value);
        assert_eq!(
            value,
            json!({
                "items": [{"a": "x", "n": null}, {"a": "y", "n": "kept"}, {"n": null}],
                "target": {"kind": "two", "k": null},
            })
        );
        let mut one = json!({"target": {"kind": "one", "m": "set"}});
        fill(&schema, &mut one);
        assert_eq!(one, json!({"target": {"kind": "one", "m": "set"}}));
        let mut unknown = json!({"target": {"kind": "three"}});
        fill(&schema, &mut unknown);
        assert_eq!(unknown, json!({"target": {"kind": "three"}}));
    }
}
