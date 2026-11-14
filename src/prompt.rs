//! The prompt bar's model: which agent owns a terminal, the commands it accepts and
//! the bar heights the user dragged.
use crate::rcli::Agent;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command as Process,
};

/// A built-in command of an agent, typed as `/name` into its prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    pub name: &'static str,
    pub description: &'static str,
    /// Opens a picker or dialog in the terminal that the user operates there.
    pub interactive: bool,
}

const fn command(name: &'static str, description: &'static str, interactive: bool) -> Command {
    Command {
        name,
        description,
        interactive,
    }
}

const CLAUDE: &[Command] = &[
    command("model", "Choose the model", true),
    command("resume", "Resume a conversation", true),
    command("clear", "Start a new conversation", false),
    command(
        "compact",
        "Summarize the conversation to free context",
        false,
    ),
    command("context", "Show context usage", true),
    command("cost", "Show cost and duration", false),
    command("status", "Show version, model and account", true),
    command("permissions", "Allow and deny tools", true),
    command("agents", "Manage subagents", true),
    command("mcp", "Manage MCP servers", true),
    command("memory", "Edit memory files", true),
    command("config", "Open settings", true),
    command("init", "Write CLAUDE.md for this project", false),
    command("review", "Review a pull request", false),
    command("doctor", "Check the installation", true),
    command("help", "Show help and commands", true),
];

const CODEX: &[Command] = &[
    command("model", "Choose the model and reasoning effort", true),
    command("approvals", "Choose what runs without approval", true),
    command("new", "Start a new conversation", false),
    command("resume", "Resume a conversation", true),
    command(
        "compact",
        "Summarize the conversation to free context",
        false,
    ),
    command("diff", "Show the Git diff", false),
    command("status", "Show session configuration and usage", false),
    command("review", "Review current changes", true),
    command("mention", "Mention a file", true),
    command("mcp", "List MCP tools", false),
    command("init", "Write AGENTS.md for this project", false),
    command("quit", "Exit Codex", false),
];

pub fn commands(agent: Agent) -> &'static [Command] {
    match agent {
        Agent::Claude => CLAUDE,
        Agent::Codex => CODEX,
    }
}

/// The command a single-line message names, such as `/model` or `/compact focus on X`.
pub fn command_of(agent: Agent, text: &str) -> Option<&'static Command> {
    let text = text.trim();
    let name = text.strip_prefix('/')?.split_whitespace().next()?;
    if text.contains('\n') {
        return None;
    }
    commands(agent).iter().find(|command| command.name == name)
}

/// The agent among `ps -A -o pgid= -o args=` rows of process group `group`.
/// Matches the agent binary itself, its Node launcher and `rcli … -- claude`.
pub fn agent_in_processes(processes: &str, group: u32) -> Option<Agent> {
    processes.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        if fields.next()?.parse::<u32>().ok()? != group {
            return None;
        }
        fields.find_map(|argument| {
            let name = argument.rsplit('/').next().unwrap_or(argument);
            let name = name.strip_suffix(".js").unwrap_or(name);
            match name {
                "claude" => Some(Agent::Claude),
                "codex" => Some(Agent::Codex),
                _ if name.starts_with("codex-") => Some(Agent::Codex),
                _ => None,
            }
        })
    })
}

/// The agent running in process group `group`, if any. Blocks on `ps`.
pub fn detect_agent(group: u32) -> Option<Agent> {
    let output = Process::new("ps")
        .args(["-A", "-o", "pgid=", "-o", "args="])
        .output()
        .ok()?;
    agent_in_processes(&String::from_utf8_lossy(&output.stdout), group)
}

/// Input area heights in pixels the user dragged, per project and agent.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Heights(BTreeMap<String, BTreeMap<Agent, f32>>);

impl Heights {
    pub fn store() -> Option<PathBuf> {
        crate::recent::store().map(|p| p.with_file_name("prompt-heights.json"))
    }

    /// The stored heights; a missing or broken file is empty.
    pub fn load(file: &Path) -> Self {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> anyhow::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(file, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn get(&self, project: &Path, agent: Agent) -> Option<f32> {
        self.0
            .get(project.to_string_lossy().as_ref())?
            .get(&agent)
            .copied()
    }

    /// Records a height, or forgets it with `None`.
    pub fn set(&mut self, project: &Path, agent: Agent, height: Option<f32>) {
        let key = project.to_string_lossy().into_owned();
        match height {
            Some(height) => {
                self.0.entry(key).or_default().insert(agent, height);
            }
            None => {
                if let Some(agents) = self.0.get_mut(&key) {
                    agents.remove(&agent);
                    if agents.is_empty() {
                        self.0.remove(&key);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_agents_by_binary_launcher_and_rcli_in_their_group_only() {
        let processes = "\
  100 -zsh
  200 node /opt/homebrew/bin/claude --dangerously-skip-permissions
  300 /Users/me/.local/bin/rcli -y -- codex
  400 /usr/local/lib/node_modules/@openai/codex/bin/codex.js
  500 /tmp/codex-aarch64-apple-darwin
  600 vim /Users/me/claude/notes.md
";
        assert_eq!(agent_in_processes(processes, 100), None);
        assert_eq!(agent_in_processes(processes, 200), Some(Agent::Claude));
        assert_eq!(agent_in_processes(processes, 300), Some(Agent::Codex));
        assert_eq!(agent_in_processes(processes, 400), Some(Agent::Codex));
        assert_eq!(agent_in_processes(processes, 500), Some(Agent::Codex));
        assert_eq!(agent_in_processes(processes, 600), None);
        assert_eq!(agent_in_processes(processes, 700), None);
    }

    #[test]
    fn recognizes_known_single_line_commands_with_arguments() {
        let model = command_of(Agent::Claude, " /model ").unwrap();
        assert!(model.interactive);
        assert_eq!(
            command_of(Agent::Claude, "/compact keep the API notes").map(|c| c.name),
            Some("compact")
        );
        assert_eq!(
            command_of(Agent::Codex, "/approvals").map(|c| c.name),
            Some("approvals")
        );
        assert_eq!(command_of(Agent::Claude, "/approvals"), None);
        assert_eq!(command_of(Agent::Claude, "/compact\nmore"), None);
        assert_eq!(command_of(Agent::Claude, "fix /model parsing"), None);
    }

    #[test]
    fn heights_belong_to_a_project_and_agent_pair() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested/prompt-heights.json");
        let (a, b) = (Path::new("/a"), Path::new("/b"));
        let mut heights = Heights::load(&file);
        heights.set(a, Agent::Claude, Some(120.));
        heights.set(a, Agent::Codex, Some(200.));
        heights.set(b, Agent::Claude, Some(80.));
        heights.save(&file).unwrap();
        let mut heights = Heights::load(&file);
        assert_eq!(heights.get(a, Agent::Claude), Some(120.));
        assert_eq!(heights.get(a, Agent::Codex), Some(200.));
        assert_eq!(heights.get(b, Agent::Codex), None);
        heights.set(b, Agent::Claude, None);
        assert_eq!(heights, {
            let mut only_a = Heights::default();
            only_a.set(a, Agent::Claude, Some(120.));
            only_a.set(a, Agent::Codex, Some(200.));
            only_a
        });
        std::fs::write(&file, "broken").unwrap();
        assert_eq!(Heights::load(&file), Heights::default());
    }

    #[test]
    fn every_agent_offers_a_model_picker() {
        for agent in Agent::ALL {
            assert!(
                commands(agent)
                    .iter()
                    .any(|c| c.name == "model" && c.interactive)
            );
        }
    }
}
