// Running a team from the planner (see Planner.tsx): each manager as a Claude Code session of its own, in the
// background, with its workers as its subagents, while the lead stays the chat the team was added to.
//
// The planner writes what each manager's told and which tools it gets; this starts them and keeps track of them. Each
// runs `claude` in print mode, in the lead's folder, with only the tools its planner boxes allow and everything else
// refused rather than asked about, and with a cap on its turns. When one's done, its report goes to the lead's chat
// the way a message from the list does (see hooks.rs `report`). They're stopped together, from the planner, and when
// Headroom quits.

use std::{
    collections::BTreeMap,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde_json::{json, Value};

/// A manager to start, as the planner describes it.
pub struct Manager {
    /// Its ID in the plan
    pub agent: String,
    pub name: String,
    pub model: String,
    /// What it's told
    pub prompt: String,
    /// The tools it may use, by Claude Code's names
    pub tools: Vec<String>,
    /// Its workers, as Claude Code's `--agents` takes them: by name, each with a description, instructions, a model,
    /// and tools
    pub agents: BTreeMap<String, Value>,
}

impl Manager {
    /// From what the planner sent: an object with each of the fields above.
    pub fn from(v: &Value) -> Result<Manager, String> {
        let text = |key: &str| v[key].as_str().map(String::from).ok_or(format!("A manager is missing its {key}"));
        let tools = v["tools"].as_array().ok_or("A manager is missing its tools")?;
        let agents = v["agents"].as_object().ok_or("A manager is missing its team")?;
        Ok(Manager {
            agent: text("agent")?,
            name: text("name")?,
            model: text("model")?,
            prompt: text("prompt")?,
            tools: tools.iter().filter_map(|t| t.as_str().map(String::from)).collect(),
            agents: agents.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        })
    }
}

/// How far a manager's got.
#[derive(Clone, PartialEq)]
pub enum State {
    Working,
    Done(String),
    Failed(String),
    Stopped,
}

pub struct Member {
    pub agent: String,
    pub name: String,
    pub model: String,
    /// The session ID it was started with, so its hooks' events can be told apart
    pub session: String,
    pub pid: Option<i32>,
    pub state: State,
}

pub struct Run {
    pub id: String,
    /// The team's name, from the planner
    pub name: String,
    /// The lead's session
    pub lead: String,
    pub started: i64,
    pub members: Vec<Member>,
}

#[derive(Default)]
pub struct Teams {
    pub runs: Vec<Run>,
}

/// The tools a manager or worker may be given: what the planner's boxes turn into, and running its workers.
const TOOLS: [&str; 15] = [
    "Read",
    "Glob",
    "Grep",
    "Edit",
    "Write",
    "MultiEdit",
    "NotebookEdit",
    "Bash",
    "WebFetch",
    "WebSearch",
    "Task",
    "Agent",
    "Skill",
    "SlashCommand",
    "TodoWrite",
];
const MODELS: [&str; 3] = ["opus", "sonnet", "haiku"];
/// A backstop: no manager goes on longer than this many turns.
const MAX_TURNS: &str = "200";

/// Is what the planner sent something to start? Known tools and models only, for it and each worker.
pub fn valid(m: &Manager) -> Result<(), String> {
    let tool_ok = |t: &str| TOOLS.contains(&t);
    if !MODELS.contains(&m.model.as_str()) {
        return Err(format!("{} has a model Headroom doesn't know", m.name));
    }
    if !m.tools.iter().all(|t| tool_ok(t)) {
        return Err(format!("{} has a tool Headroom doesn't know", m.name));
    }
    for (name, worker) in &m.agents {
        let tools = worker["tools"].as_array().into_iter().flatten();
        let model_ok = worker["model"].as_str().is_some_and(|model| MODELS.contains(&model));
        if !model_ok || !tools.clone().all(|t| t.as_str().is_some_and(tool_ok)) || !worker["prompt"].is_string() {
            return Err(format!("{name}, on {}'s team, isn't set up in a way Headroom can start", m.name));
        }
    }
    Ok(())
}

/// Where `claude` is: where the user's login shell finds it, as a terminal would, or failing that, the newest one the
/// Claude app keeps.
pub fn claude() -> Option<PathBuf> {
    static FOUND: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
            let out = Command::new(shell).args(["-lc", "command -v claude"]).output().ok();
            let found = out
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|p| p.starts_with('/'))
                .map(PathBuf::from)
                .filter(|p| p.exists());
            found.or_else(|| {
                let home = PathBuf::from(std::env::var_os("HOME")?);
                let dir = home.join("Library/Application Support/Claude/claude-code");
                let mut versions: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
                versions.sort();
                versions.into_iter().rev().map(|v| v.join("claude.app/Contents/MacOS/claude")).find(|p| p.exists())
            })
        })
        .clone()
}

/// Start a manager. Returns its process ID and session ID, and hands its final report to `done` when it's finished.
pub fn start(
    run: &str,
    m: &Manager,
    cwd: &Path,
    done: impl FnOnce(Result<String, String>) + Send + 'static,
) -> Result<(i32, String), String> {
    valid(m)?;
    let claude = claude().ok_or("Headroom can't find Claude Code. Is `claude` installed?")?;
    let session = uuid();
    let agents = serde_json::to_string(&m.agents).map_err(|e| e.to_string())?;
    let mut command = Command::new(claude);
    command
        .current_dir(cwd)
        .args(["-p", &m.prompt, "--model", &m.model, "--permission-mode", "dontAsk"])
        .arg("--allowedTools")
        .args(&m.tools)
        .args(["--agents", &agents, "--session-id", &session, "--output-format", "json", "--max-turns", MAX_TURNS])
        // Not the app's own: Headroom started it, not the user in an app
        .env_remove("__CFBundleIdentifier")
        .env_remove("CLAUDE_CODE_HOST_SESSION_ID")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .env("HEADROOM_TEAM", format!("{run}:{}", m.agent))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Its own process group, so stopping it stops what it's running too
        .process_group(0);
    let child = command.spawn().map_err(|e| format!("Couldn't start {}: {e}", m.name))?;
    let pid = child.id() as i32;
    std::thread::spawn(move || {
        let result = child.wait_with_output().map_err(|e| e.to_string()).and_then(|out| {
            let report: Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
            match (report["result"].as_str(), report["is_error"] == true) {
                (Some(text), false) => Ok(text.to_string()),
                (Some(text), true) => Err(text.to_string()),
                _ => Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("It stopped").to_string()),
            }
        });
        done(result);
    });
    Ok((pid, session))
}

/// Stop a manager, and whatever it's running.
pub fn stop(pid: i32) {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    // SAFETY: signals the process group this started for the manager, which is never Headroom's own
    unsafe { kill(-pid, 15) };
}

/// A random UUID, for a session's ID.
fn uuid() -> String {
    let mut bytes = [0u8; 16];
    let _ = getrandom(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

fn getrandom(bytes: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(bytes)
}

impl Teams {
    /// The runs, for the planner: each manager's name, model, and how it's getting on.
    pub fn to_json(&self) -> Value {
        let runs: Vec<Value> = self
            .runs
            .iter()
            .map(|r| {
                let members: Vec<Value> = r
                    .members
                    .iter()
                    .map(|m| {
                        let (state, text) = match &m.state {
                            State::Working => ("working", String::new()),
                            State::Done(report) => ("done", report.clone()),
                            State::Failed(why) => ("failed", why.clone()),
                            State::Stopped => ("stopped", String::new()),
                        };
                        json!({ "agent": m.agent, "name": m.name, "model": m.model, "session": m.session,
                                "state": state, "text": text })
                    })
                    .collect();
                json!({ "id": r.id, "name": r.name, "lead": r.lead, "started": r.started, "members": members })
            })
            .collect();
        Value::Array(runs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager(model: &str, tools: &[&str], worker: Value) -> Manager {
        Manager {
            agent: "dev".into(),
            name: "Development".into(),
            model: model.into(),
            prompt: "Build it".into(),
            tools: tools.iter().map(|t| t.to_string()).collect(),
            agents: [("coder".to_string(), worker)].into(),
        }
    }

    #[test]
    fn only_known_models_and_tools_start() {
        let worker = json!({ "description": "Coder", "prompt": "Code", "model": "sonnet", "tools": ["Read", "Edit"] });
        assert!(valid(&manager("opus", &["Read", "Task"], worker.clone())).is_ok());
        assert!(valid(&manager("gpt", &["Read"], worker.clone())).is_err());
        assert!(valid(&manager("opus", &["Read", "Bash(rm -rf /)"], worker.clone())).is_err());
        let sneaky = json!({ "description": "Coder", "prompt": "Code", "model": "sonnet", "tools": ["Anything"] });
        assert!(valid(&manager("opus", &["Read"], sneaky)).is_err());
    }

    #[test]
    fn session_ids_are_uuids() {
        let id = uuid();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert_ne!(id, uuid());
    }
}
