//! Details for denial messages.
//!
//! Isabelle decides which rules fail. These strings only describe the failed condition, from
//! the same facts the check theory used, with counts from Isabelle's `value` output and limits
//! from the parameters the gate read out of the theory.

use crate::check::Quantities;
use crate::model::{Action, Candidate, Context, Parameters, Role, Rule};
use crate::time::format_utc;

const MINUTE_MS: i64 = 60 * 1_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;

/// Describes why `rule` fails for `action`.
#[must_use]
pub fn detail(
    rule: Rule,
    context: &Context,
    action: &Action,
    quantities: Quantities,
    parameters: &Parameters,
) -> String {
    let request = &context.request;
    match rule {
        Rule::ViewDetailsNever => "event details are never readable by an agent".to_owned(),
        Rule::ReadWindow => match action {
            Action::ReadFreeBusy { start_ms, end_ms } => format!(
                "{}..{} is not inside the request window {}..{}",
                format_utc(*start_ms),
                format_utc(*end_ms),
                format_utc(request.window_start_ms),
                format_utc(request.window_end_ms)
            ),
            _ => String::new(),
        },
        Rule::CandCount => format!(
            "{} candidates; 1 to {} allowed",
            count(quantities, action),
            parameters.max_candidates
        ),
        Rule::AnswerCount => format!(
            "{} answers; at most {} allowed",
            count(quantities, action),
            parameters.max_candidates
        ),
        Rule::RoundLimit => format!(
            "round {}; 1 to {} allowed",
            round(action).unwrap_or(0),
            parameters.max_rounds
        ),
        Rule::Horizon => horizon(context, action, parameters),
        Rule::Grid => grid(context, action, parameters),
        Rule::ConsentLifetime => {
            let span = request.expires_at_ms - request.opened_at_ms;
            if span > parameters.consent_ttl_ms {
                format!(
                    "the negotiation runs {} hours; at most {} allowed",
                    span / HOUR_MS,
                    parameters.consent_ttl_ms / HOUR_MS
                )
            } else {
                format!(
                    "the negotiation expired at {}",
                    format_utc(request.expires_at_ms)
                )
            }
        }
        Rule::Note => note(action, quantities, parameters),
        Rule::HandshakeTarget => match action {
            Action::Handshake { peer } => {
                format!("{peer} is not the requested peer {}", request.peer)
            }
            _ => String::new(),
        },
        Rule::Consent => consent(context, action),
        Rule::WellFormed => well_formed(context, action),
    }
}

fn count(quantities: Quantities, action: &Action) -> u64 {
    quantities.payload_count.unwrap_or_else(|| {
        let length = match action {
            Action::SendPropose { candidates, .. } => candidates.len(),
            Action::SendRespond { answers, .. } => answers.len(),
            Action::SendDecide { decisions, .. } => decisions.len(),
            _ => 0,
        };
        u64::try_from(length).unwrap_or(u64::MAX)
    })
}

const fn round(action: &Action) -> Option<u32> {
    match action {
        Action::SendPropose { round, .. }
        | Action::SendRespond { round, .. }
        | Action::SendDecide { round, .. } => Some(*round),
        _ => None,
    }
}

fn checked_candidates<'a>(context: &'a Context, action: &'a Action) -> &'a [Candidate] {
    match action {
        Action::SendPropose { candidates, .. } => candidates,
        Action::SendRespond { .. } => &context.negotiation.candidates,
        _ => &[],
    }
}

fn horizon(context: &Context, action: &Action, parameters: &Parameters) -> String {
    let request = &context.request;
    let span = request.window_end_ms - request.window_start_ms;
    if span > parameters.horizon_ms {
        return format!(
            "the request window spans {} hours; at most {} allowed",
            span / HOUR_MS,
            parameters.horizon_ms / HOUR_MS
        );
    }
    let outside: Vec<String> = checked_candidates(context, action)
        .iter()
        .filter(|candidate| {
            candidate.start_ms < request.window_start_ms
                || candidate.start_ms + candidate.length_ms > request.window_end_ms
                || candidate.start_ms <= context.now_ms
        })
        .map(|candidate| format!("{} at {}", candidate.id, format_utc(candidate.start_ms)))
        .collect();
    format!(
        "{} outside the window {}..{} or not in the future",
        outside.join(", "),
        format_utc(request.window_start_ms),
        format_utc(request.window_end_ms)
    )
}

fn grid(context: &Context, action: &Action, parameters: &Parameters) -> String {
    let duration = context.request.duration_ms;
    let problems: Vec<String> = checked_candidates(context, action)
        .iter()
        .filter_map(|candidate| {
            if candidate.start_ms.rem_euclid(parameters.grid_ms) != 0 {
                Some(format!(
                    "{} starts at {}, off the {}-minute grid",
                    candidate.id,
                    format_utc(candidate.start_ms),
                    parameters.grid_ms / MINUTE_MS
                ))
            } else if candidate.length_ms != duration {
                Some(format!(
                    "{} lasts {} minutes instead of {}",
                    candidate.id,
                    candidate.length_ms / MINUTE_MS,
                    duration / MINUTE_MS
                ))
            } else {
                None
            }
        })
        .collect();
    problems.join("; ")
}

fn note(action: &Action, quantities: Quantities, parameters: &Parameters) -> String {
    let text = action.note().unwrap_or("");
    let length = quantities
        .note_length
        .unwrap_or_else(|| u64::try_from(text.chars().count()).unwrap_or(u64::MAX));
    let mut problems = Vec::new();
    if length > parameters.note_max {
        problems.push(format!(
            "{length} characters; at most {}",
            parameters.note_max
        ));
    }
    if text
        .chars()
        .any(|character| !(' '..='~').contains(&character))
    {
        problems.push("characters outside printable ASCII".to_owned());
    }
    if text.chars().any(|character| character.is_ascii_digit()) {
        problems.push("digits".to_owned());
    }
    format!("the note has {}", problems.join(" and "))
}

fn consent(context: &Context, action: &Action) -> String {
    let Action::AddMeeting {
        negotiation,
        start_ms,
        length_ms,
    } = action
    else {
        return String::new();
    };
    let said_yes = |who: &str| {
        context.consents.iter().any(|record| {
            record.signer == who
                && record.negotiation == *negotiation
                && record.start_ms == *start_ms
                && record.length_ms == *length_ms
                && record.yes
                && context.now_ms < record.expires_ms
        })
    };
    let missing: Vec<&str> = [context.owner.as_str(), context.request.peer.as_str()]
        .into_iter()
        .filter(|who| !said_yes(who))
        .collect();
    format!(
        "no valid yes from {} for the meeting at {}",
        missing.join(" or "),
        format_utc(*start_ms)
    )
}

fn well_formed(context: &Context, action: &Action) -> String {
    let negotiation = &context.negotiation;
    let (id, problems) = match action {
        Action::SendPropose {
            negotiation: id,
            round,
            candidates,
            ..
        } => {
            let mut problems = Vec::new();
            if negotiation.role != Role::Coordinator {
                problems.push("only the coordinator proposes".to_owned());
            }
            if *round != negotiation.round + 1 {
                problems.push(format!(
                    "round {round}, but the next round is {}",
                    negotiation.round + 1
                ));
            }
            let mut ids: Vec<&str> = candidates.iter().map(|c| c.id.as_str()).collect();
            ids.sort_unstable();
            if ids.windows(2).any(|pair| pair[0] == pair[1]) {
                problems.push("candidate IDs repeat".to_owned());
            }
            (id, problems)
        }
        Action::SendRespond {
            negotiation: id,
            round,
            answers,
            ..
        } => {
            let mut problems = Vec::new();
            if negotiation.role != Role::Invitee {
                problems.push("only the invitee responds".to_owned());
            }
            if *round != negotiation.round {
                problems.push(format!(
                    "round {round}, but the current round is {}",
                    negotiation.round
                ));
            }
            let answered: Vec<&str> = answers.iter().map(|a| a.candidate_id.as_str()).collect();
            let received: Vec<&str> = negotiation
                .candidates
                .iter()
                .map(|c| c.id.as_str())
                .collect();
            if answered != received {
                problems.push(format!(
                    "answers cover {answered:?}, but the received candidates are {received:?}"
                ));
            }
            (id, problems)
        }
        Action::SendDecide {
            negotiation: id,
            round,
            decisions,
            ..
        } => {
            let mut problems = Vec::new();
            if *round != negotiation.round {
                problems.push(format!(
                    "round {round}, but the current round is {}",
                    negotiation.round
                ));
            }
            let decided: Vec<&str> = decisions.iter().map(|d| d.candidate_id.as_str()).collect();
            if decided
                != negotiation
                    .mutual
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            {
                problems.push(format!(
                    "decisions cover {decided:?}, but the mutual options are {:?}",
                    negotiation.mutual
                ));
            }
            let differs = decisions.iter().any(|decision| {
                !negotiation.candidates.iter().any(|candidate| {
                    candidate.id == decision.candidate_id
                        && context.consents.iter().any(|record| {
                            record.signer == context.owner
                                && record.negotiation == negotiation.id
                                && record.start_ms == candidate.start_ms
                                && record.length_ms == candidate.length_ms
                                && record.yes == decision.accept
                        })
                })
            });
            if differs {
                problems.push("a decision differs from your owner's signed record".to_owned());
            }
            (id, problems)
        }
        Action::SendClose {
            negotiation: id, ..
        } => (id, Vec::new()),
        _ => return String::new(),
    };

    let mut all = Vec::new();
    if !negotiation.handshake_done {
        all.push("no open session with the peer".to_owned());
    }
    if *id != negotiation.id {
        all.push(format!(
            "negotiation {id} is not the active one ({})",
            negotiation.id
        ));
    }
    all.extend(problems);
    all.join("; ")
}
