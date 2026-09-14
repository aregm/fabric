use std::error::Error;
use std::process::Command;

fn run_demo() -> Result<(String, String), Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_fabric"))
        .arg("demo-meeting")
        .output()?;
    assert!(output.status.success());
    Ok((
        String::from_utf8(output.stdout)?,
        String::from_utf8(output.stderr)?,
    ))
}

#[test]
fn demo_is_deterministic_minimal_and_stops_before_scheduling() -> Result<(), Box<dyn Error>> {
    let first = run_demo()?;
    let second = run_demo()?;
    assert_eq!(first, second);
    assert!(first.1.is_empty());

    let stdout = first.0;
    for expected in [
        "protocol=fabric-schedule-sim/0",
        "transport=in-process e2ee=false provider_writes=false",
        "output=coordinator-sensitive-debug candidate_bits_disclosed=true",
        "eligibility member=bob values=c1:no,c2:yes,c3:yes",
        "decisions member=bob values=c2:no,c3:yes",
        "result=agreed",
        "selected=c3",
        "agents_hold_identical_record=true",
        "scheduled=false",
    ] {
        assert!(stdout.contains(expected), "missing output: {expected}");
    }

    // The synthetic event below exists in Alice's private calendar, but it is not
    // one of the disclosed candidates and must not appear in the transcript.
    for forbidden in [
        "1800042000000",
        "event_title",
        "calendar_id",
        "provider_id",
        "conflict_reason",
    ] {
        assert!(!stdout.contains(forbidden), "leaked field: {forbidden}");
    }
    Ok(())
}
