use std::error::Error;
use std::process::ExitCode;

use fabric_core::{
    Candidate, CandidateId, CandidateRound, Eligibility, LocalCalendar, MeetingRequest, MemberId,
    NegotiationId, OwnerDecision, ProjectionProfile, SchedulingAgent, TwoAgentNegotiation,
    UtcInterval,
};

const OPENED_AT: i64 = 1_800_000_000_000;
const NOW: i64 = OPENED_AT + 1_000;
const MEETING_SECONDS: u32 = 30 * 60;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fabric: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let command = std::env::args().nth(1);
    match command.as_deref() {
        None | Some("status") => {
            println!("{}", fabric_core::status());
            println!("Run `fabric demo-meeting` for the executable two-agent slice.");
            Ok(())
        }
        Some("demo-meeting") => demo_meeting(),
        Some(other) => {
            Err(format!("unknown command `{other}`; expected `status` or `demo-meeting`").into())
        }
    }
}

fn demo_meeting() -> Result<(), Box<dyn Error>> {
    let alice_id = MemberId::new("alice")?;
    let bob_id = MemberId::new("bob")?;
    let request = MeetingRequest::new(
        NegotiationId::new("demo-two-agent-001")?,
        alice_id.clone(),
        bob_id.clone(),
        UtcInterval::new(OPENED_AT, 7 * 24 * 60 * 60)?,
        MEETING_SECONDS,
        ProjectionProfile::FabricOnly,
        OPENED_AT,
        OPENED_AT + 86_400_000,
    )?;
    let candidates = [3_600_000_i64, 7_200_000, 10_800_000]
        .into_iter()
        .enumerate()
        .map(|(index, offset)| {
            Ok(Candidate::new(
                CandidateId::new(format!("c{}", index + 1))?,
                UtcInterval::new(OPENED_AT + offset, MEETING_SECONDS)?,
            ))
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let round = CandidateRound::new(request, candidates)?;

    // These synthetic calendars are independent private agent state. Candidate-level
    // yes/no bits are intentionally disclosed, but underlying events and reasons are not.
    let alice = SchedulingAgent::new(
        alice_id,
        LocalCalendar::from_busy(vec![UtcInterval::new(OPENED_AT + 42_000_000, 900)?]),
    );
    let bob = SchedulingAgent::new(
        bob_id,
        LocalCalendar::from_busy(vec![round.candidates()[0].interval()]),
    );
    let alice_eligibility = alice.evaluate(&round, NOW)?;
    let bob_eligibility = bob.evaluate(&round, NOW)?;

    println!("protocol=fabric-schedule-sim/0");
    println!("transport=in-process e2ee=false provider_writes=false");
    println!("output=coordinator-sensitive-debug candidate_bits_disclosed=true");
    println!("negotiation={}", round.request().negotiation_id());
    for candidate in round.candidates() {
        println!(
            "candidate={} start_utc_ms={} duration_seconds={}",
            candidate.id(),
            candidate.interval().start_utc_ms(),
            candidate.interval().duration_seconds()
        );
    }
    println!(
        "eligibility member={} values={}",
        alice_eligibility.member_id(),
        format_eligibility(alice_eligibility.values())
    );
    println!(
        "eligibility member={} values={}",
        bob_eligibility.member_id(),
        format_eligibility(bob_eligibility.values())
    );

    // Each endpoint independently evaluates the same transcript. Opposite arrival
    // orders demonstrate that delivery order does not control slot selection.
    let mut alice_view = TwoAgentNegotiation::new(round.clone(), NOW)?;
    let mut bob_view = TwoAgentNegotiation::new(round, NOW)?;
    alice_view.submit_eligibility(bob_eligibility.clone(), NOW)?;
    alice_view.submit_eligibility(alice_eligibility.clone(), NOW)?;
    bob_view.submit_eligibility(alice_eligibility, NOW)?;
    bob_view.submit_eligibility(bob_eligibility, NOW)?;
    let options = alice_view
        .mutual_options()
        .ok_or("the demo unexpectedly produced no mutual options")?
        .clone();
    if bob_view.mutual_options() != Some(&options) {
        return Err("the two endpoint views derived different mutual options".into());
    }
    println!(
        "mutual_options={}",
        options
            .candidates()
            .iter()
            .map(|candidate| candidate.id().as_str())
            .collect::<Vec<_>>()
            .join(",")
    );

    let alice_decisions = alice.record_owner_decisions(
        &options,
        &[
            OwnerDecision::new(CandidateId::new("c2")?, true),
            OwnerDecision::new(CandidateId::new("c3")?, true),
        ],
        NOW,
    )?;
    let bob_decisions = bob.record_owner_decisions(
        &options,
        &[
            OwnerDecision::new(CandidateId::new("c2")?, false),
            OwnerDecision::new(CandidateId::new("c3")?, true),
        ],
        NOW,
    )?;
    println!(
        "decisions member={} values={}",
        alice_decisions.member_id(),
        format_decisions(alice_decisions.values())
    );
    println!(
        "decisions member={} values={}",
        bob_decisions.member_id(),
        format_decisions(bob_decisions.values())
    );

    alice_view.submit_decisions(alice_decisions.clone(), NOW)?;
    alice_view.submit_decisions(bob_decisions.clone(), NOW)?;
    bob_view.submit_decisions(bob_decisions, NOW)?;
    bob_view.submit_decisions(alice_decisions, NOW)?;
    let alice_record = alice_view
        .agreement()
        .ok_or("Alice's view unexpectedly produced no agreement")?
        .clone();
    let bob_record = bob_view
        .agreement()
        .ok_or("Bob's view unexpectedly produced no agreement")?
        .clone();
    if alice_record != bob_record {
        return Err("the two endpoint views derived different agreements".into());
    }

    println!("result={}", alice_view.state());
    println!(
        "selected={} start_utc_ms={} duration_seconds={}",
        alice_record.selected_candidate().id(),
        alice_record.selected_candidate().interval().start_utc_ms(),
        alice_record
            .selected_candidate()
            .interval()
            .duration_seconds()
    );
    println!("agreement_key={}", alice_record.agreement_key());
    println!(
        "agents_hold_identical_record={}",
        alice_record == bob_record
    );
    println!("scheduled=false");
    Ok(())
}

fn format_eligibility(values: &[Eligibility]) -> String {
    values
        .iter()
        .map(|value| {
            format!(
                "{}:{}",
                value.candidate_id(),
                if value.is_available() { "yes" } else { "no" }
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn format_decisions(values: &[OwnerDecision]) -> String {
    values
        .iter()
        .map(|value| {
            format!(
                "{}:{}",
                value.candidate_id(),
                if value.is_accepted() { "yes" } else { "no" }
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}
