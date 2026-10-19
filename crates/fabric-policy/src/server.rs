//! The Isabelle server process and its line protocol (Isabelle system manual, "The Isabelle
//! server"): a password line, then commands `name argument`, answered by `OK`/`ERROR`, or by
//! `FINISHED`/`FAILED` for asynchronous tasks; long messages are length-prefixed.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::Value;

use crate::error::PolicyError;

const START_TIMEOUT: Duration = Duration::from_secs(120);

pub struct ServerOptions<'a> {
    pub isabelle_home: &'a Path,
    pub user_home: &'a Path,
    pub name: &'a str,
    pub log_dir: &'a Path,
}

/// A running `isabelle server` process.
pub struct Server {
    child: Child,
    pub port: u16,
    pub password: String,
}

impl Server {
    /// Starts (or finds) the named server and reads its address and password.
    pub fn start(options: &ServerOptions<'_>) -> Result<Self, PolicyError> {
        let mut command = isabelle_command(
            options.isabelle_home,
            options.user_home,
            &["server", "-n", options.name],
        );
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PolicyError::Protocol("no server stdout".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| PolicyError::Protocol("no server stderr".to_owned()))?;
        copy_in_background(stderr, options.log_dir.join("isabelle-server-stderr.log"));

        let (sender, receiver) = mpsc::channel();
        let stdout_log = options.log_dir.join("isabelle-server-stdout.log");
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => {
                        let _ = sender.send(None);
                        return;
                    }
                    Ok(_) => {
                        if let Some(found) = parse_server_line(&line) {
                            let _ = sender.send(Some(found));
                            break;
                        }
                    }
                }
            }
            if let Ok(mut file) = File::create(stdout_log) {
                let _ = io::copy(&mut reader, &mut file);
            }
        });

        match receiver.recv_timeout(START_TIMEOUT) {
            Ok(Some((port, password))) => Ok(Self {
                child,
                port,
                password,
            }),
            Ok(None) => {
                let _ = child.kill();
                Err(PolicyError::Isabelle(
                    "isabelle server exited before printing its address".to_owned(),
                ))
            }
            Err(_) => {
                let _ = child.kill();
                Err(PolicyError::Isabelle(format!(
                    "isabelle server printed no address within {}s",
                    START_TIMEOUT.as_secs()
                )))
            }
        }
    }

    /// Waits for the process to exit after `shutdown`, killing it after `timeout`.
    pub fn wait_or_kill(&mut self, timeout: Duration) {
        let step = Duration::from_millis(100);
        let mut waited = Duration::ZERO;
        while waited < timeout {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(step);
            waited += step;
        }
        let _ = self.child.kill();
    }
}

/// Parses `server "NAME" = 127.0.0.1:PORT (password "UUID")`.
fn parse_server_line(line: &str) -> Option<(u16, String)> {
    let rest = line.trim().strip_prefix("server \"")?;
    let (_, rest) = rest.split_once("\" = ")?;
    let (address, rest) = rest.split_once(" (password \"")?;
    let password = rest.strip_suffix("\")")?;
    let port = address.rsplit_once(':')?.1.parse().ok()?;
    Some((port, password.to_owned()))
}

fn copy_in_background(mut source: impl Read + Send + 'static, path: PathBuf) {
    thread::spawn(move || {
        if let Ok(mut file) = File::create(path) {
            let _ = io::copy(&mut source, &mut file);
        }
    });
}

/// A command that runs an Isabelle tool in the repository-local installation, with Isabelle's
/// per-user directory redirected into `user_home`.
#[cfg(windows)]
fn isabelle_command(isabelle_home: &Path, user_home: &Path, arguments: &[&str]) -> Command {
    let bash = isabelle_home
        .join("contrib")
        .join("cygwin")
        .join("bin")
        .join("bash.exe");
    let path = format!(
        "{};{}",
        isabelle_home.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut command = Command::new(bash);
    command
        .arg("--login")
        .arg("-c")
        .arg(format!("isabelle {}", arguments.join(" ")))
        .env("USER_HOME", platform_path(user_home))
        .env("HOME", user_home)
        .env("PATH", path)
        .env("LANG", "en_US.UTF-8")
        .env("CHERE_INVOKING", "true");
    command
}

#[cfg(not(windows))]
fn isabelle_command(isabelle_home: &Path, user_home: &Path, arguments: &[&str]) -> Command {
    let mut command = Command::new(isabelle_home.join("bin").join("isabelle"));
    command.args(arguments).env("USER_HOME", user_home);
    command
}

/// The path as Isabelle expects it: `/cygdrive/c/...` on Windows, unchanged elsewhere.
#[must_use]
pub fn platform_path(path: &Path) -> String {
    let text = path.display().to_string();
    if cfg!(windows) {
        let text = text.replace('\\', "/");
        if let Some((drive, rest)) = text.split_once(":/")
            && drive.len() == 1
        {
            return format!("/cygdrive/{}/{rest}", drive.to_ascii_lowercase());
        }
        text
    } else {
        text
    }
}

/// One client connection to the server.
pub struct Connection {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Connection {
    /// Connects, sends the password, and returns the server's greeting.
    pub fn open(
        port: u16,
        password: &str,
        timeout: Duration,
    ) -> Result<(Self, Value), PolicyError> {
        let stream = TcpStream::connect(("127.0.0.1", port))?;
        stream.set_read_timeout(Some(timeout))?;
        let mut connection = Self {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
        };
        writeln!(connection.writer, "{password}")?;
        let (kind, greeting) = connection.read()?;
        if kind != "OK" {
            return Err(PolicyError::Protocol(format!(
                "the server rejected the password ({kind})"
            )));
        }
        Ok((connection, greeting))
    }

    pub fn set_timeout(&self, timeout: Duration) -> Result<(), PolicyError> {
        self.writer.set_read_timeout(Some(timeout))?;
        Ok(())
    }

    /// Sends a command and returns its `OK` result.
    pub fn call(&mut self, name: &str, argument: &Value) -> Result<Value, PolicyError> {
        self.send(name, argument)?;
        let (kind, value) = self.read()?;
        match kind.as_str() {
            "OK" => Ok(value),
            "ERROR" => Err(PolicyError::Isabelle(format!("{name}: {value}"))),
            _ => Err(PolicyError::Protocol(format!(
                "{name}: unexpected {kind} {value}"
            ))),
        }
    }

    /// Runs an asynchronous command and returns its `FINISHED` result.
    pub fn task(&mut self, name: &str, argument: &Value) -> Result<Value, PolicyError> {
        let accepted = self.call(name, argument)?;
        let task = accepted["task"]
            .as_str()
            .ok_or_else(|| PolicyError::Protocol(format!("{name}: no task id in {accepted}")))?
            .to_owned();
        loop {
            let (kind, value) = self.read()?;
            let ours = value["task"].as_str() == Some(task.as_str());
            match kind.as_str() {
                "FINISHED" if ours => return Ok(value),
                "FAILED" if ours => return Err(PolicyError::Isabelle(format!("{name}: {value}"))),
                _ => {}
            }
        }
    }

    fn send(&mut self, name: &str, argument: &Value) -> Result<(), PolicyError> {
        if argument.is_null() {
            writeln!(self.writer, "{name}")?;
        } else {
            writeln!(self.writer, "{name} {}", serde_json::to_string(argument)?)?;
        }
        Ok(())
    }

    /// Reads one message: a single line, or a decimal length line followed by that many bytes.
    fn read(&mut self) -> Result<(String, Value), PolicyError> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line)? == 0 {
                return Err(PolicyError::Protocol(
                    "the server closed the connection".to_owned(),
                ));
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                continue;
            }
            let message = if trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
                let length: usize = trimmed
                    .parse()
                    .map_err(|_| PolicyError::Protocol(format!("bad length line {trimmed:?}")))?;
                let mut buffer = vec![0; length];
                self.reader.read_exact(&mut buffer)?;
                String::from_utf8(buffer)
                    .map_err(|_| PolicyError::Protocol("message is not UTF-8".to_owned()))?
            } else {
                trimmed.to_owned()
            };
            let (name, argument) = message.split_once(' ').unwrap_or((message.as_str(), ""));
            let argument = argument.trim();
            let value = if argument.is_empty() {
                Value::Null
            } else {
                serde_json::from_str(argument)
                    .unwrap_or_else(|_| Value::String(argument.to_owned()))
            };
            return Ok((name.trim().to_owned(), value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_server_line;

    #[test]
    fn reads_the_server_address_line() {
        assert_eq!(
            parse_server_line("server \"fabric\" = 127.0.0.1:4711 (password \"abc-123\")\n"),
            Some((4711, "abc-123".to_owned()))
        );
        assert_eq!(parse_server_line("Copying skeleton files."), None);
    }
}
