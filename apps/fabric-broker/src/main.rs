//! The Fabric Calendar broker executable (work package CD-W04): see the library documentation
//! of `fabric_broker`.
//!
//! The process shell (design 10.6): it refuses arguments (exit 64) and a refused configuration
//! (exit 78) before it reads stdin or starts a thread; it opens the store through the authority
//! core, reads stdin as bounded lines on a thread, runs the prover and the provider on worker
//! threads through `fabric_broker::shell`, runs due work, and writes frames to stdout one at a
//! time. stdout carries frames only; diagnostics go to stderr. End of input on stdin is the order
//! to stop (exit 0). A failed read of stdin or write of stdout is exit 74; a failed write to the
//! store ends the run with exit 75. Every exit after the shell exists first stops the Isabelle
//! worker within its bound.

use std::io;
use std::process::ExitCode;
use std::sync::mpsc::{self, SyncSender};
use std::thread;

use fabric_broker::config::{self, EXIT_CONFIG, EXIT_HALT, EXIT_IO};
use fabric_broker::shell::{self, Event, Plan, QUEUED_EVENTS, Shell, now_ms};
use fabric_broker::{Line, LineReader};
use fabric_broker_core::{Broker, Input, Settings, StartError};
use fabric_desktop_contracts::MAX_FRAME_BYTES;

/// The exit code for a usage error (`EX_USAGE`).
const EXIT_USAGE: u8 = 64;

fn spawn_reader(events: SyncSender<Event>) {
    thread::spawn(move || {
        let mut reader = LineReader::new(io::stdin().lock(), MAX_FRAME_BYTES);
        loop {
            let event = match reader.next_line() {
                Ok(Line::Frame(bytes)) => Event::Line(bytes),
                Ok(Line::Oversized) => Event::Oversized,
                Ok(Line::Unterminated(length)) => {
                    eprintln!(
                        "fabric-broker: stdin ended inside a line of {length} bytes with no line feed; it is not a frame and is not answered"
                    );
                    Event::End
                }
                Ok(Line::End) => Event::End,
                Err(e) => Event::Failed(e),
            };
            let last = matches!(event, Event::End | Event::Failed(_));
            if events.send(event).is_err() || last {
                return;
            }
        }
    });
}

fn main() -> ExitCode {
    if std::env::args_os().len() > 1 {
        eprintln!(
            "usage: fabric-broker (no arguments; it speaks the owner channel of fabric-desktop/1 on stdin and stdout, and reads {}, {}, {}, {}, {}, {}, {} and {})",
            config::DATA_DIR,
            config::KEY_SOURCE,
            config::SYNTHETIC_AGENT,
            config::ISABELLE_HOME,
            config::ISABELLE_USER_HOME,
            config::RULES_DIR,
            config::APP_RESOURCES_DIR,
            config::APP_SUPPORT_DIR
        );
        return ExitCode::from(EXIT_USAGE);
    }
    let config = match config::config_from(std::env::vars_os()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };
    // An installed Isabelle that is missing or another version: a run without a rule prover,
    // said once here.
    if let Some(line) = config.installation.line() {
        eprintln!("{line}");
    }
    let plan = Plan::of(&config, now_ms());
    let settings = Settings::cd_w04(shell::build_label(&config));
    let (mut broker, started) = match Broker::start(
        shell::opener(&config),
        settings,
        plan.attachments(),
        now_ms(),
    ) {
        Ok(started) => started,
        Err(error) => {
            eprintln!("fabric-broker: {error}");
            let code = match error {
                StartError::Config(_) => EXIT_CONFIG,
                StartError::Halt(_) => EXIT_HALT,
            };
            return ExitCode::from(code);
        }
    };
    let (sender, events) = mpsc::sync_channel(QUEUED_EVENTS);
    let mut shell = Shell::new(io::stdout().lock(), plan, sender.clone());
    spawn_reader(sender);
    let code = run(&mut shell, &mut broker, started, &events);
    // After the last frames: the Isabelle worker stops and its work directory goes.
    shell.stop(&events);
    code
}

/// The main loop: frames and results in, frames and jobs out, until the input ends or a write
/// fails.
fn run<W: io::Write>(
    shell: &mut Shell<W>,
    broker: &mut Broker,
    started: fabric_broker_core::Output,
    events: &mpsc::Receiver<Event>,
) -> ExitCode {
    if let Err(code) = shell.emit(broker, started) {
        return code;
    }
    loop {
        let input = match shell::next_event(events, shell.next_due_ms(broker)) {
            None => Input::Tick,
            Some(Event::End) => {
                return match shell.quit(broker, now_ms()) {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(code) => code,
                };
            }
            Some(Event::Failed(e)) => {
                eprintln!("fabric-broker: reading stdin failed: {e}");
                return ExitCode::from(EXIT_IO);
            }
            Some(event) => match shell::input_of(event) {
                Some(input) => input,
                None => continue,
            },
        };
        if let Err(code) = shell.turn(broker, input, now_ms()) {
            return code;
        }
    }
}
