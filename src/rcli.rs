//! rcli integration: finding the binary, its `--json` commands, and the shell lines
//! give types into a project terminal for its interactive commands.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    time::Duration,
};
use wait_timeout::ChildExt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Agent {
    Claude,
    Codex,
}
impl Agent {
    pub const ALL: [Self; 2] = [Self::Claude, Self::Codex];
    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
    pub fn other(self) -> Self {
        match self {
            Self::Claude => Self::Codex,
            Self::Codex => Self::Claude,
        }
    }
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|agent| agent.id() == text)
    }
    /// The agent of a tmux session named `rcli-<project-id>-<agent>`.
    pub fn of_session(name: &str) -> Option<Self> {
        Self::parse(name.rsplit('-').next()?)
    }
}

/// Directories GUI apps often miss from PATH, where rcli and its tools are installed.
fn extra_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join("go/bin"));
        dirs.push(home.join("bin"));
    }
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    dirs
}

/// PATH of the user's login shell plus common install directories. A Finder-launched
/// app inherits a minimal PATH without ssh, rsync or rcli itself.
pub fn search_path() -> &'static OsString {
    static PATH: OnceLock<OsString> = OnceLock::new();
    PATH.get_or_init(|| {
        let mut dirs: Vec<PathBuf> = login_shell_path()
            .map(|path| std::env::split_paths(&path).collect())
            .unwrap_or_default();
        if let Some(current) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&current));
        }
        dirs.extend(extra_dirs());
        let mut unique = Vec::new();
        for dir in dirs {
            if !dir.as_os_str().is_empty() && !unique.contains(&dir) {
                unique.push(dir);
            }
        }
        std::env::join_paths(unique).unwrap_or_default()
    })
}

fn login_shell_path() -> Option<OsString> {
    if cfg!(windows) {
        return None;
    }
    const MARKER: &str = "__GIVE_PATH__";
    let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
    let mut child = Command::new(shell)
        .args(["-l", "-c", &format!("printf '\\n{MARKER}%s' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    if child.wait_timeout(Duration::from_secs(3)).ok()?.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let text = reader.join().ok()?;
    let path = text.rsplit_once(MARKER)?.1.trim();
    (!path.is_empty()).then(|| path.into())
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|| path.into()),
        None => path.into(),
    }
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    metadata.is_file()
}

/// The rcli binary: the configured path when one is set, otherwise the first
/// `rcli` on the search path.
pub fn locate(configured: &str) -> Option<PathBuf> {
    let configured = configured.trim();
    if !configured.is_empty() {
        let path = expand_home(configured);
        return executable(&path).then_some(path);
    }
    let name = if cfg!(windows) { "rcli.exe" } else { "rcli" };
    std::env::split_paths(search_path())
        .map(|dir| dir.join(name))
        .find(|path| executable(path))
}

/// Quotes `text` for a POSIX shell command line.
pub fn shell_quote(text: &str) -> String {
    if !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-+=:@%,".contains(&b))
    {
        return text.into();
    }
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Session {
    pub name: String,
    pub attached: bool,
}
impl Session {
    pub fn agent(&self) -> Option<Agent> {
        Agent::of_session(&self.name)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Status {
    pub id: String,
    pub local: String,
    pub remote: String,
    pub client_active: bool,
    pub upload_pending: u64,
    pub download_pending: u64,
    pub conflicts: Vec<String>,
    pub sessions: Vec<Session>,
    pub updated: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Conversation {
    pub agent: String,
    pub id: String,
    pub title: String,
    pub updated: String,
    pub dangerous: bool,
}

/// An rcli client connected to this project, from the local answer cache.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Client {
    pub session: String,
    pub agent: String,
    pub pid: u32,
    pub active: bool,
    pub conversation: String,
    pub markdown: String,
}
impl Client {
    pub fn agent(&self) -> Option<Agent> {
        Agent::parse(&self.agent).or_else(|| Agent::of_session(&self.session))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SyncStats {
    #[serde(alias = "Uploaded")]
    pub uploaded: u64,
    #[serde(alias = "Downloaded")]
    pub downloaded: u64,
    #[serde(alias = "Conflicts")]
    pub conflicts: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Upgrade {
    pub agent: String,
    pub before: String,
    pub after: String,
    pub changed: bool,
    pub error: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct CleanItem {
    pub name: String,
    pub kind: String,
    pub root: String,
    pub when: String,
    pub reason: String,
    pub bytes: i64,
    pub remove: bool,
    pub active: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct CleanReport {
    pub host: String,
    pub disk_total_bytes: i64,
    pub disk_used_bytes: i64,
    pub storage_bytes: i64,
    pub planned_bytes: i64,
    pub items: Vec<CleanItem>,
    pub freed_bytes: i64,
    pub failed: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Version {
    pub version: String,
    pub commit: String,
}

/// A located rcli binary run for one project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rcli {
    pub binary: PathBuf,
    pub project: PathBuf,
}

impl Rcli {
    pub fn new(binary: PathBuf, project: PathBuf) -> Self {
        Self { binary, project }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(&self.binary);
        command
            .arg("--json")
            .args(args)
            .current_dir(&self.project)
            .env("PATH", search_path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    /// Every JSON document `rcli --json ARGS` printed; a trailing `{"error"}` fails.
    pub fn json_values(&self, args: &[&str], timeout: Duration) -> Result<Vec<serde_json::Value>> {
        let mut child = self
            .command(args)
            .spawn()
            .with_context(|| format!("Starting {}", self.binary.display()))?;
        let mut stdout = child.stdout.take().context("rcli stdout")?;
        let mut stderr = child.stderr.take().context("rcli stderr")?;
        let out = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stdout.read_to_string(&mut text);
            text
        });
        let err = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        let status = match child.wait_timeout(timeout)? {
            Some(status) => status,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                bail!(
                    "rcli {} timed out after {}s",
                    args.join(" "),
                    timeout.as_secs()
                );
            }
        };
        let stdout = out.join().unwrap_or_default();
        let stderr = err.join().unwrap_or_default();
        parse_documents(&stdout, status.success(), &stderr)
    }

    pub fn json<T: DeserializeOwned>(&self, args: &[&str], timeout: Duration) -> Result<T> {
        let value = self
            .json_values(args, timeout)?
            .into_iter()
            .next()
            .unwrap_or(serde_json::Value::Null);
        serde_json::from_value(value).context("Reading rcli JSON")
    }

    pub fn status(&self) -> Result<Status> {
        self.json(&["status"], Duration::from_secs(45))
    }
    pub fn version(&self) -> Result<Version> {
        self.json(&["version"], Duration::from_secs(10))
    }
    /// Active clients of this project; reads local files only.
    pub fn clients(&self) -> Result<Vec<Client>> {
        let clients: Option<Vec<Client>> =
            self.json(&["last-message", "--list"], Duration::from_secs(10))?;
        Ok(clients
            .unwrap_or_default()
            .into_iter()
            .filter(|client| client.active)
            .collect())
    }
    pub fn conversations(&self) -> Result<Vec<Conversation>> {
        let list: Option<Vec<Conversation>> =
            self.json(&["resume", "--list"], Duration::from_secs(45))?;
        Ok(list.unwrap_or_default())
    }
    pub fn all_sessions(&self) -> Result<Vec<Session>> {
        let list: Option<Vec<Session>> = self.json(&["sessions"], Duration::from_secs(45))?;
        Ok(list.unwrap_or_default())
    }
    pub fn logs(&self, lines: usize) -> Result<Vec<serde_json::Map<String, serde_json::Value>>> {
        let lines = lines.to_string();
        Ok(self
            .json_values(&["logs", "-n", &lines], Duration::from_secs(15))?
            .into_iter()
            .filter_map(|value| match value {
                serde_json::Value::Object(map) => Some(map),
                _ => None,
            })
            .collect())
    }
    pub fn sync(&self, prefer: Option<&str>) -> Result<SyncStats> {
        match prefer {
            Some(side) => self.json(&["sync", "--prefer", side], Duration::from_secs(300)),
            None => self.json(&["sync"], Duration::from_secs(300)),
        }
    }
    /// Stops this project's agent sessions, or every project's with `all`.
    pub fn stop(&self, all: bool) -> Result<Vec<String>> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Stopped {
            stopped: Option<Vec<String>>,
        }
        let args: &[&str] = if all { &["stop", "--all"] } else { &["stop"] };
        let stopped: Stopped = self.json(args, Duration::from_secs(60))?;
        Ok(stopped.stopped.unwrap_or_default())
    }
    pub fn doctor(&self) -> Result<Vec<(String, String)>> {
        let report: serde_json::Map<String, serde_json::Value> =
            self.json(&["doctor"], Duration::from_secs(60))?;
        Ok(report
            .into_iter()
            .map(|(key, value)| (key, plain(&value)))
            .collect())
    }
    pub fn config(&self) -> Result<serde_json::Value> {
        self.json(&["config", "show"], Duration::from_secs(10))
    }
    pub fn upgrade(&self, agent: Option<Agent>) -> Result<Vec<Upgrade>> {
        let mut args = vec!["upgrade"];
        args.extend(agent.map(Agent::id));
        let list: Option<Vec<Upgrade>> = self.json(&args, Duration::from_secs(900))?;
        Ok(list.unwrap_or_default())
    }
    pub fn clean(&self, older_than: &str, dry_run: bool) -> Result<CleanReport> {
        let mode = if dry_run { "--dry-run" } else { "--yes" };
        self.json(
            &["clean", "--older-than", older_than, mode],
            Duration::from_secs(300),
        )
    }
    pub fn remove(&self) -> Result<()> {
        self.json_values(&["remove"], Duration::from_secs(120))
            .map(drop)
    }

    fn line(&self, args: &[&str]) -> String {
        std::iter::once(self.binary.to_string_lossy().as_ref())
            .chain(args.iter().copied())
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ")
    }
    /// The shell line that attaches `agent` with live synchronization.
    pub fn agent_line(&self, agent: Agent, dangerous: bool) -> String {
        let mut args = Vec::new();
        if dangerous {
            args.push("-y");
        }
        args.extend(["--", agent.id()]);
        self.line(&args)
    }
    pub fn resume_line(&self, id: &str) -> String {
        self.line(&["resume", id])
    }
    pub fn login_line(&self, agent: Agent) -> String {
        self.line(&["login", agent.id()])
    }
    pub fn shell_line(&self) -> String {
        self.line(&["shell"])
    }
    pub fn watch_line(&self) -> String {
        self.line(&["watch"])
    }
    pub fn takeover_line(&self, agent: Agent, dangerous: bool) -> String {
        let mut args = vec!["--takeover"];
        if dangerous {
            args.push("-y");
        }
        args.extend(["--", agent.id()]);
        self.line(&args)
    }
}

/// Splits rcli's stdout into JSON documents. JSON Lines and an error document after
/// a partial result both occur.
fn parse_documents(stdout: &str, success: bool, stderr: &str) -> Result<Vec<serde_json::Value>> {
    let mut values = Vec::new();
    for value in serde_json::Deserializer::from_str(stdout).into_iter::<serde_json::Value>() {
        match value {
            Ok(value) => values.push(value),
            Err(error) if values.is_empty() && !success => {
                bail!("{}", last_line(stderr).unwrap_or_else(|| error.to_string()))
            }
            Err(error) => return Err(error).context("Reading rcli JSON"),
        }
    }
    if let Some(message) = values
        .last()
        .and_then(|value| value.get("error"))
        .and_then(|error| error.as_str())
        .filter(|_| {
            values
                .last()
                .is_some_and(|value| value.get("code").is_some())
        })
    {
        bail!("{message}");
    }
    if !success && values.is_empty() {
        bail!(
            "{}",
            last_line(stderr).unwrap_or_else(|| "rcli failed without output".into())
        );
    }
    Ok(values)
}

fn last_line(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(Into::into)
}

/// A JSON value as display text: strings unquoted.
pub fn plain(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub fn format_bytes(bytes: i64) -> String {
    let mut value = bytes.max(0) as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if value < 1024. {
            return if unit == "B" {
                format!("{value:.0} {unit}")
            } else {
                format!("{value:.1} {unit}")
            };
        }
        value /= 1024.;
    }
    format!("{value:.1} TB")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_only_what_the_shell_would_split() {
        assert_eq!(
            shell_quote("/Users/me/.local/bin/rcli"),
            "/Users/me/.local/bin/rcli"
        );
        assert_eq!(
            shell_quote("/Applications/My Tools/rcli"),
            "'/Applications/My Tools/rcli'"
        );
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn agent_lines_pass_agent_after_separator() {
        let rcli = Rcli::new("/opt/rcli bin/rcli".into(), "/tmp".into());
        assert_eq!(
            rcli.agent_line(Agent::Claude, false),
            "'/opt/rcli bin/rcli' -- claude"
        );
        assert_eq!(
            rcli.agent_line(Agent::Codex, true),
            "'/opt/rcli bin/rcli' -y -- codex"
        );
        assert_eq!(
            rcli.resume_line("01a0fd26"),
            "'/opt/rcli bin/rcli' resume 01a0fd26"
        );
    }

    #[test]
    fn sessions_name_their_agent() {
        assert_eq!(
            Agent::of_session("rcli-give-31506148707f-claude"),
            Some(Agent::Claude)
        );
        assert_eq!(Agent::of_session("rcli-a-b-codex"), Some(Agent::Codex));
        assert_eq!(Agent::of_session("rcli-give-31506148707f-sh"), None);
    }

    #[test]
    fn reads_status_errors_and_json_lines() {
        let status: Status = serde_json::from_value(
            parse_documents(
                r#"{"client_active":true,"conflicts":["a.rs"],"download_pending":0,"id":"give-1","sessions":[{"name":"rcli-give-1-claude","attached":true}],"upload_pending":2}"#,
                true,
                "",
            )
            .unwrap()
            .remove(0),
        )
        .unwrap();
        assert!(status.client_active);
        assert_eq!(status.conflicts, ["a.rs"]);
        assert_eq!(status.upload_pending, 2);
        assert_eq!(status.sessions[0].agent(), Some(Agent::Claude));

        let error = parse_documents(
            "{\"Uploaded\":0,\"Downloaded\":1,\"Conflicts\":[\"x\"]}\n{\"error\":\"concurrent edits preserved\",\"code\":1}\n",
            false,
            "",
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "concurrent edits preserved");

        let lines = parse_documents("{\"msg\":\"a\"}\n{\"msg\":\"b\"}\n", true, "").unwrap();
        assert_eq!(lines.len(), 2);
        assert!(
            parse_documents("", false, "ssh: connection refused\n")
                .unwrap_err()
                .to_string()
                .contains("connection refused")
        );
        let stats: SyncStats =
            serde_json::from_str(r#"{"Uploaded":3,"Downloaded":1,"Conflicts":null}"#).unwrap();
        assert_eq!((stats.uploaded, stats.downloaded), (3, 1));
    }

    #[test]
    fn configured_path_must_be_executable() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("rcli");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        assert_eq!(locate(file.to_str().unwrap()), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(locate(file.to_str().unwrap()), Some(file));
        }
    }
}
