//! The `fabric-mcp-bridge` executable: the arguments, the threads and the loop (W06 2.3).
//!
//! A wrong command line is exit 64 before stdin is read. Serving, a thread reads stdin as
//! bounded lines, a thread per connection reads the broker's frames, and one loop owns the
//! `Bridge` and does all writing. End of input is exit 0; a panic prints one fixed line and is
//! exit 70 (W06 2.4). stdout carries only JSON-RPC lines; stderr only fixed words, ids, tool
//! names, state names and error codes (A25).

use std::collections::VecDeque;
use std::io::{self, Write};
use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use fabric_desktop_contracts::scalar::Id;
use fabric_mcp_bridge::bridge::{Bridge, Effect, Event};
use fabric_mcp_bridge::broker::frames::{Line, LineReader};
use fabric_mcp_bridge::cli::{self, Command, Seams};
use fabric_mcp_bridge::config_text::{self, DebugPaths, PathRule};
use fabric_mcp_bridge::{NAME, VERSION};

/// The longest line accepted from the client (W06 2.5).
const MAX_CLIENT_LINE: usize = 2 * 1024 * 1024;
/// The exit code after a panic (`EX_SOFTWARE`).
const EXIT_PANIC: i32 = 70;
/// The exit code of `code` and `config` when they cannot answer.
const EXIT_FAILED: u8 = 1;

/// What the threads send the loop.
enum Input {
    Client(Line),
    ClientFailed,
    #[cfg(unix)]
    Broker {
        generation: u64,
        line: Option<Line>,
    },
}

/// The bound of the loop's input channel: a reader thread waits rather than queue without
/// limit.
const QUEUED_INPUTS: usize = 16;

/// Writes `text` to stdout; a closed stdout is exit 1, never a panic.
fn print_out(text: &str) -> ExitCode {
    let mut out = io::stdout().lock();
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(EXIT_FAILED),
    }
}

fn log(text: &str) {
    let _ = writeln!(io::stderr(), "{NAME}: {text}");
}

fn main() -> ExitCode {
    std::panic::set_hook(Box::new(|_| {
        let _ = writeln!(io::stderr(), "{NAME}: internal fault");
        std::process::exit(EXIT_PANIC);
    }));
    let command = match cli::parse(std::env::args_os().skip(1), cfg!(debug_assertions)) {
        Ok(command) => command,
        Err(error) => {
            log(error.as_str());
            let _ = writeln!(io::stderr(), "{}", cli::USAGE);
            return ExitCode::from(error.exit_code());
        }
    };
    match command {
        Command::Version => print_out(&format!("{NAME} {VERSION}\n")),
        Command::Code {
            enrollment,
            key_dir,
        } => print_code(&enrollment, key_dir),
        Command::Config {
            client,
            enrollment,
            seams,
        } => print_config(client, &enrollment, seams.as_ref()),
        Command::Serve { enrollment, seams } => serve(enrollment, seams),
    }
}

#[cfg(unix)]
fn print_code(enrollment: &Id, key_dir: Option<std::path::PathBuf>) -> ExitCode {
    use fabric_mcp_bridge::broker::key::KeyError;
    use fabric_mcp_bridge::broker::key::file::FileKeyStore;
    let result = match key_dir {
        Some(dir) => FileKeyStore::new(dir).load_or_create(enrollment),
        None => Err(KeyError::NoStore),
    };
    match result {
        Ok(key) => print_out(&format!("{}\n", key.pairing_code())),
        Err(e) => {
            log(e.as_str());
            ExitCode::from(EXIT_FAILED)
        }
    }
}

#[cfg(not(unix))]
fn print_code(_enrollment: &Id, _key_dir: Option<std::path::PathBuf>) -> ExitCode {
    log("no key store on this system");
    ExitCode::from(EXIT_FAILED)
}

fn print_config(client: config_text::Client, enrollment: &Id, seams: Option<&Seams>) -> ExitCode {
    let Some(bridge_path) = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
    else {
        log("the bridge's own path cannot be read as UTF-8");
        return ExitCode::from(EXIT_FAILED);
    };
    let extras = match seams {
        None => None,
        Some(s) => match (s.socket.to_str(), s.key_dir.to_str()) {
            (Some(socket), Some(key_dir)) => Some(DebugPaths {
                socket: socket.to_owned(),
                key_dir: key_dir.to_owned(),
            }),
            _ => {
                log("--socket or --key-dir is not UTF-8");
                return ExitCode::from(EXIT_FAILED);
            }
        },
    };
    let rule = PathRule {
        debug_build: cfg!(debug_assertions),
        home: std::env::var("HOME").ok(),
    };
    match config_text::config_text(client, &bridge_path, enrollment, extras.as_ref(), &rule) {
        Ok(text) => print_out(&text),
        Err(e) => {
            log(e.as_str());
            ExitCode::from(EXIT_FAILED)
        }
    }
}

fn spawn_stdin(events: SyncSender<Input>) {
    thread::spawn(move || {
        let mut reader = LineReader::new(io::stdin().lock(), MAX_CLIENT_LINE);
        loop {
            let input = match reader.next_line() {
                Ok(line) => Input::Client(line),
                Err(_) => Input::ClientFailed,
            };
            let last = matches!(input, Input::Client(Line::End) | Input::ClientFailed);
            if events.send(input).is_err() || last {
                return;
            }
        }
    });
}

fn millis(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The connection to the broker, as the loop holds it.
#[cfg(unix)]
mod net {
    use std::io::BufReader;
    use std::net::Shutdown;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc::SyncSender;
    use std::thread;
    use std::time::Instant;

    use fabric_desktop_contracts::scalar::Id;
    use fabric_mcp_bridge::bridge::ConnectOutcome;
    use fabric_mcp_bridge::broker::frames::{Line, LineReader};
    use fabric_mcp_bridge::broker::key::file::FileKeyStore;
    use fabric_mcp_bridge::broker::socket::{
        Connector, TimedReader, WRITE_LIMIT, installed_socket_path, write_before,
    };
    use fabric_mcp_bridge::cli::Seams;

    use super::Input;

    pub struct Net {
        connector: Connector,
        writer: Option<UnixStream>,
        generation: u64,
        current: Option<u64>,
    }

    impl Net {
        pub fn new(enrollment: Id, seams: Option<Seams>) -> Net {
            let (socket, keys) = match seams {
                Some(s) => (Ok(s.socket), Some(FileKeyStore::new(s.key_dir))),
                // The Keychain store comes in phase 2 (W06 5.4): no key store yet.
                None => (
                    installed_socket_path(std::env::var_os("HOME").as_deref()),
                    None,
                ),
            };
            Net {
                connector: Connector {
                    enrollment,
                    socket,
                    keys,
                },
                writer: None,
                generation: 0,
                current: None,
            }
        }

        pub fn connect(&mut self, events: &SyncSender<Input>) -> ConnectOutcome {
            self.close();
            let (outcome, connection) = self.connector.connect();
            if let Some(connection) = connection {
                self.generation += 1;
                let generation = self.generation;
                self.current = Some(generation);
                self.writer = Some(connection.writer);
                spawn_reader(connection.reader, generation, events.clone());
            }
            outcome
        }

        /// Writes a whole frame line within [`WRITE_LIMIT`]; false when it did not finish or
        /// the connection is gone, and the connection is then closed (W06 2.5).
        pub fn send(&mut self, line: &str) -> bool {
            let deadline = Instant::now() + WRITE_LIMIT;
            let sent = self
                .writer
                .as_mut()
                .is_some_and(|w| write_before(w, line.as_bytes(), deadline).is_ok());
            if !sent {
                self.close();
            }
            sent
        }

        pub fn close(&mut self) {
            if let Some(writer) = self.writer.take() {
                let _ = writer.shutdown(Shutdown::Both);
            }
            self.current = None;
        }

        pub fn is_current(&self, generation: u64) -> bool {
            self.current == Some(generation)
        }
    }

    fn spawn_reader(
        mut reader: LineReader<BufReader<TimedReader>>,
        generation: u64,
        events: SyncSender<Input>,
    ) {
        thread::spawn(move || {
            loop {
                let line = reader.next_line().ok();
                let last = !matches!(line, Some(Line::Line(_)));
                let line = match line {
                    Some(Line::End) => None,
                    other => other,
                };
                if events.send(Input::Broker { generation, line }).is_err() || last {
                    return;
                }
            }
        });
    }
}

#[cfg(not(unix))]
mod net {
    use std::sync::mpsc::SyncSender;

    use fabric_desktop_contracts::scalar::Id;
    use fabric_mcp_bridge::bridge::{ConnectOutcome, not_running};
    use fabric_mcp_bridge::cli::Seams;

    use super::Input;

    pub struct Net;

    impl Net {
        pub fn new(_enrollment: Id, _seams: Option<Seams>) -> Net {
            Net
        }

        pub fn connect(&mut self, _events: &SyncSender<Input>) -> ConnectOutcome {
            not_running("not supported on this system")
        }

        pub fn send(&mut self, _line: &str) -> bool {
            false
        }

        pub fn close(&mut self) {}
    }
}

fn serve(enrollment: Id, seams: Option<Seams>) -> ExitCode {
    let (sender, inputs) = mpsc::sync_channel(QUEUED_INPUTS);
    spawn_stdin(sender.clone());
    let mut net = net::Net::new(enrollment, seams);
    let start = Instant::now();
    let mut bridge = Bridge::new(Box::new(fabric_mcp_bridge::random_128));
    let mut stdout = io::stdout().lock();
    loop {
        let now = millis(start);
        let received = match bridge.next_deadline() {
            Some(due) if due <= now => Err(RecvTimeoutError::Timeout),
            Some(due) => inputs.recv_timeout(Duration::from_millis(due - now)),
            None => inputs.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        let event = match received {
            Err(RecvTimeoutError::Timeout) => Event::Tick,
            Err(RecvTimeoutError::Disconnected) => return ExitCode::SUCCESS,
            Ok(Input::Client(Line::Line(bytes))) => Event::McpLine(bytes),
            Ok(Input::Client(Line::TooLong)) => Event::McpLineTooLong,
            // End of input, or stdin can no longer be read: the client is gone (W06 2.3).
            Ok(Input::Client(Line::End) | Input::ClientFailed) => return ExitCode::SUCCESS,
            #[cfg(unix)]
            Ok(Input::Broker { generation, line }) => {
                if !net.is_current(generation) {
                    continue;
                }
                match line {
                    Some(Line::Line(bytes)) => Event::BrokerFrame(bytes),
                    Some(Line::TooLong) => Event::BrokerFrameTooLong,
                    Some(Line::End) | None => {
                        net.close();
                        Event::BrokerClosed
                    }
                }
            }
        };
        let mut queue = VecDeque::from([event]);
        while let Some(event) = queue.pop_front() {
            for effect in bridge.handle(event, millis(start)) {
                match effect {
                    Effect::Stdout(line) => {
                        let written = writeln!(stdout, "{line}").and_then(|()| stdout.flush());
                        if written.is_err() {
                            // stdout is closed: the client is gone.
                            return ExitCode::SUCCESS;
                        }
                    }
                    Effect::SendFrame(line) => {
                        if !net.send(&line) {
                            queue.push_back(Event::BrokerWriteFailed);
                        }
                    }
                    Effect::Connect => queue.push_back(Event::Connected(net.connect(&sender))),
                    Effect::Close => net.close(),
                    Effect::Log(line) => {
                        let _ = writeln!(io::stderr(), "{line}");
                    }
                }
            }
        }
    }
}
