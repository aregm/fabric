//! The Fabric Calendar broker executable. In work package CD-W02 it is a spike: see the
//! library documentation of `fabric_broker`.
//!
//! The process shell: it refuses arguments, reads stdin as bounded lines on a thread, runs the
//! script's timers, and writes frames to stdout. stdout carries frames only; diagnostics go to
//! stderr. End of input on stdin is the order to stop.

use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fabric_broker::{Line, LineReader, Spike};
use fabric_desktop_contracts::{Frame, MAX_FRAME_BYTES};

/// The exit code for a usage error (`EX_USAGE`).
const EXIT_USAGE: u8 = 64;
/// The exit code when stdin or stdout fails (`EX_IOERR`).
const EXIT_IO: u8 = 74;

/// How many read lines may wait for the main loop. The reader blocks beyond this, so a fast
/// host cannot make the broker hold more than a few frames in memory.
const QUEUED_LINES: usize = 4;

/// Unix time in milliseconds from one reading of the system clock at start and a monotonic
/// clock since, so it never goes back.
struct Clock {
    start_unix_ms: u64,
    start: Instant,
}

impl Clock {
    fn start() -> Clock {
        let start_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        Clock {
            start_unix_ms,
            start: Instant::now(),
        }
    }

    fn now_ms(&self) -> u64 {
        let elapsed = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.start_unix_ms.saturating_add(elapsed)
    }
}

/// What the reader thread passes to the main loop.
enum Input {
    Line(Vec<u8>),
    Oversized,
    /// The input ended; the main loop stops.
    End,
    /// Reading failed; the main loop stops with an error.
    Failed(io::Error),
}

fn spawn_reader() -> Receiver<Input> {
    let (sender, receiver) = mpsc::sync_channel(QUEUED_LINES);
    thread::spawn(move || {
        let mut reader = LineReader::new(io::stdin().lock(), MAX_FRAME_BYTES);
        loop {
            let input = match reader.next_line() {
                Ok(Line::Frame(bytes)) => Input::Line(bytes),
                Ok(Line::Oversized) => Input::Oversized,
                Ok(Line::Unterminated(length)) => {
                    eprintln!(
                        "fabric-broker: stdin ended inside a line of {length} bytes with no line feed; it is not a frame and is not answered"
                    );
                    Input::End
                }
                Ok(Line::End) => Input::End,
                Err(e) => Input::Failed(e),
            };
            let last = matches!(input, Input::End | Input::Failed(_));
            if sender.send(input).is_err() || last {
                return;
            }
        }
    });
    receiver
}

fn write_frames(out: &mut impl Write, frames: Vec<Frame>) -> io::Result<()> {
    for frame in frames {
        out.write_all(frame.to_line().as_bytes())?;
        out.flush()?;
    }
    Ok(())
}

fn main() -> ExitCode {
    if std::env::args_os().len() > 1 {
        eprintln!(
            "usage: fabric-broker (no arguments; it speaks the owner channel of fabric-desktop/1 on stdin and stdout)"
        );
        return ExitCode::from(EXIT_USAGE);
    }
    let clock = Clock::start();
    let inputs = spawn_reader();
    let mut spike = Spike::new();
    let mut out = io::stdout().lock();
    loop {
        let input = match spike.next_due_ms() {
            Some(due) => {
                let now = clock.now_ms();
                if due <= now {
                    if let Err(e) = write_frames(&mut out, spike.tick(now)) {
                        return stdout_failed(&e);
                    }
                    continue;
                }
                match inputs.recv_timeout(Duration::from_millis(due - now)) {
                    Ok(input) => input,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => Input::End,
                }
            }
            None => inputs.recv().unwrap_or(Input::End),
        };
        let frames = match input {
            Input::Line(bytes) => spike.handle_line(&bytes, clock.now_ms()),
            Input::Oversized => spike.handle_oversized(),
            Input::End => return ExitCode::SUCCESS,
            Input::Failed(e) => {
                eprintln!("fabric-broker: reading stdin failed: {e}");
                return ExitCode::from(EXIT_IO);
            }
        };
        if let Err(e) = write_frames(&mut out, frames) {
            return stdout_failed(&e);
        }
    }
}

fn stdout_failed(e: &io::Error) -> ExitCode {
    eprintln!("fabric-broker: writing stdout failed, so the host is gone: {e}");
    ExitCode::from(EXIT_IO)
}
