//! Rendering of facts and actions as HOL terms for check theories.
//!
//! Every value that reaches a check theory goes through this module. Identifiers are limited
//! to characters that cannot end a HOL string literal, and free text becomes a list of
//! character codes, so no input can change the theory's syntax.

use crate::error::PolicyError;
use crate::model::{
    Action, Answer, Candidate, CloseReason, Consent, Context, Decision, Negotiation, Request, Role,
};

/// Renders an identifier as a HOL string literal: 1 to 128 ASCII letters, digits, `.`, `_`
/// or `-`.
pub fn identifier(value: &str) -> Result<String, PolicyError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if valid {
        Ok(format!("''{value}''"))
    } else {
        Err(PolicyError::Render(format!(
            "identifier {value:?} must be 1 to 128 ASCII letters, digits, '.', '_' or '-'"
        )))
    }
}

/// Renders free text as the list of its Unicode code points.
#[must_use]
pub fn note_codes(text: &str) -> String {
    list(
        text.chars()
            .map(|character| u32::from(character).to_string()),
    )
}

/// Renders an integer, parenthesizing negative values so they stay one argument.
#[must_use]
pub fn int(value: i64) -> String {
    if value < 0 {
        format!("({value})")
    } else {
        value.to_string()
    }
}

const fn boolean(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

fn list(items: impl IntoIterator<Item = String>) -> String {
    format!("[{}]", items.into_iter().collect::<Vec<_>>().join(", "))
}

fn record(fields: &[(&str, String)]) -> String {
    let body = fields
        .iter()
        .map(|(name, value)| format!("{name} = {value}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("\\<lparr>{body}\\<rparr>")
}

fn candidate(value: &Candidate) -> Result<String, PolicyError> {
    Ok(record(&[
        ("cand_id", identifier(&value.id)?),
        ("cand_start", int(value.start_ms)),
        ("cand_length", int(value.length_ms)),
    ]))
}

fn candidate_list(values: &[Candidate]) -> Result<String, PolicyError> {
    Ok(list(
        values
            .iter()
            .map(candidate)
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

fn answer(value: &Answer) -> Result<String, PolicyError> {
    Ok(record(&[
        ("ans_id", identifier(&value.candidate_id)?),
        ("ans_available", boolean(value.available).to_owned()),
    ]))
}

fn decision(value: &Decision) -> Result<String, PolicyError> {
    Ok(record(&[
        ("dec_id", identifier(&value.candidate_id)?),
        ("dec_accept", boolean(value.accept).to_owned()),
    ]))
}

const fn role(value: Role) -> &'static str {
    match value {
        Role::Coordinator => "Coordinator",
        Role::Invitee => "Invitee",
    }
}

const fn close_reason(value: CloseReason) -> &'static str {
    match value {
        CloseReason::Agreed => "Agreed",
        CloseReason::NoMatch => "No_match",
        CloseReason::Withdrawn => "Withdrawn",
    }
}

fn request(value: &Request) -> Result<String, PolicyError> {
    Ok(record(&[
        ("req_peer", identifier(&value.peer)?),
        ("req_duration", int(value.duration_ms)),
        ("req_window_start", int(value.window_start_ms)),
        ("req_window_end", int(value.window_end_ms)),
        ("req_opened_at", int(value.opened_at_ms)),
        ("req_expires_at", int(value.expires_at_ms)),
    ]))
}

fn negotiation(value: &Negotiation) -> Result<String, PolicyError> {
    let mutual = value
        .mutual
        .iter()
        .map(|id| identifier(id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(record(&[
        ("neg_id", identifier(&value.id)?),
        ("neg_role", role(value.role).to_owned()),
        ("neg_round", value.round.to_string()),
        ("neg_handshake", boolean(value.handshake_done).to_owned()),
        ("neg_candidates", candidate_list(&value.candidates)?),
        ("neg_mutual", list(mutual)),
    ]))
}

fn consent(value: &Consent) -> Result<String, PolicyError> {
    Ok(record(&[
        ("con_signer", identifier(&value.signer)?),
        ("con_negotiation", identifier(&value.negotiation)?),
        ("con_start", int(value.start_ms)),
        ("con_length", int(value.length_ms)),
        ("con_yes", boolean(value.yes).to_owned()),
        ("con_expires", int(value.expires_ms)),
    ]))
}

/// Renders the gate context as a `gate_context` record.
pub fn context(value: &Context) -> Result<String, PolicyError> {
    let consents = value
        .consents
        .iter()
        .map(consent)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(record(&[
        ("ctx_owner", identifier(&value.owner)?),
        ("ctx_now", int(value.now_ms)),
        ("ctx_request", request(&value.request)?),
        ("ctx_negotiation", negotiation(&value.negotiation)?),
        ("ctx_consents", list(consents)),
    ]))
}

/// Renders an action as a term of the theory's `action` datatype.
pub fn action(value: &Action) -> Result<String, PolicyError> {
    Ok(match value {
        Action::ReadFreeBusy { start_ms, end_ms } => {
            format!("ReadFreeBusy {} {}", int(*start_ms), int(*end_ms))
        }
        Action::ReadEventDetails { start_ms, end_ms } => {
            format!("ReadEventDetails {} {}", int(*start_ms), int(*end_ms))
        }
        Action::Handshake { peer } => format!("Handshake {}", identifier(peer)?),
        Action::SendPropose {
            negotiation,
            round,
            candidates,
            note,
        } => format!(
            "SendPropose {} {round} {} {}",
            identifier(negotiation)?,
            candidate_list(candidates)?,
            note_codes(note)
        ),
        Action::SendRespond {
            negotiation,
            round,
            answers,
            note,
        } => format!(
            "SendRespond {} {round} {} {}",
            identifier(negotiation)?,
            list(answers.iter().map(answer).collect::<Result<Vec<_>, _>>()?),
            note_codes(note)
        ),
        Action::SendDecide {
            negotiation,
            round,
            decisions,
            note,
        } => format!(
            "SendDecide {} {round} {} {}",
            identifier(negotiation)?,
            list(
                decisions
                    .iter()
                    .map(decision)
                    .collect::<Result<Vec<_>, _>>()?
            ),
            note_codes(note)
        ),
        Action::SendClose {
            negotiation,
            reason,
            note,
        } => format!(
            "SendClose {} {} {}",
            identifier(negotiation)?,
            close_reason(*reason),
            note_codes(note)
        ),
        Action::AddMeeting {
            negotiation,
            start_ms,
            length_ms,
        } => format!(
            "AddMeeting {} {} {}",
            identifier(negotiation)?,
            int(*start_ms),
            int(*length_ms)
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::{action, identifier, int, note_codes};
    use crate::model::Action;

    #[test]
    fn identifiers_cannot_escape_the_string_literal() {
        assert_eq!(identifier("bob").unwrap(), "''bob''");
        assert!(identifier("").is_err());
        assert!(identifier("bob'' \\<or> True").is_err());
        assert!(identifier("b\"ob").is_err());
        assert!(identifier(&"x".repeat(129)).is_err());
    }

    #[test]
    fn notes_become_code_points_and_negatives_stay_one_argument() {
        assert_eq!(note_codes("hi é"), "[104, 105, 32, 233]");
        assert_eq!(note_codes(""), "[]");
        assert_eq!(int(-5), "(-5)");
        let read = Action::ReadFreeBusy {
            start_ms: -5,
            end_ms: 10,
        };
        assert_eq!(action(&read).unwrap(), "ReadFreeBusy (-5) 10");
    }
}
