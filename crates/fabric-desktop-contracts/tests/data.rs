//! The Rust constants against `contracts/desktop/v1/data/` and the `const` and `enum`
//! definitions of the schemas.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use fabric_desktop_contracts::agent::{AgentCall, NON_TOOL_METHODS};
use fabric_desktop_contracts::common::ToolName;
use fabric_desktop_contracts::error::ErrorCode;
use fabric_desktop_contracts::handshake::HANDSHAKE_METHODS;
use fabric_desktop_contracts::owner::{
    AgentClient, AgentEnrollment, BuiltInProfile, Destination, Mode, OwnerMethod, PolicyRow,
};
use fabric_desktop_contracts::suite::read_json;
use fabric_desktop_contracts::{
    AUTHORIZATION_LIFETIME_SECONDS, AUTHORIZATION_PROFILE, Limits, PROTOCOL, PlainRequestState,
    RequestState, SCHEMA_VERSION, canonical, decode_record, encode, typed,
};
use serde_json::Value;

fn contract() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/desktop/v1")
}

fn data(name: &str) -> Value {
    read_json(&contract().join("data").join(name)).unwrap()
}

/// Resolves `file.schema.json#/$defs/name`.
fn schema_def(reference: &str) -> Option<Value> {
    let (file, pointer) = reference.split_once('#')?;
    let schema = read_json(&contract().join("schemas").join(file)).ok()?;
    schema.pointer(pointer).cloned()
}

fn names(values: &Value) -> Vec<String> {
    values
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn limits_are_the_cd_g0_values() {
    let file = data("limits.json");
    assert_eq!(canonical(&file["limits"]), encode(&Limits::CD_G0));
}

#[test]
fn versions_match() {
    let file = data("version.json");
    assert_eq!(file["protocol"], PROTOCOL);
    assert_eq!(file["schema_version"], SCHEMA_VERSION);
    assert_eq!(file["authorization_profile"], AUTHORIZATION_PROFILE);
}

#[test]
fn protocol_const_matches() {
    assert_eq!(
        schema_def("common.schema.json#/$defs/protocol").unwrap()["const"],
        PROTOCOL
    );
}

#[test]
fn authorization_lifetime_is_sixty_seconds() {
    assert_eq!(AUTHORIZATION_LIFETIME_SECONDS, 60);
    assert_eq!(Limits::CD_G0.authorization_lifetime_seconds.get(), 60);
    let def = schema_def("owner-channel.schema.json#/$defs/authorization").unwrap();
    assert_eq!(def["properties"]["lifetime_seconds"]["const"], 60);
    assert_eq!(def["properties"]["profile"]["const"], AUTHORIZATION_PROFILE);
}

#[test]
fn tool_names_and_actions_match_the_catalog() {
    let catalog = data("tool-catalog.json");
    let tools = catalog["tools"].as_array().unwrap();
    let rust: Vec<&str> = ToolName::ALL.iter().map(|t| t.as_str()).collect();
    let listed: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(rust, listed);
    for (tool, entry) in ToolName::ALL.iter().zip(tools) {
        assert_eq!(entry["creates_request"], tool.creates_request(), "{tool:?}");
        let reference = entry["arguments"].as_str().unwrap();
        assert_eq!(
            reference,
            format!(
                "agent-channel.schema.json#/$defs/{}",
                tool.arguments_definition()
            )
        );
        assert!(
            schema_def(reference).is_some(),
            "{reference} does not resolve"
        );
    }
    let schema_enum = schema_def("common.schema.json#/$defs/tool_name").unwrap();
    assert_eq!(names(&schema_enum["enum"]), listed);
}

#[test]
fn agent_request_methods_are_the_handshake_the_cancel_and_the_tools() {
    let schema = schema_def("agent-channel.schema.json#/$defs/agent_request").unwrap();
    let listed: Vec<String> = schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            b["properties"]["method"]["const"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let rust: Vec<String> = NON_TOOL_METHODS
        .iter()
        .map(|m| (*m).to_owned())
        .chain(ToolName::ALL.iter().map(|t| t.as_str().to_owned()))
        .collect();
    assert_eq!(rust, listed);
    for method in &listed {
        assert!(AgentCall::is_method(method), "{method}");
    }
    let response = schema_def("agent-channel.schema.json#/$defs/agent_response").unwrap();
    let not_tools: Vec<&str> = response["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|b| b["properties"]["method"]["const"].as_str())
        .collect();
    assert_eq!(not_tools, NON_TOOL_METHODS);
    assert_eq!(NON_TOOL_METHODS[..2], HANDSHAKE_METHODS);
}

#[test]
fn agent_enrollments_and_clients_match_the_schema() {
    let enrollment =
        schema_def("owner-channel.schema.json#/$defs/agent/properties/enrollment").unwrap();
    let rust: Vec<String> = [
        AgentEnrollment::Pending,
        AgentEnrollment::Approved,
        AgentEnrollment::Removed,
    ]
    .iter()
    .map(|e| encode(e).trim_matches('"').to_owned())
    .collect();
    assert_eq!(rust, names(&enrollment["enum"]));
    let client =
        schema_def("owner-channel.schema.json#/$defs/params_add_agent/properties/client").unwrap();
    let rust: Vec<String> = [
        AgentClient::ClaudeDesktop,
        AgentClient::ClaudeCode,
        AgentClient::CodexDesktop,
    ]
    .iter()
    .map(|c| encode(c).trim_matches('"').to_owned())
    .collect();
    assert_eq!(rust, names(&client["enum"]));
}

#[test]
fn agent_clients_are_the_add_agent_clients_with_valid_destinations() {
    let file = data("agent-clients.json");
    let clients = file["clients"].as_array().unwrap();
    let schema =
        schema_def("owner-channel.schema.json#/$defs/params_add_agent/properties/client").unwrap();
    let named: Vec<Value> = clients.iter().map(|e| e["client"].clone()).collect();
    assert_eq!(&named, schema["enum"].as_array().unwrap());
    for entry in clients {
        let client: AgentClient = typed::from_value(&entry["client"]).unwrap();
        assert_eq!(encode(&client), canonical(&entry["client"]));
        let destination: Destination =
            decode_record(canonical(&entry["destination"]).as_bytes()).unwrap();
        assert_eq!(encode(&destination), canonical(&entry["destination"]));
    }
}

#[test]
fn owner_methods_and_commands_match() {
    let file = data("owner-methods.json");
    let methods = file["methods"].as_array().unwrap();
    assert_eq!(methods.len(), OwnerMethod::ALL.len());
    for (method, entry) in OwnerMethod::ALL.iter().zip(methods) {
        assert_eq!(entry["method"], method.as_str());
        assert_eq!(entry["command"], method.is_command(), "{method:?}");
        let o = |name: &str| format!("owner-channel.schema.json#/$defs/{name}");
        assert_eq!(entry["params"], o(method.params_definition()));
        assert_eq!(entry["result"], o(method.result_definition()));
        for key in ["params", "result"] {
            let reference = entry[key].as_str().unwrap();
            assert!(
                schema_def(reference).is_some(),
                "{reference} does not resolve"
            );
        }
    }
}

#[test]
fn policy_rows_match_the_matrix() {
    let file = data("policy-matrix.json");
    let rows = file["rows"].as_array().unwrap();
    let listed: Vec<&str> = rows.iter().map(|r| r["row"].as_str().unwrap()).collect();
    let rust: Vec<&str> = PolicyRow::ALL.iter().map(|r| r.as_str()).collect();
    assert_eq!(rust, listed);
    let schema_enum = schema_def("owner-channel.schema.json#/$defs/policy_row").unwrap();
    assert_eq!(names(&schema_enum["enum"]), listed);
}

#[test]
fn built_in_modes_match_the_matrix() {
    let file = data("policy-matrix.json");
    for entry in file["rows"].as_array().unwrap() {
        let row = PolicyRow::ALL
            .into_iter()
            .find(|r| entry["row"] == r.as_str())
            .unwrap();
        assert_eq!(
            entry["careful"]["mode"],
            BuiltInProfile::Careful.rows().mode(row).as_str()
        );
        assert_eq!(
            entry["flexible"]["mode"],
            BuiltInProfile::Flexible.rows().mode(row).as_str()
        );
    }
}

#[test]
fn custom_modes_are_fixed_only_in_the_fixed_rows() {
    let file = data("policy-matrix.json");
    for entry in file["rows"].as_array().unwrap() {
        let row = PolicyRow::ALL
            .into_iter()
            .find(|r| entry["row"] == r.as_str())
            .unwrap();
        let allowed = names(&entry["custom_modes"]);
        match row.fixed_mode() {
            Some(mode) => assert_eq!(allowed, [mode.as_str()], "{row:?}"),
            None => assert_eq!(allowed.len(), 3, "{row:?}"),
        }
    }
}

#[test]
fn mode_order_is_least_to_most_permissive() {
    let file = data("policy-matrix.json");
    let rust: Vec<&str> = Mode::BY_PERMISSIVENESS.iter().map(|m| m.as_str()).collect();
    assert_eq!(names(&file["modes"]), rust);
    assert!(Mode::Allow.is_wider_than(Mode::Ask));
    assert!(Mode::Ask.is_wider_than(Mode::Block));
    assert!(!Mode::Ask.is_wider_than(Mode::Ask));
}

#[test]
fn built_in_profiles_are_the_schema_constants() {
    for (profile, def) in [
        (BuiltInProfile::Careful, "profile_careful"),
        (BuiltInProfile::Flexible, "profile_flexible"),
    ] {
        let schema = schema_def(&format!("owner-channel.schema.json#/$defs/{def}")).unwrap();
        let props = &schema["properties"];
        let value = profile.to_value();
        for key in ["kind", "profile_id", "name", "version", "rows"] {
            assert_eq!(
                canonical(&props[key]["const"]),
                canonical(&value[key]),
                "{def}.{key}"
            );
        }
    }
}

#[test]
fn fixed_rows_are_the_schema_constants() {
    let rows = schema_def("owner-channel.schema.json#/$defs/profile_rows").unwrap();
    for row in PolicyRow::ALL {
        let prop = &rows["properties"][row.as_str()];
        match row.fixed_mode() {
            Some(mode) => assert_eq!(prop["const"], mode.as_str(), "{row:?}"),
            None => assert_eq!(prop["$ref"], "#/$defs/mode", "{row:?}"),
        }
    }
}

#[test]
fn lifecycle_states_and_terminals_match() {
    let file = data("request-lifecycle.json");
    assert_eq!(file["initial"], RequestState::INITIAL.as_str());
    let states = file["states"].as_array().unwrap();
    assert_eq!(states.len(), RequestState::ALL.len());
    for (state, entry) in RequestState::ALL.iter().zip(states) {
        assert_eq!(entry["state"], state.as_str());
        assert_eq!(entry["terminal"], state.is_terminal(), "{state:?}");
    }
}

#[test]
fn lifecycle_transitions_match_exactly() {
    let file = data("request-lifecycle.json");
    let listed: BTreeSet<(String, String)> = file["transitions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["from"].as_str().unwrap().to_owned(),
                t["to"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let mut rust = BTreeSet::new();
    for from in RequestState::ALL {
        for to in RequestState::ALL {
            if from.can_transition_to(to) {
                rust.insert((from.as_str().to_owned(), to.as_str().to_owned()));
            }
        }
    }
    assert_eq!(rust, listed);
    assert_eq!(rust.len(), 42);
}

#[test]
fn request_state_enum_matches_the_schema() {
    let schema = schema_def("common.schema.json#/$defs/request_state").unwrap();
    let rust: Vec<&str> = RequestState::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(names(&schema["enum"]), rust);
}

#[test]
fn plain_and_carrying_states_are_the_26_states() {
    let schema = schema_def("common.schema.json#/$defs/plain_request_state").unwrap();
    let plain: Vec<&str> = PlainRequestState::ALL
        .iter()
        .map(|p| p.state().as_str())
        .collect();
    assert_eq!(names(&schema["enum"]), plain);
    assert_eq!(plain.len(), 23);
    let carrying = ["pending_approval", "blocked", "terminal_failure"];
    let mut all: BTreeSet<&str> = plain.iter().copied().collect();
    for c in carrying {
        assert!(all.insert(c), "{c} is also plain");
    }
    let from_schema = schema_def("common.schema.json#/$defs/request_state").unwrap();
    let file = data("request-lifecycle.json");
    let from_data: BTreeSet<String> = file["states"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["state"].as_str().unwrap().to_owned())
        .collect();
    let all: BTreeSet<String> = all.into_iter().map(str::to_owned).collect();
    assert_eq!(all, names(&from_schema["enum"]).into_iter().collect());
    assert_eq!(all, from_data);
    for state in RequestState::ALL {
        assert_eq!(state.plain().is_none(), carrying.contains(&state.as_str()));
    }
}

#[test]
fn error_codes_fix_retryable_and_recovery_as_the_schema_does() {
    for code in ErrorCode::ALL {
        let def = schema_def(&format!(
            "common.schema.json#/$defs/error_{}",
            code.as_str()
        ))
        .unwrap_or_else(|| panic!("no schema for {}", code.as_str()));
        let props = &def["properties"];
        assert_eq!(props["code"]["const"], code.as_str());
        assert_eq!(props["retryable"]["const"], code.retryable(), "{code:?}");
        assert_eq!(
            props["recovery"]["const"],
            code.recovery().as_str(),
            "{code:?}"
        );
    }
}

fn error_set(name: &str) -> Vec<String> {
    schema_def(&format!("common.schema.json#/$defs/{name}")).unwrap()["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            b["$ref"]
                .as_str()
                .unwrap()
                .trim_start_matches("#/$defs/error_")
                .to_owned()
        })
        .collect()
}

#[test]
fn owner_and_agent_error_sets_match_the_schema() {
    use fabric_desktop_contracts::error::{AgentFault, Fault, OwnerFault};
    let codes =
        |set: &[ErrorCode]| -> Vec<String> { set.iter().map(|c| c.as_str().to_owned()).collect() };
    assert_eq!(codes(OwnerFault::CODES), error_set("owner_error"));
    assert_eq!(codes(AgentFault::CODES), error_set("agent_error"));
    for code in ErrorCode::ALL {
        let refused = |e: Result<(), String>| matches!(e, Err(m) if m.contains("is not an"));
        let owner = OwnerFault::from_parts(code, Value::Null).map(|_| ());
        let agent = AgentFault::from_parts(code, Value::Null).map(|_| ());
        assert_eq!(
            refused(owner),
            !OwnerFault::CODES.contains(&code),
            "{code:?}"
        );
        assert_eq!(
            refused(agent),
            !AgentFault::CODES.contains(&code),
            "{code:?}"
        );
    }
}
