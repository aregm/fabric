//! The Isabelle server process and its line protocol (Isabelle system manual, "The Isabelle
//! server"): a password line, then commands `name argument`, answered by `OK`/`ERROR`, or by
//! `FINISHED`/`FAILED` for asynchronous tasks; long messages are length-prefixed.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
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
    /// The handle through which any thread can kill the server ([`Server::kill_handle`]).
    handle: KillHandle,
    pub port: u16,
    pub password: String,
}

/// Kills a server started with [`Server::start_in_group_with`] from any thread, at any time:
/// while it starts, while its owner waits on it, or after it stopped (then it does nothing).
///
/// A killed handle stays killed: a start with it is refused, so a new gate needs a new handle.
/// A handle serves one server at a time: a start with a handle whose server is still running
/// is refused too.
/// It cannot reach a server that `isabelle server` found already running under the same
/// name, because that server is not a child of this process. On Unix the kill ends the
/// server's process group and the groups led by processes below it; elsewhere [`KillHandle::kill`]
/// only sets the flag.
#[derive(Clone, Debug, Default)]
pub struct KillHandle {
    state: Arc<Mutex<KillState>>,
}

#[derive(Debug, Default)]
struct KillState {
    /// The process group of a grouped server whose wrapper is still unreaped. Its id is the
    /// wrapper's pid, which cannot be reused while the wrapper is an unreaped child, so it is
    /// cleared, under the lock, wherever the wrapper is reaped.
    #[cfg(unix)]
    group: Option<u32>,
    killed: bool,
}

impl KillState {
    /// Whether the group of a server that is not reaped yet is registered. A handle serves
    /// one server at a time.
    fn in_use(&self) -> bool {
        #[cfg(unix)]
        {
            self.group.is_some()
        }
        #[cfg(not(unix))]
        {
            false
        }
    }

    /// Records `child`'s group when it leads one of its own.
    fn register(&mut self, child: &Child, grouped: bool) {
        #[cfg(unix)]
        if grouped {
            self.group = Some(child.id());
        }
        #[cfg(not(unix))]
        let _ = (child, grouped);
    }

    /// The wrapper `child` was reaped: its pid, and so the group id, may be reused from now
    /// on. A group registered for another, later server stays.
    fn reaped(&mut self, child: &Child) {
        #[cfg(unix)]
        if self.group == Some(child.id()) {
            self.group = None;
        }
        #[cfg(not(unix))]
        let _ = child;
    }
}

impl KillHandle {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks the handle killed for good and, if a started server's group is registered, ends
    /// that group and the groups led by the processes below it, with a helper process that
    /// finishes its work even if this process ends meanwhile. It waits for the helper up to
    /// [`KILL_HELPER_WAIT`] and is an error if the helper could not run or did not finish (the
    /// server's own group is then killed directly). It does not reap the server: its owner
    /// does. Calling it again is harmless.
    pub fn kill(&self) -> io::Result<()> {
        let mut state = self.lock();
        state.killed = true;
        #[cfg(unix)]
        if let Some(group) = state.group {
            return end_groups(group).map(|_| ());
        }
        Ok(())
    }

    /// Whether [`KillHandle::kill`] was called.
    #[must_use]
    pub fn is_killed(&self) -> bool {
        self.lock().killed
    }

    fn lock(&self) -> MutexGuard<'_, KillState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
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
        Self::start_with(options, false, &KillHandle::new())
    }

    /// Starts the named server as [`Server::start`] does, but on Unix in a process group of
    /// its own, so that [`Server::kill`] ends the Java server and everything it started, not
    /// only the `isabelle` shell wrapper (which runs Java as a child). Elsewhere it is
    /// [`Server::start`]. It is [`Server::start_in_group_with`] with a new handle.
    pub fn start_in_group(options: &ServerOptions<'_>) -> Result<Self, PolicyError> {
        Self::start_in_group_with(options, &KillHandle::new())
    }

    /// [`Server::start_in_group`], killable through `handle` from the moment the process
    /// exists: the group is registered with the handle before the handle's lock is released,
    /// so a [`KillHandle::kill`] that arrives during the spawn finds it. A handle already
    /// killed refuses the start without spawning anything. A kill while the server has not
    /// printed its address makes the start an error at once.
    pub fn start_in_group_with(
        options: &ServerOptions<'_>,
        handle: &KillHandle,
    ) -> Result<Self, PolicyError> {
        Self::start_with(options, cfg!(unix), handle)
    }

    fn start_with(
        options: &ServerOptions<'_>,
        grouped: bool,
        handle: &KillHandle,
    ) -> Result<Self, PolicyError> {
        // Lock order: `STARTING`, then the handle's lock; nothing takes them the other way.
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
        let mut child = spawn_registered(&mut command, grouped, handle)?;
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
                handle: handle.clone(),
                port,
                password,
            }),
            Ok(None) => {
                kill_child(&mut child, grouped, handle, None);
                Err(PolicyError::Isabelle(if handle.is_killed() {
                    "isabelle server was killed through its kill handle before printing its address"
                        .to_owned()
                } else {
                    "isabelle server exited before printing its address".to_owned()
                }))
            }
            Err(_) => {
                kill_child(&mut child, grouped, handle, None);
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
            if self.reap_if_exited() {
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
        self.reap_if_exited()
    }

    /// Reaps the wrapper if it has exited, holding the handle's lock, and then forgets its
    /// group.
    fn reap_if_exited(&mut self) -> bool {
        let mut state = self.handle.lock();
        let exited = matches!(self.child.try_wait(), Ok(Some(_)));
        if exited {
            state.reaped(&self.child);
        }
        exited
    }

    /// Kills the server: for one started with [`Server::start_in_group`] on Unix, every
    /// process of its group (the wrapper, Java, and what Java started in that group) and of
    /// the groups that processes below it lead (Isabelle runs each `poly` in a group of its
    /// own), as [`KillHandle::kill`] does but without marking the handle killed, waiting up to
    /// [`KILL_WAIT`] for them to be gone and for the port to be closed, and reaps the wrapper;
    /// otherwise it kills the process this value holds, as before.
    pub fn kill(&mut self) {
        kill_child(&mut self.child, self.grouped, &self.handle, Some(self.port));
    }

    /// A handle that kills this server from any thread ([`KillHandle`]). For a server
    /// started with [`Server::start`] it reaches nothing.
    #[must_use]
    pub fn kill_handle(&self) -> KillHandle {
        self.handle.clone()
    }
}

/// How long [`Server::kill`] waits for the killed processes of a server started with
/// [`Server::start_in_group`] to be gone and its port closed.
pub const KILL_WAIT: Duration = Duration::from_secs(2);

/// How long [`KillHandle::kill`] and [`Server::kill`] wait for the helper process that ends
/// a server's groups. It needs some tens of milliseconds.
#[cfg(unix)]
pub const KILL_HELPER_WAIT: Duration = Duration::from_secs(2);

/// Spawns `command` under `handle`'s lock, unless the handle is killed or still belongs to a
/// running server, and registers the child's group with it before the lock is released.
fn spawn_registered(
    command: &mut Command,
    grouped: bool,
    handle: &KillHandle,
) -> Result<Child, PolicyError> {
    let mut state = handle.lock();
    if state.killed {
        return Err(PolicyError::Isabelle(
            "the server's kill handle was killed before the server started; a new start needs a new handle"
                .to_owned(),
        ));
    }
    if state.in_use() {
        return Err(PolicyError::Isabelle(
            "the server's kill handle already belongs to a running server; each server needs its own handle"
                .to_owned(),
        ));
    }
    let child = command.spawn()?;
    state.register(&child, grouped);
    Ok(child)
}

/// Kills `child` as before; when `grouped`, it first ends the registered group and the groups
/// below it ([`end_groups`]), then reaps the child and forgets the group, all under the
/// handle's lock, and then waits up to [`KILL_WAIT`] for the groups to be gone and `port`
/// closed.
fn kill_child(child: &mut Child, grouped: bool, handle: &KillHandle, port: Option<u16>) {
    if !grouped {
        let _ = child.kill();
        return;
    }
    let started = Instant::now();
    let mut state = handle.lock();
    // A registered group belongs to the unreaped child (even if it has exited and Java was
    // re-parented), so its id is still ours to signal. A group registered for another server
    // is not this child's to end.
    #[cfg(unix)]
    let groups = match state.group {
        Some(group) if group == child.id() => end_groups(group).unwrap_or_else(|_| vec![group]),
        _ => Vec::new(),
    };
    let _ = child.kill();
    let _ = child.wait();
    state.reaped(child);
    drop(state);
    #[cfg(unix)]
    {
        // `kill` fails if any operand fails, so each group is asked on its own.
        let alive = |group: &u32| signal_groups("0", &[*group]).is_ok();
        while groups.iter().any(alive) && started.elapsed() < KILL_WAIT {
            thread::sleep(Duration::from_millis(10));
        }
    }
    // A killed JVM holds its listening socket for a few milliseconds after `kill -0` stops
    // finding it.
    if let Some(port) = port {
        while TcpStream::connect(("127.0.0.1", port)).is_ok() && started.elapsed() < KILL_WAIT {
            thread::sleep(Duration::from_millis(10));
        }
    }
}

/// The script of the kill helper, run as `sh -c SCRIPT sh GROUP`. It stops the group, so it
/// starts nothing new while the processes are listed; computes from `ps` the groups to end:
/// those whose leader is below the server, where "below" starts from every process of the
/// group (so `poly` is still found when the wrapper has died and Java was re-parented) and
/// follows parent pids, and never any other group; kills those groups and the server's own;
/// and only then prints the groups it ended below the server, one per line. It ignores
/// `SIGPIPE`, so a caller that went away does not stop it.
#[cfg(unix)]
const KILL_SCRIPT: &str = r#"trap '' PIPE
g=$1
kill -s STOP -- "-$g" 2>/dev/null
below=$(ps -A -o pid=,ppid=,pgid= | awk -v g="$g" '
{ pid[NR] = $1; ppid[NR] = $2; pgid[NR] = $3 }
END {
  for (i = 1; i <= NR; i++) if (pgid[i] == g) below[pid[i]] = 1
  for (grown = 1; grown; ) {
    grown = 0
    for (i = 1; i <= NR; i++)
      if (!(pid[i] in below) && (ppid[i] in below)) { below[pid[i]] = 1; grown = 1 }
  }
  for (i = 1; i <= NR; i++)
    if (pid[i] == pgid[i] && pid[i] != g && (pid[i] in below)) print pid[i]
}')
for group in $below; do kill -s KILL -- "-$group" 2>/dev/null; done
kill -s KILL -- "-$g" 2>/dev/null
for group in $below; do echo "$group"; done
exit 0
"#;

/// Ends `group` and the groups led by processes below it with the kill helper, and returns
/// them all (`group` first). The caller holds the handle's lock with `group` registered.
/// When the helper fails, `group` itself is killed directly and the error returned.
#[cfg(unix)]
fn end_groups(group: u32) -> io::Result<Vec<u32>> {
    let result = run_kill_helper(group);
    if result.is_err() {
        let _ = signal_groups("KILL", &[group]);
    }
    result
}

/// Runs [`KILL_SCRIPT`] for `group` in a process group of its own (so a signal to the
/// caller's group does not reach it) and waits for it up to [`KILL_HELPER_WAIT`]. A helper
/// out of time is killed with its group and reaped, so it signals nothing afterwards.
#[cfg(unix)]
fn run_kill_helper(group: u32) -> io::Result<Vec<u32>> {
    use std::os::unix::process::CommandExt as _;

    let mut helper = Command::new("/bin/sh")
        .args(["-c", KILL_SCRIPT, "sh", &group.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    let started = Instant::now();
    let status = loop {
        match helper.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < KILL_HELPER_WAIT => {
                thread::sleep(Duration::from_millis(5));
            }
            outcome => {
                // The helper's group id is its pid, ours while it is unreaped.
                let _ = signal_groups("KILL", &[helper.id()]);
                let _ = helper.kill();
                let _ = helper.wait();
                return Err(match outcome {
                    Err(error) => error,
                    _ => io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!(
                            "the kill helper for group {group} did not finish within {:.1}s",
                            KILL_HELPER_WAIT.as_secs_f64()
                        ),
                    ),
                });
            }
        }
    };
    let mut output = String::new();
    if let Some(mut stdout) = helper.stdout.take() {
        stdout.read_to_string(&mut output)?;
    }
    if !status.success() {
        return Err(io::Error::other(format!(
            "the kill helper for group {group} exited with {status}"
        )));
    }
    let mut groups = vec![group];
    groups.extend(
        output
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok()),
    );
    Ok(groups)
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

    /// Connects, sends the password, waits for the greeting, sends the command `name` without
    /// an argument, and closes the connection without waiting for an answer; everything is
    /// over by `deadline`. `Ok` says only that the command was written after the greeting.
    ///
    /// The Isabelle server reads a client's first command only after it has sent the greeting,
    /// and a client that closes its socket with the greeting still unread may reset the
    /// connection and so lose the command; read first, the command is followed by an orderly
    /// close, and the server reads it before it sees the end of the stream.
    pub fn send_unanswered(
        port: u16,
        password: &str,
        name: &str,
        deadline: Instant,
    ) -> Result<(), PolicyError> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(PolicyError::Io(io::Error::new(
                io::ErrorKind::TimedOut,
                "the deadline passed before connecting",
            )));
        }
        let stream = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), left)?;
        stream.set_write_timeout(Some(left))?;
        let mut connection = Self {
            reader: BufReader::new(DeadlineStream {
                stream: stream.try_clone()?,
                deadline: Some(deadline),
            }),
            writer: stream,
        };
        writeln!(connection.writer, "{password}")?;
        let (kind, _) = connection.read()?;
        if kind != "OK" {
            return Err(PolicyError::Protocol(format!(
                "the server rejected the password ({kind})"
            )));
        }
        connection.send(name, &Value::Null)?;
        connection.writer.flush()?;
        connection.writer.shutdown(Shutdown::Write)?;
        Ok(())
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

    /// `shutdown` without waiting for an answer: written after the greeting, and read by the
    /// server, however soon the client closes.
    #[test]
    fn a_command_sent_unanswered_reaches_the_server() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sender, receiver) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut password = String::new();
            reader.read_line(&mut password).unwrap();
            // A slow greeting: the client must wait for it.
            thread::sleep(Duration::from_millis(50));
            writeln!(&stream, "OK {{}}").unwrap();
            let mut command = String::new();
            reader.read_line(&mut command).unwrap();
            let mut rest = Vec::new();
            let _ = std::io::Read::read_to_end(&mut reader, &mut rest);
            sender.send((password, command, rest)).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        Connection::send_unanswered(port, "secret", "shutdown", deadline).unwrap();
        let (password, command, rest) = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(password, "secret\n");
        assert_eq!(command, "shutdown\n");
        assert!(rest.is_empty(), "the client closed after the command");

        // A deadline already past sends nothing.
        let error = Connection::send_unanswered(port, "secret", "shutdown", Instant::now());
        assert!(matches!(error, Err(PolicyError::Io(_))), "{error:?}");
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

    #[cfg(unix)]
    mod kill_handle {
        use std::io::{BufRead, BufReader};
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::thread;
        use std::time::{Duration, Instant};

        use super::super::{KillHandle, Server, spawn_registered};

        fn alive(pid: u32) -> bool {
            Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        }

        /// Waits up to five seconds for `pid` to be gone (an orphan is reaped by init shortly
        /// after it dies).
        fn gone(pid: u32) -> bool {
            let started = Instant::now();
            while alive(pid) && started.elapsed() < Duration::from_secs(5) {
                thread::sleep(Duration::from_millis(10));
            }
            !alive(pid)
        }

        fn group_of(pid: u32) -> u32 {
            let output = Command::new("ps")
                .args(["-o", "pgid=", "-p", &pid.to_string()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse()
                .unwrap()
        }

        fn registered(handle: &KillHandle) -> Option<u32> {
            handle.lock().group
        }

        /// A stand-in for a grouped server: a shell leading a group of its own, registered
        /// with `handle`, with a `sleep` in a group of its own below it (as Isabelle runs
        /// `poly`) and a `sleep` in its group (as the wrapper runs Java). It returns the two
        /// sleeps' pids.
        fn fake_server(handle: &KillHandle) -> (Server, u32, u32) {
            // `bash`, because `dash` (the `sh` of Debian and Ubuntu) turns job control off
            // without a terminal, and `set -m` is what gives the first sleep its own group.
            let mut command = Command::new("/bin/bash");
            command
                .args([
                    "-c",
                    "set -m; sleep 30 & echo $!; set +m; sleep 30 & echo $!; wait",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .process_group(0);
            let mut child = spawn_registered(&mut command, true, handle).unwrap();
            let mut reader = BufReader::new(child.stdout.take().unwrap());
            let mut pid = || {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                line.trim().parse::<u32>().unwrap()
            };
            let (below, inside) = (pid(), pid());
            assert_eq!(registered(handle), Some(child.id()));
            assert_eq!(
                group_of(below),
                below,
                "the first sleep leads its own group"
            );
            assert_eq!(group_of(inside), child.id(), "the second is in the shell's");
            let server = Server {
                child,
                grouped: true,
                handle: handle.clone(),
                port: 0,
                password: String::new(),
            };
            (server, below, inside)
        }

        fn wait_reaped(server: &mut Server) {
            let started = Instant::now();
            while !server.has_exited() {
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "the shell outlived the kill"
                );
                thread::sleep(Duration::from_millis(10));
            }
        }

        #[test]
        fn a_handle_ends_the_group_and_a_group_below_it() {
            let handle = KillHandle::new();
            let (mut server, below, inside) = fake_server(&handle);
            assert!(alive(below) && alive(inside));
            assert!(!handle.is_killed());

            handle.kill().unwrap();
            assert!(handle.is_killed());
            assert!(
                gone(below),
                "the sleep {below} in a group below outlived the kill"
            );
            assert!(
                gone(inside),
                "the sleep {inside} in the group outlived the kill"
            );
            wait_reaped(&mut server);
            assert_eq!(registered(&handle), None);
            // Twice is fine, and now reaches nothing.
            handle.kill().unwrap();
            assert!(
                server.kill_handle().is_killed(),
                "the server holds the same handle"
            );
        }

        #[test]
        fn a_kill_from_another_thread_ends_a_server_its_owner_waits_on() {
            let handle = KillHandle::new();
            let (mut server, below, inside) = fake_server(&handle);
            let killer = {
                let handle = server.kill_handle();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(300));
                    handle.kill()
                })
            };
            let started = Instant::now();
            // The owner is blocked here, polling the wrapper under the handle's lock.
            server.wait_or_kill(Duration::from_secs(30));
            let waited = started.elapsed();
            killer.join().unwrap().unwrap();
            assert!(waited < Duration::from_secs(3), "waited {waited:?}");
            assert!(gone(below) && gone(inside));
            assert_eq!(registered(&handle), None);
        }

        #[test]
        fn a_handle_killed_before_the_start_refuses_it_without_spawning() {
            let handle = KillHandle::new();
            handle.kill().unwrap();
            let marker = std::env::temp_dir().join(format!(
                "fabric-policy-kill-handle-spawned-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&marker);
            let mut command = Command::new("/bin/sh");
            command
                .arg("-c")
                .arg(format!("echo spawned > '{}'", marker.display()));
            let error = spawn_registered(&mut command, true, &handle).unwrap_err();
            assert!(error.to_string().contains("kill handle"), "{error}");
            thread::sleep(Duration::from_millis(200));
            assert!(!marker.exists(), "nothing was spawned");
            assert_eq!(registered(&handle), None);
        }

        #[test]
        fn after_the_wrapper_is_reaped_the_handle_holds_no_group() {
            let handle = KillHandle::new();
            let mut command = Command::new("/bin/sh");
            command
                .args(["-c", "exit 0"])
                .stdin(Stdio::null())
                .process_group(0);
            let child = spawn_registered(&mut command, true, &handle).unwrap();
            assert_eq!(registered(&handle), Some(child.id()));
            let mut server = Server {
                child,
                grouped: true,
                handle: handle.clone(),
                port: 0,
                password: String::new(),
            };
            wait_reaped(&mut server);
            assert_eq!(registered(&handle), None);
            handle.kill().unwrap();
            handle.kill().unwrap();
            assert!(handle.is_killed());
        }

        #[test]
        fn a_handle_serves_one_server_at_a_time() {
            let handle = KillHandle::new();
            let (mut first, below, inside) = fake_server(&handle);
            // While the first server runs, the handle is refused for a second one.
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "exit 0"]).stdin(Stdio::null());
            let error = spawn_registered(&mut command, true, &handle).unwrap_err();
            assert!(error.to_string().contains("already belongs"), "{error}");
            assert_eq!(registered(&handle), Some(first.child.id()));

            // Once the first is killed and reaped, the handle may serve a second server, and
            // the first server's value can no longer end or forget the second one's group.
            first.kill();
            assert!(gone(below) && gone(inside));
            let (mut second, below, inside) = fake_server(&handle);
            let group = second.child.id();
            first.kill();
            assert!(first.has_exited());
            assert_eq!(registered(&handle), Some(group));
            assert!(alive(below) && alive(inside));
            second.kill();
            assert!(gone(below) && gone(inside));
            assert_eq!(registered(&handle), None);
        }

        #[test]
        fn a_server_kill_goes_the_same_way_and_leaves_the_handle_usable() {
            let handle = KillHandle::new();
            let (mut server, below, inside) = fake_server(&handle);
            let started = Instant::now();
            server.kill();
            eprintln!("Server::kill took {:?}", started.elapsed());
            assert!(gone(below) && gone(inside));
            assert_eq!(registered(&handle), None);
            assert!(!handle.is_killed(), "only the handle's own kill marks it");
            assert!(server.has_exited());
        }
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
