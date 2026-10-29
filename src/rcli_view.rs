//! The rcli panel: project status, conversations, the journal, the VPS and cleanup.
//! Reads run as `rcli --json` in the background; agent and terminal actions go
//! through the project window, which owns the terminal.
use crate::ui::Give;
use give::{
    rcli::{self, Agent, CleanReport, Conversation, Rcli, Session, Status, Upgrade},
    theme,
};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Sizable, WindowExt,
    button::{Button, ButtonVariants},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Overview,
    Conversations,
    Logs,
    Server,
    Cleanup,
}

enum Load<T> {
    Idle,
    Loading,
    Ready(T),
    Failed(String),
}
impl<T> Load<T> {
    fn loading(&self) -> bool {
        matches!(self, Self::Loading)
    }
}

type LogEvent = serde_json::Map<String, serde_json::Value>;

pub struct RcliView {
    give: WeakEntity<Give>,
    rcli: Rcli,
    section: Section,
    status: Load<Status>,
    conversations: Load<Vec<Conversation>>,
    agent_filter: Option<Agent>,
    logs: Load<Vec<LogEvent>>,
    doctor: Load<Vec<(String, String)>>,
    sessions: Load<Vec<Session>>,
    upgrade: Load<Vec<Upgrade>>,
    config: Load<String>,
    older_than: &'static str,
    clean: Load<CleanReport>,
    message: Option<(String, bool)>,
}

pub fn open(
    give: WeakEntity<Give>,
    rcli: Rcli,
    section: Section,
    window: &mut Window,
    cx: &mut App,
) {
    if window.has_active_dialog(cx) {
        return;
    }
    let view = cx.new(|cx| {
        let mut view = RcliView {
            give,
            rcli,
            section,
            status: Load::Idle,
            conversations: Load::Idle,
            agent_filter: None,
            logs: Load::Idle,
            doctor: Load::Idle,
            sessions: Load::Idle,
            upgrade: Load::Idle,
            config: Load::Idle,
            older_than: "24h",
            clean: Load::Idle,
            message: None,
        };
        view.show(section, cx);
        view
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(div().px_5().py_4().child("rcli"))
            .width(px(880.))
            .margin_top(px(24.))
            .p_0()
            .gap_0()
            .child(view.clone())
    });
}

/// Loads `run` in the background into the field `slot` picks.
fn load<T: Send + 'static>(
    this: &mut RcliView,
    cx: &mut Context<RcliView>,
    slot: fn(&mut RcliView) -> &mut Load<T>,
    run: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) {
    *slot(this) = Load::Loading;
    let task = cx.background_executor().spawn(async move { run() });
    cx.spawn(async move |view, cx| {
        let result = task.await;
        let _ = view.update(cx, |this, cx| {
            *slot(this) = match result {
                Ok(value) => Load::Ready(value),
                Err(error) => Load::Failed(format!("{error:#}")),
            };
            cx.notify();
        });
    })
    .detach();
    cx.notify();
}

impl RcliView {
    fn show(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        match section {
            Section::Overview if matches!(self.status, Load::Idle) => self.load_status(cx),
            Section::Conversations if matches!(self.conversations, Load::Idle) => {
                self.load_conversations(cx)
            }
            Section::Logs if matches!(self.logs, Load::Idle) => self.load_logs(cx),
            Section::Server if matches!(self.sessions, Load::Idle) => {
                self.load_sessions(cx);
                self.load_config(cx);
            }
            _ => {}
        }
        cx.notify();
    }

    fn load_status(&mut self, cx: &mut Context<Self>) {
        let rcli = self.rcli.clone();
        load(self, cx, |this| &mut this.status, move || rcli.status());
    }
    fn load_conversations(&mut self, cx: &mut Context<Self>) {
        let rcli = self.rcli.clone();
        load(
            self,
            cx,
            |this| &mut this.conversations,
            move || rcli.conversations(),
        );
    }
    fn load_logs(&mut self, cx: &mut Context<Self>) {
        let rcli = self.rcli.clone();
        load(
            self,
            cx,
            |this| &mut this.logs,
            move || {
                let mut logs = rcli.logs(200)?;
                logs.reverse();
                Ok(logs)
            },
        );
    }
    fn load_sessions(&mut self, cx: &mut Context<Self>) {
        let rcli = self.rcli.clone();
        load(
            self,
            cx,
            |this| &mut this.sessions,
            move || rcli.all_sessions(),
        );
    }
    fn load_config(&mut self, cx: &mut Context<Self>) {
        let rcli = self.rcli.clone();
        load(
            self,
            cx,
            |this| &mut this.config,
            move || Ok(serde_json::to_string_pretty(&rcli.config()?)?),
        );
    }
    fn load_clean(&mut self, cx: &mut Context<Self>) {
        let rcli = self.rcli.clone();
        let older_than = self.older_than;
        load(
            self,
            cx,
            |this| &mut this.clean,
            move || rcli.clean(older_than, true),
        );
    }

    fn say(&mut self, message: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        self.message = Some((message.into(), error));
        cx.notify();
    }

    /// Hands an action to the project window and closes the panel.
    fn in_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut Give, &mut Window, &mut Context<Give>),
    ) {
        window.close_dialog(cx);
        let _ = self.give.update(cx, |give, cx| action(give, window, cx));
    }

    /// A non-interactive command that refreshes status when it finishes.
    fn command<T: Send + 'static>(
        &mut self,
        label: &str,
        run: impl FnOnce(Rcli) -> anyhow::Result<T> + Send + 'static,
        report: impl FnOnce(T) -> String + 'static,
        cx: &mut Context<Self>,
    ) {
        self.say(label, false, cx);
        let rcli = self.rcli.clone();
        let task = cx.background_executor().spawn(async move { run(rcli) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |this, cx| {
                match result {
                    Ok(value) => this.say(report(value), false, cx),
                    Err(error) => this.say(format!("{error:#}"), true, cx),
                }
                this.load_status(cx);
                let _ = this.give.update(cx, |give, cx| give.rcli_poll(cx));
            });
        })
        .detach();
    }

    fn overview(&self, cx: &mut Context<Self>) -> Div {
        let theme = theme::current();
        let p = &theme.colors;
        let running = self
            .give
            .read_with(cx, |give, cx| {
                (
                    give.rcli_running(cx),
                    give.rcli_operation().map(str::to_owned),
                )
            })
            .unwrap_or((crate::ui::Running::None, None));
        let (running, operation) = running;
        let busy = operation.is_some();
        let mut agents = div().flex().flex_wrap().gap_2();
        match running.agent() {
            None if running == crate::ui::Running::None => {
                for agent in Agent::ALL {
                    agents = agents.child(
                        Button::new(("rcli-start", agent as usize))
                            .small()
                            .primary()
                            .label(format!("Start {}", agent.label()))
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.in_window(window, cx, |give, window, cx| {
                                    give.rcli_start(agent, window, cx)
                                })
                            })),
                    );
                }
            }
            current => {
                for agent in Agent::ALL.into_iter().filter(|a| Some(*a) != current) {
                    agents = agents.child(
                        Button::new(("rcli-switch", agent as usize))
                            .small()
                            .primary()
                            .label(format!("Switch to {}", agent.label()))
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.in_window(window, cx, |give, window, cx| {
                                    give.rcli_start(agent, window, cx)
                                })
                            })),
                    );
                }
                if matches!(running, crate::ui::Running::Elsewhere(_)) {
                    agents = agents.child(
                        Button::new("rcli-takeover")
                            .small()
                            .label("Take over here")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.in_window(window, cx, Give::rcli_takeover)
                            })),
                    );
                }
                agents = agents.child(
                    Button::new("rcli-stop")
                        .small()
                        .danger()
                        .label("Stop agent")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.in_window(window, cx, Give::rcli_stop)
                        })),
                );
            }
        }
        let summary = match running {
            crate::ui::Running::None => "No agent is running for this project.".to_owned(),
            crate::ui::Running::Here(agent) => format!(
                "{} is running in this window's terminal.",
                agent.map_or("An agent", Agent::label)
            ),
            crate::ui::Running::Elsewhere(agent) => format!(
                "{} is running in another rcli client.",
                agent.map_or("An agent", Agent::label)
            ),
        };
        let status = match &self.status {
            Load::Idle | Load::Loading => muted("Reading status from the VPS…"),
            Load::Failed(error) => failure(error),
            Load::Ready(status) => {
                let mut table = div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(kv("Local", status.local.clone()))
                    .child(kv("VPS", status.remote.clone()))
                    .child(kv(
                        "Client",
                        if status.client_active {
                            "connected and syncing"
                        } else {
                            "not running"
                        },
                    ))
                    .child(kv(
                        "Pending",
                        format!(
                            "{} to upload · {} to download",
                            status.upload_pending, status.download_pending
                        ),
                    ))
                    .child(kv(
                        "Sessions",
                        if status.sessions.is_empty() {
                            "none".to_owned()
                        } else {
                            status
                                .sessions
                                .iter()
                                .map(|session| {
                                    format!(
                                        "{}{}",
                                        session
                                            .agent()
                                            .map_or(session.name.as_str(), |a| a.label()),
                                        if session.attached { " (attached)" } else { "" }
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join(", ")
                        },
                    ));
                if !status.conflicts.is_empty() {
                    table = table.child(
                        div()
                            .mt_2()
                            .p_3()
                            .rounded(px(5.))
                            .border_1()
                            .border_color(rgb(p.red))
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_color(rgb(p.red))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!(
                                        "{} conflicting files — edited on both sides",
                                        status.conflicts.len()
                                    )),
                            )
                            .children(status.conflicts.iter().take(12).map(|path| {
                                div()
                                    .font_family(give::settings::font_family())
                                    .text_size(px(11.))
                                    .child(path.clone())
                            }))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        Button::new("prefer-local")
                                            .small()
                                            .label("Keep Mac versions")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.resolve("local", cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("prefer-remote")
                                            .small()
                                            .label("Keep VPS versions")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.resolve("remote", cx)
                                            })),
                                    ),
                            ),
                    );
                }
                table
            }
        };
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(heading(
                "Project",
                "Agent session and sync state of this project.",
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(div().child(operation.unwrap_or(summary)))
                    .child(agents),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Sync"))
                            .child(
                                Button::new("status-refresh")
                                    .ghost()
                                    .xsmall()
                                    .label("Refresh")
                                    .disabled(self.status.loading())
                                    .on_click(cx.listener(|this, _, _, cx| this.load_status(cx))),
                            ),
                    )
                    .child(status),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("sync-now")
                            .small()
                            .label("Sync now")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.command(
                                    "Syncing…",
                                    |rcli| rcli.sync(None),
                                    |stats| {
                                        format!(
                                            "Synced: {} uploaded, {} downloaded",
                                            stats.uploaded, stats.downloaded
                                        )
                                    },
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("vps-shell")
                            .small()
                            .label("Open VPS shell")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let line = this.rcli.shell_line();
                                this.in_window(window, cx, move |give, window, cx| {
                                    give.rcli_interactive(line, window, cx)
                                })
                            })),
                    )
                    .child(
                        Button::new("watch")
                            .small()
                            .label("Sync without agent")
                            .tooltip("rcli watch: live sync in the terminal, no agent")
                            .disabled(running != crate::ui::Running::None)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let line = this.rcli.watch_line();
                                this.in_window(window, cx, move |give, window, cx| {
                                    give.rcli_interactive(line, window, cx)
                                })
                            })),
                    )
                    .child(
                        Button::new("remove-workspace")
                            .small()
                            .danger()
                            .label("Remove VPS copy…")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.confirm_remove(window, cx)),
                            ),
                    ),
            )
    }

    fn resolve(&mut self, side: &'static str, cx: &mut Context<Self>) {
        self.command(
            "Resolving conflicts…",
            move |rcli| rcli.sync(Some(side)),
            move |_| {
                format!(
                    "Conflicts resolved with the {} versions",
                    if side == "local" { "Mac" } else { "VPS" }
                )
            },
            cx,
        );
    }

    fn confirm_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(
            PromptLevel::Warning,
            "Remove this project's VPS copy?",
            Some(
                "Deletes the VPS workspace, its backups and agent sessions. Files on this Mac stay. The next rcli start copies the project again.",
            ),
            &["Remove", "Cancel"],
            cx,
        );
        cx.spawn(async move |view, cx| {
            if answer.await == Ok(0) {
                let _ = view.update(cx, |this, cx| {
                    this.command(
                        "Removing the VPS copy…",
                        |rcli| rcli.remove(),
                        |_| "VPS copy removed".into(),
                        cx,
                    )
                });
            }
        })
        .detach();
    }

    fn conversations(&self, cx: &mut Context<Self>) -> Div {
        let theme = theme::current();
        let p = &theme.colors;
        let filter = self.agent_filter;
        let list = match &self.conversations {
            Load::Idle | Load::Loading => muted("Reading conversations from the VPS…"),
            Load::Failed(error) => failure(error),
            Load::Ready(list) => {
                let rows: Vec<_> = list
                    .iter()
                    .filter(|c| filter.is_none_or(|agent| agent.id() == c.agent))
                    .take(100)
                    .cloned()
                    .collect();
                if rows.is_empty() {
                    muted("No saved conversations.")
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .children(rows.into_iter().enumerate().map(|(i, conversation)| {
                            let id = conversation.id.clone();
                            div()
                                .id(("conversation", i))
                                .flex()
                                .items_center()
                                .gap_3()
                                .px_2()
                                .py_2()
                                .border_b_1()
                                .border_color(rgb(p.border))
                                .hover(|el| el.bg(rgb(p.surface)))
                                .child(badge(
                                    Agent::parse(&conversation.agent)
                                        .map_or(conversation.agent.as_str(), |a| a.label())
                                        .to_owned(),
                                    p.accent,
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .child(
                                            div().truncate().child(
                                                conversation
                                                    .title
                                                    .lines()
                                                    .next()
                                                    .unwrap_or("Untitled")
                                                    .to_owned(),
                                            ),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(10.))
                                                .text_color(rgb(p.muted))
                                                .child(format!(
                                                    "{} · {}{}",
                                                    short_time(&conversation.updated),
                                                    &conversation.id
                                                        [..conversation.id.len().min(8)],
                                                    if conversation.dangerous {
                                                        " · -y"
                                                    } else {
                                                        ""
                                                    }
                                                )),
                                        ),
                                )
                                .child(Button::new(("resume", i)).small().label("Resume").on_click(
                                    cx.listener(move |this, _, window, cx| {
                                        let id = id.clone();
                                        this.in_window(window, cx, move |give, window, cx| {
                                            give.rcli_resume(id, window, cx)
                                        })
                                    }),
                                ))
                        }))
                }
            }
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(heading(
                "Conversations",
                "Saved Claude and Codex conversations of this project. Resuming stops a running agent first.",
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .children(
                        [(None, "All"), (Some(Agent::Claude), "Claude"), (Some(Agent::Codex), "Codex")]
                            .into_iter()
                            .enumerate()
                            .map(|(i, (agent, label))| {
                                Button::new(("agent-filter", i))
                                    .small()
                                    .label(label)
                                    .when(filter == agent, |b| b.primary())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.agent_filter = agent;
                                        cx.notify();
                                    }))
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("conversations-refresh")
                            .ghost()
                            .xsmall()
                            .label("Refresh")
                            .disabled(self.conversations.loading())
                            .on_click(cx.listener(|this, _, _, cx| this.load_conversations(cx))),
                    ),
            )
            .child(list)
    }

    fn logs(&self, cx: &mut Context<Self>) -> Div {
        let theme = theme::current();
        let p = &theme.colors;
        let list = match &self.logs {
            Load::Idle | Load::Loading => muted("Reading the journal…"),
            Load::Failed(error) => failure(error),
            Load::Ready(events) if events.is_empty() => muted("The journal is empty."),
            Load::Ready(events) => {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(events.iter().map(|event| {
                        let field = |key: &str| event.get(key).map(rcli::plain).unwrap_or_default();
                        let level = field("level");
                        let details = event
                            .iter()
                            .filter(|(key, _)| !["time", "level", "msg"].contains(&key.as_str()))
                            .map(|(key, value)| format!("{key}={}", rcli::plain(value)))
                            .collect::<Vec<_>>()
                            .join("  ");
                        div()
                            .flex()
                            .gap_3()
                            .font_family(give::settings::font_family())
                            .text_size(px(11.))
                            .child(
                                div()
                                    .w(px(130.))
                                    .flex_shrink_0()
                                    .text_color(rgb(p.muted))
                                    .child(field("time")),
                            )
                            .child(
                                div()
                                    .w(px(44.))
                                    .flex_shrink_0()
                                    .text_color(rgb(match level.as_str() {
                                        "ERROR" => p.red,
                                        "WARN" => p.yellow,
                                        _ => p.muted,
                                    }))
                                    .child(level),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(field("msg"))
                                    .when(!details.is_empty(), |el| {
                                        el.child(div().text_color(rgb(p.muted)).child(details))
                                    }),
                            )
                    }))
            }
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .child(heading(
                        "Journal",
                        "Sessions, transfers, conflicts and connection changes, newest first.",
                    ))
                    .child(
                        Button::new("logs-refresh")
                            .ghost()
                            .xsmall()
                            .label("Refresh")
                            .disabled(self.logs.loading())
                            .on_click(cx.listener(|this, _, _, cx| this.load_logs(cx))),
                    ),
            )
            .child(list)
    }

    fn server(&self, cx: &mut Context<Self>) -> Div {
        let theme = theme::current();
        let p = &theme.colors;
        let doctor = match &self.doctor {
            Load::Idle => {
                muted("Checks SSH, rsync, tmux, curl, the browser and the agents on the VPS.")
            }
            Load::Loading => muted("Checking the VPS…"),
            Load::Failed(error) => failure(error),
            Load::Ready(checks) => div().flex().flex_col().gap_2().children(
                checks
                    .iter()
                    .map(|(key, value)| kv_owned(key.clone(), value.clone())),
            ),
        };
        let upgrade = match &self.upgrade {
            Load::Idle => div(),
            Load::Loading => muted("Updating agents on the VPS… this can take a few minutes."),
            Load::Failed(error) => failure(error),
            Load::Ready(results) => {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(results.iter().map(|result| {
                        kv_owned(
                            result.agent.clone(),
                            if !result.error.is_empty() {
                                format!("failed: {}", result.error)
                            } else if result.changed {
                                format!("{} → {}", result.before, result.after)
                            } else {
                                format!("{} (up to date)", result.after)
                            },
                        )
                    }))
            }
        };
        let sessions = match &self.sessions {
            Load::Idle | Load::Loading => muted("Reading sessions…"),
            Load::Failed(error) => failure(error),
            Load::Ready(list) if list.is_empty() => muted("No rcli sessions run on the VPS."),
            Load::Ready(list) => {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(list.iter().map(|session| {
                        div()
                            .flex()
                            .gap_2()
                            .font_family(give::settings::font_family())
                            .text_size(px(11.))
                            .child(session.name.clone())
                            .when(session.attached, |el| {
                                el.child(div().text_color(rgb(p.green)).child("attached"))
                            })
                    }))
            }
        };
        let upgrading = self.upgrade.loading();
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(heading("VPS", "The server rcli runs agents on."))
            .child(
                section_box("Health").child(doctor).child(
                    div().child(
                        Button::new("doctor")
                            .small()
                            .label(if self.doctor.loading() {
                                "Checking…"
                            } else {
                                "Run checks"
                            })
                            .disabled(self.doctor.loading())
                            .on_click(cx.listener(|this, _, _, cx| {
                                let rcli = this.rcli.clone();
                                load(this, cx, |this| &mut this.doctor, move || rcli.doctor());
                            })),
                    ),
                ),
            )
            .child(
                section_box("Agents").child(upgrade).child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(upgrade_button(
                            "upgrade-all",
                            "Update all",
                            None,
                            upgrading,
                            cx,
                        ))
                        .child(upgrade_button(
                            "upgrade-claude",
                            "Update Claude",
                            Some(Agent::Claude),
                            upgrading,
                            cx,
                        ))
                        .child(upgrade_button(
                            "upgrade-codex",
                            "Update Codex",
                            Some(Agent::Codex),
                            upgrading,
                            cx,
                        ))
                        .children(Agent::ALL.into_iter().map(|agent| {
                            Button::new(("login", agent as usize))
                                .small()
                                .label(format!("Sign in to {}…", agent.label()))
                                .tooltip(
                                    "Runs rcli login in a terminal; the browser opens from the VPS",
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let line = this.rcli.login_line(agent);
                                    this.in_window(window, cx, move |give, window, cx| {
                                        give.rcli_interactive(line, window, cx)
                                    })
                                }))
                        })),
                ),
            )
            .child(
                section_box("Sessions of every project")
                    .child(sessions)
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("sessions-refresh")
                                    .small()
                                    .label("Refresh")
                                    .disabled(self.sessions.loading())
                                    .on_click(cx.listener(|this, _, _, cx| this.load_sessions(cx))),
                            )
                            .child(
                                Button::new("stop-all")
                                    .small()
                                    .danger()
                                    .label("Stop all sessions…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_stop_all(window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                section_box("Configuration").child(match &self.config {
                    Load::Idle | Load::Loading => muted("Reading configuration…"),
                    Load::Failed(error) => failure(error),
                    Load::Ready(text) => div()
                        .p_3()
                        .rounded(px(5.))
                        .bg(rgb(p.panel))
                        .font_family(give::settings::font_family())
                        .text_size(px(11.))
                        .child(text.clone()),
                }),
            )
    }

    fn confirm_stop_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(
            PromptLevel::Warning,
            "Stop every project's rcli sessions?",
            Some("Agents of all projects stop on the VPS. Their conversations stay available to resume."),
            &["Stop All", "Cancel"],
            cx,
        );
        cx.spawn(async move |view, cx| {
            if answer.await == Ok(0) {
                let _ = view.update(cx, |this, cx| {
                    this.command(
                        "Stopping all sessions…",
                        |rcli| rcli.stop(true),
                        |stopped| format!("Stopped {} sessions", stopped.len()),
                        cx,
                    );
                    this.load_sessions(cx);
                });
            }
        })
        .detach();
    }

    fn cleanup(&self, cx: &mut Context<Self>) -> Div {
        let theme = theme::current();
        let p = &theme.colors;
        let plan = match &self.clean {
            Load::Idle => muted("Preview what would be removed before cleaning."),
            Load::Loading => muted("Inspecting the VPS…"),
            Load::Failed(error) => failure(error),
            Load::Ready(report) => {
                let removing: Vec<_> = report.items.iter().filter(|item| item.remove).collect();
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(kv_owned(
                        "Disk".into(),
                        format!(
                            "{} of {} used · rcli storage {}",
                            rcli::format_bytes(report.disk_used_bytes),
                            rcli::format_bytes(report.disk_total_bytes),
                            rcli::format_bytes(report.storage_bytes)
                        ),
                    ))
                    .when(report.failed.is_some(), |el| {
                        el.child(kv_owned(
                            "Freed".into(),
                            format!(
                                "{}{}",
                                rcli::format_bytes(report.freed_bytes),
                                match report.failed {
                                    Some(failed) if failed > 0 => format!(" · {failed} failed"),
                                    _ => String::new(),
                                }
                            ),
                        ))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .children(report.items.iter().map(|item| {
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_3()
                                    .py_1p5()
                                    .border_b_1()
                                    .border_color(rgb(p.border))
                                    .text_color(rgb(if item.remove { p.text } else { p.muted }))
                                    .child(
                                        div()
                                            .w(px(16.))
                                            .text_color(rgb(p.red))
                                            .child(if item.remove { "✕" } else { "" }),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .flex_col()
                                            .child(div().truncate().child(item.name.clone()))
                                            .child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(rgb(p.muted))
                                                    .truncate()
                                                    .child(if item.root.is_empty() {
                                                        item.kind.clone()
                                                    } else {
                                                        item.root.clone()
                                                    }),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .w(px(140.))
                                            .text_size(px(10.))
                                            .child(format!("{} · {}", item.when, item.reason)),
                                    )
                                    .child(
                                        div()
                                            .w(px(70.))
                                            .text_right()
                                            .child(rcli::format_bytes(item.bytes)),
                                    )
                            })),
                    )
                    .when(report.failed.is_none(), |el| {
                        el.child(
                            div().child(
                                Button::new("clean-run")
                                    .small()
                                    .danger()
                                    .label(format!(
                                        "Remove {} items · {}",
                                        removing.len(),
                                        rcli::format_bytes(report.planned_bytes)
                                    ))
                                    .disabled(removing.is_empty())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_clean(window, cx)
                                    })),
                            ),
                        )
                    })
            }
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(heading(
                "Cleanup",
                "Free VPS space: copies of projects unused for a while and old helper builds. Running projects are kept; Mac files never change.",
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(div().mr_2().child("Unused for"))
                    .children(["6h", "24h", "72h", "168h"].into_iter().enumerate().map(|(i, age)| {
                        Button::new(("older-than", i))
                            .small()
                            .label(if age == "168h" { "7d" } else { age })
                            .when(self.older_than == age, |b| b.primary())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.older_than = age;
                                this.load_clean(cx);
                            }))
                    }))
                    .child(div().flex_1())
                    .child(
                        Button::new("clean-preview")
                            .small()
                            .label("Preview")
                            .disabled(self.clean.loading())
                            .on_click(cx.listener(|this, _, _, cx| this.load_clean(cx))),
                    ),
            )
            .child(plan)
    }

    fn confirm_clean(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(
            PromptLevel::Warning,
            "Remove the marked VPS copies?",
            Some(
                "Their next rcli start copies them again. Conversations stay available to resume.",
            ),
            &["Remove", "Cancel"],
            cx,
        );
        cx.spawn(async move |view, cx| {
            if answer.await == Ok(0) {
                let _ = view.update(cx, |this, cx| {
                    let rcli = this.rcli.clone();
                    let older_than = this.older_than;
                    load(
                        this,
                        cx,
                        |this| &mut this.clean,
                        move || rcli.clean(older_than, false),
                    );
                });
            }
        })
        .detach();
    }
}

fn upgrade_button(
    id: &'static str,
    label: &'static str,
    agent: Option<Agent>,
    upgrading: bool,
    cx: &mut Context<RcliView>,
) -> Button {
    Button::new(id)
        .small()
        .label(label)
        .disabled(upgrading)
        .on_click(cx.listener(move |this, _, _, cx| {
            let rcli = this.rcli.clone();
            load(
                this,
                cx,
                |this| &mut this.upgrade,
                move || rcli.upgrade(agent),
            );
        }))
}

impl Render for RcliView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current();
        let p = &theme.colors;
        let height = (f32::from(window.viewport_size().height) - 180.).clamp(320., 560.);
        let content = match self.section {
            Section::Overview => self.overview(cx),
            Section::Conversations => self.conversations(cx),
            Section::Logs => self.logs(cx),
            Section::Server => self.server(cx),
            Section::Cleanup => self.cleanup(cx),
        };
        div()
            .debug_selector(|| "rcli-dialog".into())
            .flex()
            .flex_col()
            .text_size(px(12.))
            .text_color(rgb(p.text))
            .child(
                div()
                    .flex()
                    .h(px(height))
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .child(
                        div()
                            .w(px(160.))
                            .flex_shrink_0()
                            .p_3()
                            .bg(rgb(p.panel))
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(
                                [
                                    (Section::Overview, "Project"),
                                    (Section::Conversations, "Conversations"),
                                    (Section::Logs, "Journal"),
                                    (Section::Server, "VPS"),
                                    (Section::Cleanup, "Cleanup"),
                                ]
                                .into_iter()
                                .enumerate()
                                .map(|(i, (section, label))| {
                                    div()
                                        .id(("rcli-section", i))
                                        .debug_selector(move || format!("rcli-section-{i}"))
                                        .px_3()
                                        .py_2()
                                        .rounded(px(5.))
                                        .cursor_pointer()
                                        .bg(rgb(if self.section == section {
                                            p.selected
                                        } else {
                                            p.panel
                                        }))
                                        .hover(|el| el.bg(rgb(p.surface)))
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.show(section, cx)
                                            }),
                                        )
                                        .child(label)
                                }),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .px_2()
                                    .text_size(px(10.))
                                    .text_color(rgb(p.muted))
                                    .truncate()
                                    .child(self.rcli.binary.display().to_string()),
                            ),
                    )
                    .child(
                        div()
                            .id("rcli-scroll")
                            .flex_1()
                            .min_w_0()
                            .overflow_y_scroll()
                            .p_5()
                            .bg(rgb(p.background))
                            .child(content),
                    ),
            )
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .px_5()
                    .py_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .children(self.message.clone().map(|(message, error)| {
                                div()
                                    .text_color(rgb(if error { p.red } else { p.muted }))
                                    .child(message)
                            })),
                    )
                    .child(
                        Button::new("rcli-settings")
                            .ghost()
                            .small()
                            .label("Settings…")
                            .on_click(|_, window, cx| {
                                window.close_dialog(cx);
                                crate::settings_view::open_rcli(window, cx);
                            }),
                    )
                    .child(
                        Button::new("rcli-close")
                            .small()
                            .label("Close")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}

fn heading(title: &'static str, detail: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(theme::current().colors.muted))
                .child(detail),
        )
}
fn section_box(title: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
}
fn kv(key: &'static str, value: impl Into<SharedString>) -> Div {
    kv_owned(key.into(), value.into().to_string())
}
fn kv_owned(key: String, value: String) -> Div {
    div()
        .flex()
        .gap_3()
        .child(
            div()
                .w(px(90.))
                .flex_shrink_0()
                .text_color(rgb(theme::current().colors.muted))
                .child(key),
        )
        .child(div().flex_1().min_w_0().child(value))
}
fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_color(rgb(theme::current().colors.muted))
        .child(text.into())
}
fn failure(text: &str) -> Div {
    div()
        .text_color(rgb(theme::current().colors.red))
        .child(text.to_owned())
}
fn badge(text: String, color: u32) -> Div {
    div()
        .px_1p5()
        .rounded(px(3.))
        .border_1()
        .border_color(rgb(color))
        .text_size(px(10.))
        .text_color(rgb(color))
        .flex_shrink_0()
        .child(text)
}
/// `2026-10-28T05:37:01.18Z` as `2026-10-28 05:37`.
fn short_time(time: &str) -> String {
    time.get(..16).unwrap_or(time).replace('T', " ")
}
