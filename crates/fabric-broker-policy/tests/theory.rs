//! The Rust mirrors agree with `Fabric_Broker.thy` as written: constructor names and order,
//! the block and approval partition, record field order, and the two built-in profiles.

mod common;

use fabric_broker_policy::{
    EventField, Mode, Outcome, Profile, Response, Row, Rule, SendUpdates, Tool, hol,
};
use fabric_desktop_contracts::owner::ProfileRows;

const THEORY: &str = include_str!("../../../isabelle/Fabric_Broker/Fabric_Broker.thy");

/// The text from `header` to the next blank line.
fn block(header: &str) -> String {
    let start = THEORY
        .find(header)
        .unwrap_or_else(|| panic!("{header} is in the theory"));
    let rest = &THEORY[start + header.len()..];
    rest[..rest.find("\n\n").unwrap_or(rest.len())].to_owned()
}

/// The constructor names of `datatype name = ...`, in order.
fn constructors(name: &str) -> Vec<String> {
    block(&format!("datatype {name} ="))
        .split('|')
        .map(|part| part.split_whitespace().next().unwrap().to_owned())
        .collect()
}

/// The field names of `record name =`, in order.
fn fields(name: &str) -> Vec<String> {
    block(&format!("record {name} ="))
        .lines()
        .filter_map(|line| line.split_once("::").map(|(f, _)| f.trim().to_owned()))
        .collect()
}

/// The items of a list between `[` and `]` after `header`.
fn list_after(header: &str) -> Vec<String> {
    let text = block(header);
    let inner = &text[text.find('[').unwrap() + 1..text.find(']').unwrap()];
    inner
        .split(',')
        .map(|item| item.trim().to_owned())
        .collect()
}

/// The `p_x = Mode` pairs of `definition name :: profile`.
fn profile(name: &str) -> Vec<(String, String)> {
    let text = block(&format!("definition {name} :: profile where"));
    let inner = &text[text.find("\\<lparr>").unwrap() + 8..text.find("\\<rparr>").unwrap()];
    inner
        .split(',')
        .map(|pair| {
            let (field, mode) = pair.split_once('=').unwrap();
            (field.trim().to_owned(), mode.trim().to_owned())
        })
        .collect()
}

fn names<T: Copy>(items: &[T], name: fn(T) -> &'static str) -> Vec<String> {
    items.iter().map(|&item| name(item).to_owned()).collect()
}

#[test]
fn rules_and_their_partition_are_the_theorys() {
    assert_eq!(
        constructors("rule"),
        names(&Rule::ALL, Rule::hol_constructor)
    );
    let block_rules = list_after("definition block_rules :: \"rule list\" where");
    let approval_rules = list_after("definition approval_rules :: \"rule list\" where");
    let rust_block: Vec<Rule> = Rule::ALL
        .into_iter()
        .filter(|r| r.is_block_rule())
        .collect();
    let rust_approval: Vec<Rule> = Rule::ALL
        .into_iter()
        .filter(|r| !r.is_block_rule())
        .collect();
    assert_eq!(block_rules, names(&rust_block, Rule::hol_constructor));
    assert_eq!(approval_rules, names(&rust_approval, Rule::hol_constructor));
    // `Rule::ALL` is `block_rules @ approval_rules`, the order `first_failing` uses.
    let mut both = block_rules;
    both.extend(approval_rules);
    assert_eq!(both, names(&Rule::ALL, Rule::hol_constructor));
    for rule in Rule::ALL {
        assert_eq!(rule.name(), rule.hol_constructor().to_ascii_lowercase());
        assert!(fabric_desktop_contracts::scalar::NameToken::is_valid(
            rule.name()
        ));
        assert!(fabric_desktop_contracts::scalar::Explanation::is_valid(
            rule.explanation()
        ));
    }
}

#[test]
fn the_other_datatypes_are_the_theorys() {
    assert_eq!(constructors("row"), names(&Row::ALL, Row::hol_constructor));
    assert_eq!(
        constructors("mode"),
        names(
            &[Mode::Blocks, Mode::Asks, Mode::Allows],
            Mode::hol_constructor
        )
    );
    let tools = [
        Tool::List,
        Tool::Availability,
        Tool::Read,
        Tool::Create,
        Tool::Update,
        Tool::Invite,
        Tool::Respond,
        Tool::Delete,
    ];
    assert_eq!(constructors("tool"), names(&tools, Tool::hol_constructor));
    let fields_ = [
        EventField::Time,
        EventField::Title,
        EventField::Location,
        EventField::Description,
        EventField::Attendees,
        EventField::Organizer,
        EventField::OwnResponse,
    ];
    assert_eq!(
        constructors("event_field"),
        names(&fields_, EventField::hol_constructor)
    );
    let updates = [
        SendUpdates::NotifyAll,
        SendUpdates::NotifyExternal,
        SendUpdates::NotifyNone,
    ];
    assert_eq!(
        constructors("send_updates"),
        names(&updates, SendUpdates::hol_constructor)
    );
    assert_eq!(
        constructors("response"),
        names(
            &[Response::Accepted, Response::Declined],
            Response::hol_constructor
        )
    );
    assert_eq!(
        constructors("outcome"),
        names(&Outcome::ALL, Outcome::hol_constructor)
    );
    assert_eq!(
        constructors("duration"),
        ["One_request", "Session", "Until_revoked"]
    );
    assert_eq!(constructors("recipient_rule"), ["Exact", "Sets"]);
    assert_eq!(constructors("actor"), ["Agent", "System"]);
    assert_eq!(constructors("target"), ["Whole_event", "Occurrence"]);
    assert_eq!(
        constructors("action"),
        [
            "List_calendars",
            "Read_availability",
            "Read_events",
            "Create_event",
            "Update_event",
            "Change_invitation",
            "Respond",
            "Delete_event",
            "Disclose",
            "Disclose_warning",
            "Delete_calendar",
            "Change_sharing",
            "System_refresh",
            "System_reconcile",
        ]
    );
}

/// The field names of the rendered record term that starts at `\<lparr>` in `term`, at the
/// top level only.
fn rendered_fields(term: &str) -> Vec<String> {
    let body = &term[term.find("\\<lparr>").unwrap() + 8..term.rfind("\\<rparr>").unwrap()];
    let (mut depth, mut start, mut parts) = (0_i32, 0, Vec::new());
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if body[i..].starts_with("\\<lparr>") {
            depth += 1;
            i += 8;
            continue;
        }
        if body[i..].starts_with("\\<rparr>") {
            depth -= 1;
            i += 8;
            continue;
        }
        match bytes[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b',' if depth == 0 => {
                parts.push(body[start..i].to_owned());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(body[start..].to_owned());
    parts
        .iter()
        .map(|part| part.split('=').next().unwrap().trim().to_owned())
        .collect()
}

#[test]
fn rendered_records_have_the_theorys_fields_in_order() {
    let context = common::context();
    let rendered = hol::context(&context).unwrap();
    assert_eq!(rendered_fields(&rendered), fields("broker_context"));
    assert_eq!(
        rendered_fields(&hol::profile(&Profile::CAREFUL)),
        fields("profile")
    );
    let profile_fields: Vec<String> = Row::ALL
        .iter()
        .map(|row| row.profile_field().to_owned())
        .collect();
    assert_eq!(profile_fields, fields("profile"));
    // The nested records: render each alone through a context that holds just it.
    let between = |from: &str| {
        let at = rendered.find(from).unwrap() + from.len();
        rendered[at..].to_owned()
    };
    for (name, from) in [
        ("calendar", "ctx_calendars = ["),
        ("grant", "ctx_grants = ["),
        ("recipient_set", "ctx_recipient_sets = ["),
        ("event", "ctx_event = Some ("),
        ("recipient", "ev_attendees = ["),
    ] {
        let text = between(from);
        // The first record term in `text`: up to its matching `\<rparr>`.
        let mut depth = 0;
        let mut end = 0;
        let mut i = 0;
        while i < text.len() {
            if text[i..].starts_with("\\<lparr>") {
                depth += 1;
                i += 8;
            } else if text[i..].starts_with("\\<rparr>") {
                depth -= 1;
                i += 8;
                if depth == 0 {
                    end = i;
                    break;
                }
            } else {
                i += 1;
            }
        }
        assert_eq!(rendered_fields(&text[..end]), fields(name), "{name}");
    }
}

#[test]
fn the_built_in_profiles_are_the_theorys_and_the_contracts() {
    for (name, rust, rows) in [
        ("careful", Profile::CAREFUL, ProfileRows::CAREFUL),
        ("flexible", Profile::FLEXIBLE, ProfileRows::FLEXIBLE),
    ] {
        let expected: Vec<(String, String)> = Row::ALL
            .iter()
            .map(|&row| {
                (
                    row.profile_field().to_owned(),
                    rust.field(row).hol_constructor().to_owned(),
                )
            })
            .collect();
        assert_eq!(profile(name), expected, "{name}");
        assert_eq!(Profile::from(&rows), rust, "{name}");
    }
}

#[test]
fn the_fixed_rows_of_mode_of_are_the_contracts() {
    assert!(THEORY.contains("\"mode_of p Row_delete = Asks\""));
    assert!(THEORY.contains("\"mode_of p Row_sharing = Blocks\""));
    let mut rows = ProfileRows::FLEXIBLE;
    rows.respond_to_invitation = fabric_desktop_contracts::owner::Mode::Block;
    let profile = Profile::from(&rows);
    assert_eq!(profile.respond, Mode::Blocks);
    assert_eq!(profile.delete, Mode::Asks);
    assert_eq!(profile.sharing, Mode::Blocks);
}
