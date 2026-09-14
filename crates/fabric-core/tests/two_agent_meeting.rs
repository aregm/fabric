use std::error::Error;

use fabric_core::{
    Candidate, CandidateId, CandidateRound, LocalCalendar, MeetingRequest, MemberId, NegotiationId,
    NegotiationState, OwnerDecision, ProjectionProfile, RendezvousError, SchedulingAgent,
    SubmissionOutcome, TwoAgentNegotiation, UtcInterval,
};

const OPENED_AT: i64 = 1_800_000_000_000;
const NOW: i64 = OPENED_AT + 1_000;
const EXPIRES_AT: i64 = OPENED_AT + 86_400_000;
const HORIZON_SECONDS: u32 = 7 * 24 * 60 * 60;
const MEETING_SECONDS: u32 = 30 * 60;

fn id(value: &str) -> CandidateId {
    CandidateId::new(value).expect("fixture candidate ID")
}

fn fixture_round(negotiation: &str) -> CandidateRound {
    let request = MeetingRequest::new(
        NegotiationId::new(negotiation).expect("fixture negotiation ID"),
        MemberId::new("alice").expect("fixture member ID"),
        MemberId::new("bob").expect("fixture member ID"),
        UtcInterval::new(OPENED_AT, HORIZON_SECONDS).expect("fixture horizon"),
        MEETING_SECONDS,
        ProjectionProfile::FabricOnly,
        OPENED_AT,
        EXPIRES_AT,
    )
    .expect("fixture request");

    CandidateRound::new(
        request,
        [3_600_000_i64, 7_200_000, 10_800_000]
            .into_iter()
            .enumerate()
            .map(|(index, offset)| {
                Candidate::new(
                    id(&format!("c{}", index + 1)),
                    UtcInterval::new(OPENED_AT + offset, MEETING_SECONDS)
                        .expect("fixture candidate"),
                )
            })
            .collect(),
    )
    .expect("fixture round")
}

fn agent(member: &str, round: &CandidateRound, availability_bits: u8) -> SchedulingAgent {
    let busy = round
        .candidates()
        .iter()
        .enumerate()
        .filter(|(index, _)| availability_bits & (1 << index) == 0)
        .map(|(_, candidate)| candidate.interval())
        .collect();
    SchedulingAgent::new(
        MemberId::new(member).expect("fixture member ID"),
        LocalCalendar::from_busy(busy),
    )
}

fn owner_decisions(options: &[Candidate], decision_bits: u8) -> Vec<OwnerDecision> {
    options
        .iter()
        .map(|candidate| {
            let index = usize::from(candidate.id().as_str().as_bytes()[1] - b'1');
            OwnerDecision::new(candidate.id().clone(), decision_bits & (1 << index) != 0)
        })
        .collect()
}

#[test]
fn two_agents_agree_on_the_first_unanimous_option() -> Result<(), Box<dyn Error>> {
    let round = fixture_round("happy-path");
    let alice = agent("alice", &round, 0b111);
    let bob = agent("bob", &round, 0b110);
    let alice_eligibility = alice.evaluate(&round, NOW)?;
    let bob_eligibility = bob.evaluate(&round, NOW)?;
    let mut alice_view = TwoAgentNegotiation::new(round.clone(), NOW)?;
    let mut bob_view = TwoAgentNegotiation::new(round, NOW)?;

    // Response arrival order does not confer selection priority.
    alice_view.submit_eligibility(bob_eligibility.clone(), NOW)?;
    assert_eq!(
        alice_view.submit_eligibility(bob_eligibility.clone(), EXPIRES_AT)?,
        SubmissionOutcome::DuplicateIgnored
    );
    alice_view.submit_eligibility(alice_eligibility.clone(), NOW)?;
    bob_view.submit_eligibility(alice_eligibility, NOW)?;
    bob_view.submit_eligibility(bob_eligibility, NOW)?;
    assert_eq!(alice_view.state(), NegotiationState::AwaitingDecisions);
    assert_eq!(bob_view.state(), NegotiationState::AwaitingDecisions);

    let options = alice_view
        .mutual_options()
        .expect("two mutual options")
        .clone();
    assert_eq!(bob_view.mutual_options(), Some(&options));
    assert_eq!(
        options
            .candidates()
            .iter()
            .map(|candidate| candidate.id().as_str())
            .collect::<Vec<_>>(),
        ["c2", "c3"]
    );

    let alice_decision = alice.record_owner_decisions(
        &options,
        &[
            OwnerDecision::new(id("c2"), true),
            OwnerDecision::new(id("c3"), true),
        ],
        NOW,
    )?;
    let bob_decision = bob.record_owner_decisions(
        &options,
        &[
            OwnerDecision::new(id("c2"), false),
            OwnerDecision::new(id("c3"), true),
        ],
        NOW,
    )?;
    alice_view.submit_decisions(alice_decision.clone(), NOW)?;
    alice_view.submit_decisions(bob_decision.clone(), NOW)?;
    assert_eq!(
        alice_view.submit_decisions(bob_decision.clone(), EXPIRES_AT)?,
        SubmissionOutcome::DuplicateIgnored
    );
    bob_view.submit_decisions(bob_decision, NOW)?;
    bob_view.submit_decisions(alice_decision, NOW)?;

    assert_eq!(alice_view.state(), NegotiationState::Agreed);
    assert_eq!(bob_view.state(), NegotiationState::Agreed);
    let alice_record = alice_view.agreement().expect("Alice agreement");
    let bob_record = bob_view.agreement().expect("Bob agreement");
    assert_eq!(alice_record.selected_candidate().id().as_str(), "c3");
    assert_eq!(alice_record, bob_record);
    assert_eq!(alice_record.agreement_key(), bob_record.agreement_key());
    Ok(())
}

#[test]
fn all_boolean_combinations_select_exactly_the_first_unanimous_candidate()
-> Result<(), Box<dyn Error>> {
    for eligibility_bits in 0_u8..64 {
        for decision_bits in 0_u8..64 {
            let round = fixture_round("exhaustive");
            let alice_eligible = eligibility_bits & 0b111;
            let bob_eligible = (eligibility_bits >> 3) & 0b111;
            let alice_decisions = decision_bits & 0b111;
            let bob_decisions = (decision_bits >> 3) & 0b111;
            let alice = agent("alice", &round, alice_eligible);
            let bob = agent("bob", &round, bob_eligible);
            let mut negotiation = TwoAgentNegotiation::new(round.clone(), NOW)?;

            negotiation.submit_eligibility(alice.evaluate(&round, NOW)?, NOW)?;
            negotiation.submit_eligibility(bob.evaluate(&round, NOW)?, NOW)?;

            let expected = (0..3).find(|index| {
                let bit = 1 << index;
                alice_eligible & bit != 0
                    && bob_eligible & bit != 0
                    && alice_decisions & bit != 0
                    && bob_decisions & bit != 0
            });

            if let Some(options) = negotiation.mutual_options().cloned() {
                let alice_choices = owner_decisions(options.candidates(), alice_decisions);
                let bob_choices = owner_decisions(options.candidates(), bob_decisions);
                negotiation.submit_decisions(
                    alice.record_owner_decisions(&options, &alice_choices, NOW)?,
                    NOW,
                )?;
                negotiation.submit_decisions(
                    bob.record_owner_decisions(&options, &bob_choices, NOW)?,
                    NOW,
                )?;
            }

            match expected {
                Some(index) => {
                    assert_eq!(negotiation.state(), NegotiationState::Agreed);
                    assert_eq!(
                        negotiation
                            .agreement()
                            .expect("expected agreement")
                            .selected_candidate()
                            .id()
                            .as_str(),
                        format!("c{}", index + 1)
                    );
                }
                None => assert_eq!(negotiation.state(), NegotiationState::NoMatch),
            }
        }
    }
    Ok(())
}

#[test]
fn changed_candidate_scope_and_conflicting_replay_are_rejected() -> Result<(), Box<dyn Error>> {
    let round = fixture_round("scope-check");
    let first_alice = agent("alice", &round, 0b111);
    let conflicting_alice = agent("alice", &round, 0b110);
    let mut negotiation = TwoAgentNegotiation::new(round.clone(), NOW)?;

    negotiation.submit_eligibility(first_alice.evaluate(&round, NOW)?, NOW)?;
    let replay_error = negotiation
        .submit_eligibility(conflicting_alice.evaluate(&round, NOW)?, NOW)
        .expect_err("different replay must fail");
    assert_eq!(
        replay_error,
        RendezvousError::ConflictingReplay("alice".to_owned())
    );

    let request = round.request().clone();
    let changed_round = CandidateRound::new(
        request,
        vec![Candidate::new(
            id("c1"),
            UtcInterval::new(OPENED_AT + 60_000, MEETING_SECONDS)?,
        )],
    )?;
    let changed_response = first_alice.evaluate(&changed_round, NOW)?;
    assert_eq!(
        negotiation
            .submit_eligibility(changed_response, NOW)
            .expect_err("altered exact candidate scope must fail"),
        RendezvousError::EnvelopeScopeMismatch
    );
    Ok(())
}

#[test]
fn decisions_require_a_complete_ordered_vector_and_reject_replay_or_scope_changes()
-> Result<(), Box<dyn Error>> {
    let round = fixture_round("decision-scope");
    let alice = agent("alice", &round, 0b111);
    let bob = agent("bob", &round, 0b111);
    let mut negotiation = TwoAgentNegotiation::new(round.clone(), NOW)?;
    negotiation.submit_eligibility(alice.evaluate(&round, NOW)?, NOW)?;
    negotiation.submit_eligibility(bob.evaluate(&round, NOW)?, NOW)?;
    let options = negotiation.mutual_options().expect("all mutual").clone();

    assert_eq!(
        alice
            .record_owner_decisions(
                &options,
                &[
                    OwnerDecision::new(id("c1"), true),
                    OwnerDecision::new(id("c2"), true),
                ],
                NOW,
            )
            .expect_err("an omitted decision must not become implicit no"),
        RendezvousError::CandidateVectorMismatch
    );
    assert_eq!(
        alice
            .record_owner_decisions(
                &options,
                &[
                    OwnerDecision::new(id("c3"), true),
                    OwnerDecision::new(id("c2"), true),
                    OwnerDecision::new(id("c1"), true),
                ],
                NOW,
            )
            .expect_err("a reordered decision vector must fail"),
        RendezvousError::CandidateVectorMismatch
    );

    let first = alice.record_owner_decisions(
        &options,
        &owner_decisions(options.candidates(), 0b111),
        NOW,
    )?;
    let conflicting = alice.record_owner_decisions(
        &options,
        &owner_decisions(options.candidates(), 0b110),
        NOW,
    )?;
    negotiation.submit_decisions(first, NOW)?;
    assert_eq!(
        negotiation
            .submit_decisions(conflicting, NOW)
            .expect_err("a changed decision replay must fail"),
        RendezvousError::ConflictingReplay("alice".to_owned())
    );

    let other_round = fixture_round("different-decision-scope");
    let other_alice = agent("alice", &other_round, 0b111);
    let other_bob = agent("bob", &other_round, 0b111);
    let mut other_view = TwoAgentNegotiation::new(other_round.clone(), NOW)?;
    other_view.submit_eligibility(other_alice.evaluate(&other_round, NOW)?, NOW)?;
    other_view.submit_eligibility(other_bob.evaluate(&other_round, NOW)?, NOW)?;
    let other_options = other_view.mutual_options().expect("other mutual").clone();
    let other_response = other_bob.record_owner_decisions(
        &other_options,
        &owner_decisions(other_options.candidates(), 0b111),
        NOW,
    )?;
    assert_eq!(
        negotiation
            .submit_decisions(other_response, NOW)
            .expect_err("a decision for another exact scope must fail"),
        RendezvousError::EnvelopeScopeMismatch
    );
    Ok(())
}

#[test]
fn duplicate_intervals_and_started_candidates_fail_closed() -> Result<(), Box<dyn Error>> {
    let round = fixture_round("candidate-validation");
    let request = round.request().clone();
    let interval = UtcInterval::new(OPENED_AT + 3_600_000, MEETING_SECONDS)?;
    assert_eq!(
        CandidateRound::new(
            request.clone(),
            vec![
                Candidate::new(id("a"), interval),
                Candidate::new(id("b"), interval),
            ],
        )
        .expect_err("different IDs must not alias one exact interval"),
        RendezvousError::DuplicateCandidateInterval("b".to_owned())
    );
    assert_eq!(
        CandidateRound::new(
            request.clone(),
            vec![Candidate::new(
                id("started"),
                UtcInterval::new(OPENED_AT, MEETING_SECONDS)?,
            )],
        )
        .expect_err("a candidate cannot start at request opening"),
        RendezvousError::CandidateAlreadyStarted("started".to_owned())
    );

    let just_started = CandidateRound::new(
        request,
        vec![Candidate::new(
            id("stage-started"),
            UtcInterval::new(NOW, MEETING_SECONDS)?,
        )],
    )?;
    let alice = agent("alice", &just_started, 0b001);
    assert_eq!(
        alice
            .evaluate(&just_started, NOW)
            .expect_err("a stage cannot evaluate a candidate that has started"),
        RendezvousError::CandidateAlreadyStarted("stage-started".to_owned())
    );
    Ok(())
}

#[test]
fn expiry_and_candidate_disclosure_bounds_are_enforced() -> Result<(), Box<dyn Error>> {
    let round = fixture_round("expiry");
    let alice = agent("alice", &round, 0b111);
    assert_eq!(
        alice
            .evaluate(&round, EXPIRES_AT)
            .expect_err("expiry boundary is exclusive"),
        RendezvousError::Expired
    );

    let request = round.request().clone();
    let too_many = (0..4)
        .map(|index| {
            Ok(Candidate::new(
                id(&format!("extra-{index}")),
                UtcInterval::new(OPENED_AT + i64::from(index) * 3_600_000, MEETING_SECONDS)?,
            ))
        })
        .collect::<Result<Vec<_>, fabric_core::CalendarError>>()?;
    assert_eq!(
        CandidateRound::new(request, too_many).expect_err("four candidates must fail"),
        RendezvousError::InvalidCandidateCount(4)
    );
    Ok(())
}

#[test]
fn eligibility_response_does_not_contain_private_calendar_input() -> Result<(), Box<dyn Error>> {
    let round = fixture_round("privacy-shape");
    let private_start = OPENED_AT + 42_000_000;
    let alice = SchedulingAgent::new(
        MemberId::new("alice")?,
        LocalCalendar::from_busy(vec![UtcInterval::new(private_start, 900)?]),
    );

    let response = alice.evaluate(&round, NOW)?;
    let debug_projection = format!("{response:?}");
    assert!(!debug_projection.contains(&private_start.to_string()));
    assert!(!debug_projection.contains("reason"));
    assert_eq!(response.values().len(), round.candidates().len());
    Ok(())
}

#[test]
fn agreement_key_changes_with_exact_request_expiry() -> Result<(), Box<dyn Error>> {
    fn agreement_for(expiry: i64) -> Result<String, Box<dyn Error>> {
        let request = MeetingRequest::new(
            NegotiationId::new("key-scope")?,
            MemberId::new("alice")?,
            MemberId::new("bob")?,
            UtcInterval::new(OPENED_AT, HORIZON_SECONDS)?,
            MEETING_SECONDS,
            ProjectionProfile::FabricOnly,
            OPENED_AT,
            expiry,
        )?;
        let candidate = Candidate::new(
            id("c1"),
            UtcInterval::new(OPENED_AT + 3_600_000, MEETING_SECONDS)?,
        );
        let round = CandidateRound::new(request, vec![candidate])?;
        let alice = agent("alice", &round, 0b001);
        let bob = agent("bob", &round, 0b001);
        let mut negotiation = TwoAgentNegotiation::new(round.clone(), NOW)?;
        negotiation.submit_eligibility(alice.evaluate(&round, NOW)?, NOW)?;
        negotiation.submit_eligibility(bob.evaluate(&round, NOW)?, NOW)?;
        let options = negotiation.mutual_options().expect("mutual option").clone();
        negotiation.submit_decisions(
            alice.record_owner_decisions(&options, &[OwnerDecision::new(id("c1"), true)], NOW)?,
            NOW,
        )?;
        negotiation.submit_decisions(
            bob.record_owner_decisions(&options, &[OwnerDecision::new(id("c1"), true)], NOW)?,
            NOW,
        )?;
        Ok(negotiation
            .agreement()
            .expect("agreement")
            .agreement_key()
            .to_owned())
    }

    assert_ne!(
        agreement_for(EXPIRES_AT)?,
        agreement_for(EXPIRES_AT + 1_000)?
    );
    Ok(())
}

#[test]
fn agreement_key_binds_nonselected_candidates_and_proposal_order() -> Result<(), Box<dyn Error>> {
    fn agreement_for(candidates: Vec<Candidate>) -> Result<(String, String), Box<dyn Error>> {
        let request = MeetingRequest::new(
            NegotiationId::new("key-candidate-scope")?,
            MemberId::new("alice")?,
            MemberId::new("bob")?,
            UtcInterval::new(OPENED_AT, HORIZON_SECONDS)?,
            MEETING_SECONDS,
            ProjectionProfile::FabricOnly,
            OPENED_AT,
            EXPIRES_AT,
        )?;
        let round = CandidateRound::new(request, candidates)?;
        let alice = agent("alice", &round, 0b111);
        let bob = agent("bob", &round, 0b111);
        let mut negotiation = TwoAgentNegotiation::new(round.clone(), NOW)?;
        negotiation.submit_eligibility(alice.evaluate(&round, NOW)?, NOW)?;
        negotiation.submit_eligibility(bob.evaluate(&round, NOW)?, NOW)?;
        let options = negotiation
            .mutual_options()
            .expect("mutual options")
            .clone();
        let decisions = options
            .candidates()
            .iter()
            .map(|candidate| {
                OwnerDecision::new(candidate.id().clone(), candidate.id() == &id("c1"))
            })
            .collect::<Vec<_>>();
        negotiation.submit_decisions(
            alice.record_owner_decisions(&options, &decisions, NOW)?,
            NOW,
        )?;
        negotiation
            .submit_decisions(bob.record_owner_decisions(&options, &decisions, NOW)?, NOW)?;
        let agreement = negotiation.agreement().expect("agreement");
        Ok((
            agreement.selected_candidate().id().to_string(),
            agreement.agreement_key().to_owned(),
        ))
    }

    let c1 = Candidate::new(
        id("c1"),
        UtcInterval::new(OPENED_AT + 3_600_000, MEETING_SECONDS)?,
    );
    let c2 = Candidate::new(
        id("c2"),
        UtcInterval::new(OPENED_AT + 7_200_000, MEETING_SECONDS)?,
    );
    let changed_c2 = Candidate::new(
        id("c2"),
        UtcInterval::new(OPENED_AT + 10_800_000, MEETING_SECONDS)?,
    );
    let baseline = agreement_for(vec![c1.clone(), c2.clone()])?;
    let changed_nonselected = agreement_for(vec![c1.clone(), changed_c2])?;
    let reordered = agreement_for(vec![c2, c1])?;
    assert_eq!(baseline.0, "c1");
    assert_eq!(changed_nonselected.0, "c1");
    assert_eq!(reordered.0, "c1");
    assert_ne!(baseline.1, changed_nonselected.1);
    assert_ne!(baseline.1, reordered.1);
    Ok(())
}

#[test]
fn identifiers_are_bounded_and_safe_for_fixture_output() {
    assert_eq!(
        MemberId::new("line\nbreak").expect_err("newlines are not valid IDs"),
        RendezvousError::InvalidIdentifier("member")
    );
    assert_eq!(
        NegotiationId::new("x".repeat(65)).expect_err("overlong IDs must fail"),
        RendezvousError::InvalidIdentifier("negotiation")
    );
}
