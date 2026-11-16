//! The Isabelle server process and its line protocol (Isabelle system manual, "The Isabelle
//! server"): a password line, then commands `name argument`, answered by `OK`/`ERROR`, or by
//! `FINISHED`/`FAILED` for asynchronous tasks; long messages are length-prefixed.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

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
    /// Whether the server leads a process group of its own ([`Server::start_in_group`]), so
    /// [`Server::kill`] signals the whole group.
    grouped: bool,
    pub port: u16,
    pub password: String,
}

/// Where a running server listens, and its password. The demo's launcher starts one server
/// and passes this privately to both policy gates (section 5.5).
#[derive(Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ServerAddress {
    pub port: u16,
    pub password: String,
}

impl std::fmt::Debug for ServerAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerAddress")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl Server {
    /// Starts (or finds) the named server and reads its address and password.
    ///
    /// Isabelle keeps its named servers in a SQLite database under the user home, and two
    /// servers registering at once make one of them exit with `SQLITE_BUSY`. Starts within
    /// this process are therefore serialized; the live tests start one server per test.
    pub fn start(options: &ServerOptions<'_>) -> Result<Self, PolicyError> {
        Self::start_with(options, false)
    }

    /// Starts the named server as [`Server::start`] does, but on Unix in a process group of
    /// its own, so that [`Server::kill`] ends the Java server and everything it started, not
    /// only the `isabelle` shell wrapper (which runs Java as a child). Elsewhere it is
    /// [`Server::start`].
    pub fn start_in_group(options: &ServerOptions<'_>) -> Result<Self, PolicyError> {
        Self::start_with(options, cfg!(unix))
    }

    fn start_with(options: &ServerOptions<'_>, grouped: bool) -> Result<Self, PolicyError> {
        static STARTING: Mutex<()> = Mutex::new(());
        let _starting = STARTING.lock().unwrap_or_else(PoisonError::into_inner);
        let mut command = isabelle_command(
            options.isabelle_home,
            options.user_home,
            &["server", "-n", options.name],
        );
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        if grouped {
            std::os::unix::process::CommandExt::process_group(&mut command, 0);
        }
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
                grouped,
                port,
                password,
            }),
            Ok(None) => {
                kill_child(&mut child, grouped);
                Err(PolicyError::Isabelle(
                    "isabelle server exited before printing its address".to_owned(),
                ))
            }
            Err(_) => {
                kill_child(&mut child, grouped);
                Err(PolicyError::Isabelle(format!(
                    "isabelle server printed no address within {}s",
                    START_TIMEOUT.as_secs()
                )))
            }
        }
    }

    #[must_use]
    pub fn address(&self) -> ServerAddress {
        ServerAddress {
            port: self.port,
            password: self.password.clone(),
        }
    }

    /// Asks the server to shut down and waits for it, killing it after 20 seconds.
    pub fn shutdown(&mut self) {
        if let Ok((mut connection, _)) =
            Connection::open(self.port, &self.password, Duration::from_secs(10))
        {
            let _ = connection.call("shutdown", &Value::Null);
        }
        self.wait_or_kill(Duration::from_secs(20));
    }

    /// Waits for the process to exit after `shutdown`, killing it after `timeout` (with
    /// [`Server::kill`]).
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
        self.kill();
    }

    /// Whether the `isabelle server` process this value holds has exited. For a server
    /// started on Unix that is the shell wrapper, which exits only after Java has.
    pub fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Kills the server: for one started with [`Server::start_in_group`] on Unix, every
    /// process of its group (the wrapper, Java, and what Java started in that group) and of
    /// the groups that processes below the wrapper lead (Isabelle runs each `poly` in a group
    /// of its own), waiting up to [`KILL_WAIT`] for them to be gone and for the port to be
    /// closed, and reaps the wrapper; otherwise it kills the process this value holds, as
    /// before.
    pub fn kill(&mut self) {
        let started = Instant::now();
        kill_child(&mut self.child, self.grouped);
        if self.grouped {
            // A killed JVM holds its listening socket for a few milliseconds after `kill -0`
            // stops finding it.
            while TcpStream::connect(("127.0.0.1", self.port)).is_ok()
                && started.elapsed() < KILL_WAIT
            {
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// How long [`Server::kill`] waits for the killed processes of a server started with
/// [`Server::start_in_group`] to be gone and its port closed.
pub const KILL_WAIT: Duration = Duration::from_secs(2);

/// Kills `child` as before; when `grouped`, first its group and the groups below it, and then
/// reaps it and waits for the groups to be gone.
fn kill_child(child: &mut Child, grouped: bool) {
    #[cfg(unix)]
    let groups = if grouped && matches!(child.try_wait(), Ok(None)) {
        // The group id is the leader's pid, which stays ours while the child is unreaped.
        let group = child.id();
        // Stopped first, the group starts nothing new while the processes below it are listed.
        let _ = signal_groups("STOP", &[group]);
        let mut groups = vec![group];
        groups.extend(groups_below(group));
        let _ = signal_groups("KILL", &groups);
        groups
    } else {
        Vec::new()
    };
    let _ = child.kill();
    if !grouped {
        return;
    }
    let _ = child.wait();
    #[cfg(unix)]
    if !groups.is_empty() {
        let started = Instant::now();
        // `kill` fails if any operand fails, so each group is asked on its own.
        let alive = |group: &u32| signal_groups("0", &[*group]).is_ok();
        while groups.iter().any(alive) && started.elapsed() < KILL_WAIT {
            thread::sleep(Duration::from_millis(10));
        }
    }
}

/// The process groups led by processes below `leader` (its descendants by parent pid), from
/// `ps -A -o pid=,ppid=,pgid=`. Only groups whose leader is such a descendant are named, so no
/// group outside the server is ever signalled.
#[cfg(unix)]
fn groups_below(leader: u32) -> Vec<u32> {
    let Ok(output) = Command::new("ps")
        .args(["-A", "-o", "pid=,ppid=,pgid="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    let rows: Vec<[u32; 3]> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace().map(str::parse::<u32>);
            Some([
                fields.next()?.ok()?,
                fields.next()?.ok()?,
                fields.next()?.ok()?,
            ])
        })
        .collect();
    let mut below = vec![leader];
    let mut next = 0;
    while next < below.len() {
        let parent = below[next];
        for [pid, ppid, _] in &rows {
            if *ppid == parent && !below.contains(pid) {
                below.push(*pid);
            }
        }
        next += 1;
    }
    let mut groups: Vec<u32> = rows
        .iter()
        .filter(|[pid, _, pgid]| pid == pgid && *pid != leader && below.contains(pid))
        .map(|[pid, _, _]| *pid)
        .collect();
    groups.sort_unstable();
    groups.dedup();
    groups
}

/// Sends `SIGKILL` to every process of the group `group` with the system `kill`, in the
/// POSIX form `kill -s KILL -- -<pgid>` (std has no safe call for signalling a group).
#[cfg(unix)]
pub fn kill_group(group: u32) -> io::Result<()> {
    signal_groups("KILL", &[group])
}

/// Runs `/bin/kill -s <signal> -- -<group>...`; it fails when no process of any of the groups
/// could be signalled, which with signal `0` means they are all gone.
#[cfg(unix)]
fn signal_groups(signal: &str, groups: &[u32]) -> io::Result<()> {
    let mut command = Command::new("/bin/kill");
    command.args(["-s", signal, "--"]);
    command.args(groups.iter().map(|group| format!("-{group}")));
    let status = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "kill -s {signal} for the groups {groups:?} exited with {status}"
        )))
    }
}

/// The value of one Isabelle setting, as `isabelle getenv -b NAME` prints it, with the same
/// environment [`Server::start`] gives the server.
pub fn getenv(isabelle_home: &Path, user_home: &Path, name: &str) -> Result<String, PolicyError> {
    let output = isabelle_command(isabelle_home, user_home, &["getenv", "-b", name])
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(PolicyError::Isabelle(format!(
            "isabelle getenv {name} exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8(output.stdout)
        .map_err(|_| PolicyError::Isabelle(format!("isabelle getenv {name} printed no UTF-8")))?;
    Ok(text.trim_end_matches(['\r', '\n']).to_owned())
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
    reader: BufReader<DeadlineStream>,
    writer: TcpStream,
}

/// The socket's read half. While a deadline is set, every read waits only for the time left
/// before it, so a deadline bounds a whole exchange however the server spreads its bytes.
struct DeadlineStream {
    stream: TcpStream,
    deadline: Option<Instant>,
}

impl Read for DeadlineStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if let Some(deadline) = self.deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the deadline for the server's answer has passed",
                ));
            }
            self.stream.set_read_timeout(Some(left))?;
        }
        self.stream.read(buffer)
    }
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
            reader: BufReader::new(DeadlineStream {
                stream: stream.try_clone()?,
                deadline: None,
            }),
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

    /// [`Connection::call`], over by `deadline`: sending and every read wait only for the
    /// time left, and a deadline already past, or passed while waiting, is an I/O error
    /// (after which the connection may hold the rest of the answer and must not be reused).
    pub fn call_until(
        &mut self,
        name: &str,
        argument: &Value,
        deadline: Instant,
    ) -> Result<Value, PolicyError> {
        self.until(deadline, |connection| connection.call(name, argument))
    }

    /// [`Connection::task`], over by `deadline` as [`Connection::call_until`] is: the bound
    /// covers the acceptance and every message up to the task's end.
    pub fn task_until(
        &mut self,
        name: &str,
        argument: &Value,
        deadline: Instant,
    ) -> Result<Value, PolicyError> {
        self.until(deadline, |connection| connection.task(name, argument))
    }

    fn until(
        &mut self,
        deadline: Instant,
        exchange: impl FnOnce(&mut Self) -> Result<Value, PolicyError>,
    ) -> Result<Value, PolicyError> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(PolicyError::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "the deadline passed before the request was sent",
            )));
        }
        // The per-message read timeout of [`Connection::set_timeout`] is put back afterwards.
        let timeout = self.writer.read_timeout()?;
        self.writer.set_write_timeout(Some(left))?;
        self.reader.get_mut().deadline = Some(deadline);
        let result = exchange(self);
        self.reader.get_mut().deadline = None;
        let restored = self
            .writer
            .set_read_timeout(timeout)
            .and_then(|()| self.writer.set_write_timeout(None));
        let value = result?;
        restored?;
        Ok(value)
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
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, Instant};

    use serde_json::Value;

    use super::{Connection, parse_server_line};
    use crate::error::PolicyError;

    /// A server that accepts the password, greets, and then never answers: the stand-in for
    /// an Isabelle server that has stopped responding.
    fn silent_server() -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            writeln!(&stream, "OK {{\"isabelle_name\":\"Silent\"}}").unwrap();
            // Read what the client sends, answer nothing, and keep the socket open.
            let mut sink = Vec::new();
            let _ = std::io::Read::read_to_end(&mut reader, &mut sink);
        });
        port
    }

    #[test]
    fn a_call_with_a_deadline_gives_up_at_the_deadline() {
        let (mut connection, greeting) =
            Connection::open(silent_server(), "secret", Duration::from_secs(30)).unwrap();
        assert_eq!(greeting["isabelle_name"], "Silent");
        for task in [false, true] {
            let started = Instant::now();
            let deadline = started + Duration::from_millis(200);
            let result = if task {
                connection.task_until("use_theories", &Value::Null, deadline)
            } else {
                connection.call_until("echo", &Value::Null, deadline)
            };
            let waited = started.elapsed();
            assert!(matches!(result, Err(PolicyError::Io(_))), "{result:?}");
            assert!(
                waited >= Duration::from_millis(190) && waited < Duration::from_millis(900),
                "waited {waited:?}"
            );
        }
    }

    #[test]
    fn a_deadline_already_past_is_an_error_at_once() {
        let (mut connection, _) =
            Connection::open(silent_server(), "secret", Duration::from_secs(30)).unwrap();
        let started = Instant::now();
        let result = connection.call_until("echo", &Value::Null, started);
        assert!(matches!(result, Err(PolicyError::Io(_))), "{result:?}");
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    /// The group kill must reach a grandchild, as it must reach Java behind the `isabelle`
    /// wrapper and what Java starts.
    #[cfg(unix)]
    #[test]
    fn killing_a_group_ends_the_shell_and_its_children() {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30 & echo $!; wait"])
            .stdout(Stdio::piped())
            .process_group(0);
        let mut shell = command.spawn().unwrap();
        let mut line = String::new();
        BufReader::new(shell.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let sleep: u32 = line.trim().parse().unwrap();
        let alive = |pid: u32| {
            Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        };
        assert!(alive(sleep));

        super::kill_group(shell.id()).unwrap();
        let status = shell.wait().unwrap();
        assert!(!status.success(), "the shell was killed: {status}");
        // The orphaned `sleep` is reaped by init shortly after it dies.
        let started = Instant::now();
        while alive(sleep) && started.elapsed() < Duration::from_secs(5) {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!alive(sleep), "the sleep {sleep} outlived the group kill");
    }

    #[test]
    fn reads_the_server_address_line() {
        assert_eq!(
            parse_server_line("server \"fabric\" = 127.0.0.1:4711 (password \"abc-123\")\n"),
            Some((4711, "abc-123".to_owned()))
        );
        assert_eq!(parse_server_line("Copying skeleton files."), None);
    }
}
