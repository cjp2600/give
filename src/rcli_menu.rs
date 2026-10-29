//! rcli in the project window: autostart, the title bar menu and agent lifecycle.
//! Interactive rcli commands are typed into the project terminal; the rest run as
//! `rcli --json` in the background.
use super::Give;
use give::{
    rcli::{self, Agent, Client, Rcli},
    settings::{self, RcliAutostart},
    theme,
};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    menu::{DropdownMenu, PopupMenuItem},
};
use std::{path::PathBuf, time::Duration};

/// How long a stopped agent's local client may take for its final sync and exit.
const STOP_WAIT: Duration = Duration::from_secs(90);

#[derive(Default)]
pub(super) struct State {
    /// The located binary, for the `rcli_path` it was located from.
    binary: Option<PathBuf>,
    located_for: Option<String>,
    /// Active rcli clients of the open project.
    clients: Vec<Client>,
    /// A running stop, switch or background command.
    operation: Option<String>,
    poll_request: u64,
}

/// Where an agent of this project runs, as far as give can tell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Running {
    None,
    Here(Option<Agent>),
    Elsewhere(Option<Agent>),
}
impl Running {
    pub(crate) fn agent(self) -> Option<Agent> {
        match self {
            Self::None => None,
            Self::Here(agent) | Self::Elsewhere(agent) => agent,
        }
    }
}

impl Give {
    pub(crate) fn rcli(&self) -> Option<Rcli> {
        Some(Rcli::new(
            self.rcli.binary.clone()?,
            self.repo.as_ref()?.root.clone(),
        ))
    }

    pub(crate) fn rcli_running(&self, cx: &App) -> Running {
        let Some(client) = self.rcli.clients.first() else {
            return Running::None;
        };
        let here = self
            .project_terminal()
            .and_then(|terminal| terminal.read(cx).foreground());
        if self.rcli.clients.iter().any(|c| Some(c.pid) == here) {
            let client = self
                .rcli
                .clients
                .iter()
                .find(|c| Some(c.pid) == here)
                .unwrap_or(client);
            Running::Here(client.agent())
        } else {
            Running::Elsewhere(client.agent())
        }
    }

    fn project_terminal(&self) -> Option<&Entity<crate::terminal_view::TerminalView>> {
        self.terminals.get(&self.repo.as_ref()?.root)
    }

    fn rcli_notice(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.notice = message.into();
        self.error = false;
        cx.notify();
    }

    /// Locates the binary when settings name a different path; `then` runs after.
    pub(super) fn rcli_locate(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let configured = settings::current().rcli_path;
        // UI tests open projects; they must not start the developer's real rcli.
        if cfg!(test) || self.rcli.located_for.as_ref() == Some(&configured) {
            then(self, window, cx);
            return;
        }
        let task = cx.background_executor().spawn({
            let configured = configured.clone();
            async move { rcli::locate(&configured) }
        });
        cx.spawn_in(window, async move |view, cx| {
            let binary = task.await;
            let _ = view.update_in(cx, |this, window, cx| {
                this.rcli.binary = binary;
                this.rcli.located_for = Some(configured);
                this.rcli_poll(cx);
                then(this, window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Settings changed the binary path: locate it again.
    pub(super) fn rcli_settings_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rcli.located_for.as_ref() != Some(&settings::current().rcli_path) {
            self.rcli_locate(window, cx, |_, _, _| {});
        }
    }

    /// Refreshes the active clients from rcli's local cache; no VPS round trip.
    pub(crate) fn rcli_poll(&mut self, cx: &mut Context<Self>) {
        let Some(rcli) = self.rcli() else {
            if !self.rcli.clients.is_empty() {
                self.rcli.clients.clear();
                cx.notify();
            }
            return;
        };
        self.rcli.poll_request += 1;
        let request = self.rcli.poll_request;
        let task = cx
            .background_executor()
            .spawn(async move { rcli.clients().unwrap_or_default() });
        cx.spawn(async move |view, cx| {
            let clients = task.await;
            let _ = view.update(cx, |this, cx| {
                if this.rcli.poll_request == request && this.rcli.clients != clients {
                    this.rcli.clients = clients;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// A project terminal was just created: start the agent chosen in settings.
    pub(super) fn rcli_autostart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = settings::current().rcli_autostart;
        if mode == RcliAutostart::Off {
            self.rcli_locate(window, cx, |_, _, _| {});
            return;
        }
        let root = self.repo.as_ref().map(|repo| repo.root.clone());
        self.rcli_locate(window, cx, move |this, window, cx| {
            let Some(rcli) = this.rcli() else {
                return;
            };
            if this.repo.as_ref().map(|repo| &repo.root) != root.as_ref() {
                return;
            }
            // Another client already serves this project; a second would only wait.
            let task = cx
                .background_executor()
                .spawn(async move { rcli.clients().unwrap_or_default() });
            cx.spawn_in(window, async move |view, cx| {
                let clients = task.await;
                let _ = view.update_in(cx, |this, window, cx| {
                    if this.repo.as_ref().map(|repo| &repo.root) != root.as_ref() {
                        return;
                    }
                    if let Some(client) = clients.first() {
                        let agent = client.agent().map_or("An agent", Agent::label);
                        this.rcli.clients = clients;
                        this.rcli_notice(
                            format!(
                                "rcli: {agent} is already running for this project in another client"
                            ),
                            cx,
                        );
                        return;
                    }
                    match mode {
                        RcliAutostart::Claude => this.rcli_start(Agent::Claude, window, cx),
                        RcliAutostart::Codex => this.rcli_start(Agent::Codex, window, cx),
                        RcliAutostart::Ask => this.rcli_ask(window, cx),
                        RcliAutostart::Off => {}
                    }
                });
            })
            .detach();
        });
    }

    fn rcli_ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .repo
            .as_ref()
            .and_then(|repo| repo.root.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let answer = window.prompt(
            PromptLevel::Info,
            "Start an agent with rcli?",
            Some(&format!(
                "{name} runs on your VPS with live sync to this Mac.\n\nChoose “Always ask”, Claude, Codex or Off in Settings › rcli."
            )),
            &["Claude", "Codex", "Shell Only"],
            cx,
        );
        cx.spawn_in(window, async move |view, cx| {
            let answer = answer.await;
            let _ = view.update_in(cx, |this, window, cx| match answer {
                Ok(0) => this.rcli_start(Agent::Claude, window, cx),
                Ok(1) => this.rcli_start(Agent::Codex, window, cx),
                _ => {}
            });
        })
        .detach();
    }

    /// Starts `agent`, first stopping another running agent of this project.
    pub(crate) fn rcli_start(&mut self, agent: Agent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rcli) = self.rcli() else {
            return self.show_error("rcli was not found. Choose it in Settings › rcli.", cx);
        };
        let running = self.rcli_running(cx);
        if running != Running::None && running.agent() == Some(agent) {
            return self.rcli_notice(format!("rcli: {} is already running", agent.label()), cx);
        }
        let line = rcli.agent_line(agent, settings::current().rcli_dangerous);
        let label = format!("Starting {}…", agent.label());
        self.rcli_launch(line, label, window, cx);
    }

    /// Continues a saved conversation; rcli keeps its approval mode.
    pub(crate) fn rcli_resume(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rcli) = self.rcli() else {
            return;
        };
        self.rcli_launch(
            rcli.resume_line(&id),
            "Resuming conversation…".into(),
            window,
            cx,
        );
    }

    /// Takes the project over from another rcli client, which exits gracefully.
    pub(crate) fn rcli_takeover(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(rcli) = self.rcli() else {
            return;
        };
        let agent = self.rcli_running(cx).agent().unwrap_or(Agent::Claude);
        let line = rcli.takeover_line(agent, settings::current().rcli_dangerous);
        if !self.rcli_type(&line, cx) {
            self.show_error(
                "The project terminal is busy. Quit its program before taking over.",
                cx,
            );
        }
    }

    /// Runs an agent `line` in the project terminal. A running agent of this project
    /// is stopped first and its client allowed to finish the final sync.
    fn rcli_launch(
        &mut self,
        line: String,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.rcli.operation.is_some() {
            return;
        }
        if self.rcli_running(cx) == Running::None {
            if !self.rcli_type(&line, cx) {
                self.show_error(
                    "The project terminal is busy. Quit its program, then start the agent again.",
                    cx,
                );
            }
            return;
        }
        self.rcli_stop_then(Some(line), label, window, cx);
    }

    /// Stops this project's agents; with `then`, types it once the client has exited.
    fn rcli_stop_then(
        &mut self,
        then: Option<String>,
        label: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(rcli) = self.rcli() else {
            return;
        };
        let stopping = self.rcli_running(cx).agent().map_or("agent", Agent::label);
        self.rcli.operation = Some(format!("Stopping {stopping}…"));
        self.rcli_notice(format!("rcli: stopping {stopping}…"), cx);
        let stop = cx.background_executor().spawn({
            let rcli = rcli.clone();
            async move { rcli.stop(false) }
        });
        cx.spawn_in(window, async move |view, cx| {
            if let Err(error) = stop.await {
                let _ = view.update(cx, |this, cx| {
                    this.rcli.operation = None;
                    this.show_error(format!("rcli stop failed: {error:#}"), cx);
                });
                return;
            }
            // The local client sees its tmux session end, syncs a last time and exits.
            let started = std::time::Instant::now();
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let clients = cx
                    .background_executor()
                    .spawn({
                        let rcli = rcli.clone();
                        async move { rcli.clients().unwrap_or_default() }
                    })
                    .await;
                let Ok(idle) = view.update(cx, |this, cx| {
                    this.rcli.clients = clients.clone();
                    cx.notify();
                    clients.is_empty()
                        && this
                            .project_terminal()
                            .is_none_or(|terminal| !terminal.read(cx).busy())
                }) else {
                    return;
                };
                if idle {
                    break;
                }
                if started.elapsed() > STOP_WAIT {
                    let _ = view.update(cx, |this, cx| {
                        this.rcli.operation = None;
                        this.show_error(
                            "rcli: the stopped client has not exited; check the project terminal",
                            cx,
                        );
                    });
                    return;
                }
            }
            let _ = view.update_in(cx, |this, _, cx| {
                this.rcli.operation = None;
                match then {
                    Some(line) => {
                        if this.rcli_type(&line, cx) {
                            this.rcli_notice(format!("rcli: {}", label.to_lowercase()), cx);
                        } else {
                            this.show_error("The project terminal is busy", cx);
                        }
                    }
                    None => this.rcli_notice("rcli: agent stopped", cx),
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn rcli_stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.rcli.operation.is_none() {
            self.rcli_stop_then(None, String::new(), window, cx);
        }
    }

    /// Types `line` into the idle project terminal and shows it.
    fn rcli_type(&mut self, line: &str, cx: &mut Context<Self>) -> bool {
        let Some(terminal) = self.project_terminal().cloned() else {
            return false;
        };
        let typed = terminal.update(cx, |view, cx| view.run_line(line, cx));
        if typed {
            self.terminal_visible = true;
        }
        typed
    }

    /// Interactive commands such as login or a VPS shell use the project terminal
    /// when it is idle and the quick terminal beside a running agent otherwise.
    pub(crate) fn rcli_interactive(
        &mut self,
        line: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.rcli_type(&line, cx) {
            return;
        }
        let Some(terminal) = self.project_terminal().cloned() else {
            return;
        };
        if !terminal.update(cx, |view, cx| view.run_in_quick_terminal(&line, window, cx)) {
            self.show_error("Both project terminals are busy", cx);
        }
    }

    /// Runs a non-interactive rcli command in the background and reports it.
    pub(crate) fn rcli_background<T: Send + 'static>(
        &mut self,
        label: &str,
        run: impl FnOnce(Rcli) -> anyhow::Result<T> + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(rcli) = self.rcli() else {
            return;
        };
        if self.rcli.operation.is_some() {
            return;
        }
        self.rcli.operation = Some(label.to_owned());
        self.rcli_notice(format!("rcli: {}", label.to_lowercase()), cx);
        let task = cx.background_executor().spawn(async move { run(rcli) });
        cx.spawn(async move |view, cx| {
            let result = task.await;
            let _ = view.update(cx, |this, cx| {
                this.rcli.operation = None;
                match result {
                    Ok(value) => done(this, value, cx),
                    Err(error) => this.show_error(format!("rcli: {error:#}"), cx),
                }
                this.rcli_poll(cx);
            });
        })
        .detach();
    }

    pub(crate) fn rcli_sync(&mut self, prefer: Option<&'static str>, cx: &mut Context<Self>) {
        self.rcli_background(
            "Syncing…",
            move |rcli| rcli.sync(prefer),
            |this, stats, cx| {
                this.rcli_notice(
                    format!(
                        "rcli: synced · {} up · {} down",
                        stats.uploaded, stats.downloaded
                    ),
                    cx,
                );
                this.refresh(cx);
            },
            cx,
        );
    }

    pub(crate) fn rcli_operation(&self) -> Option<&str> {
        self.rcli.operation.as_deref()
    }

    pub(crate) fn rcli_open_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rcli_open_at(crate::rcli_view::Section::Overview, window, cx);
    }

    pub(crate) fn rcli_open_at(
        &mut self,
        section: crate::rcli_view::Section,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.rcli() {
            Some(rcli) => {
                crate::rcli_view::open(cx.entity().downgrade(), rcli, section, window, cx);
            }
            None if self.rcli.binary.is_none() => crate::settings_view::open_rcli(window, cx),
            None => {}
        }
    }

    /// The title bar's rcli menu, next to Settings.
    pub(super) fn rcli_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme::current();
        let p = &theme.colors;
        let view = cx.entity().downgrade();
        let found = self.rcli.binary.is_some();
        let project = self.repo.is_some();
        let running = self.rcli_running(cx);
        let operation = self.rcli.operation.clone();
        let (dot, label) = match running {
            Running::None => (p.muted, "rcli".to_owned()),
            Running::Here(agent) => (
                p.green,
                format!("rcli · {}", agent.map_or("agent", Agent::label)),
            ),
            Running::Elsewhere(agent) => (
                p.yellow,
                format!("rcli · {}", agent.map_or("agent", Agent::label)),
            ),
        };
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(div().w(px(6.)).h(px(6.)).rounded_full().bg(rgb(if found {
                dot
            } else {
                p.red
            })))
            .child(
                Button::new("rcli-menu")
                    .ghost()
                    .xsmall()
                    .label(operation.clone().unwrap_or(label))
                    .icon(gpui_component::Icon::empty().path("icons/chevron-down.svg"))
                    .tooltip(if found {
                        "Remote agents and sync"
                    } else {
                        "rcli was not found"
                    })
                    .disabled(self.opening)
                    .dropdown_menu_with_anchor(Corner::TopRight, move |menu, _, _| {
                        let act = |f: fn(&mut Give, &mut Window, &mut Context<Give>)| {
                            let view = view.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                let _ = view.update(cx, |this, cx| f(this, window, cx));
                            }
                        };
                        let mut menu = menu.min_w(px(240.));
                        if !found {
                            return menu.label("rcli was not found").item(
                                PopupMenuItem::new("Choose rcli binary…").on_click(
                                    |_, window, cx| crate::settings_view::open_rcli(window, cx),
                                ),
                            );
                        }
                        if !project {
                            return menu.label("Open a project to use rcli").item(
                                PopupMenuItem::new("rcli settings…").on_click(|_, window, cx| {
                                    crate::settings_view::open_rcli(window, cx)
                                }),
                            );
                        }
                        let busy = operation.is_some();
                        menu = menu.label(match (&operation, running) {
                            (Some(operation), _) => operation.clone(),
                            (None, Running::None) => "No agent running".into(),
                            (None, Running::Here(agent)) => format!(
                                "{} running in this terminal",
                                agent.map_or("Agent", Agent::label)
                            ),
                            (None, Running::Elsewhere(agent)) => format!(
                                "{} running in another client",
                                agent.map_or("Agent", Agent::label)
                            ),
                        });
                        match running.agent() {
                            None if running == Running::None => {
                                menu = menu
                                    .item(
                                        PopupMenuItem::new("Start Claude").disabled(busy).on_click(
                                            act(|this, window, cx| {
                                                this.rcli_start(Agent::Claude, window, cx)
                                            }),
                                        ),
                                    )
                                    .item(
                                        PopupMenuItem::new("Start Codex").disabled(busy).on_click(
                                            act(|this, window, cx| {
                                                this.rcli_start(Agent::Codex, window, cx)
                                            }),
                                        ),
                                    );
                            }
                            current => {
                                for agent in Agent::ALL {
                                    if Some(agent) == current {
                                        continue;
                                    }
                                    let view = view.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(format!("Switch to {}", agent.label()))
                                            .disabled(busy)
                                            .on_click(move |_, window, cx| {
                                                let _ = view.update(cx, |this, cx| {
                                                    this.rcli_start(agent, window, cx)
                                                });
                                            }),
                                    );
                                }
                                if matches!(running, Running::Elsewhere(_)) {
                                    menu = menu.item(
                                        PopupMenuItem::new("Take over in this window")
                                            .disabled(busy)
                                            .on_click(act(Give::rcli_takeover)),
                                    );
                                }
                                menu = menu.item(
                                    PopupMenuItem::new("Stop agent")
                                        .disabled(busy)
                                        .on_click(act(Give::rcli_stop)),
                                );
                            }
                        }
                        menu.separator()
                            .item(
                                PopupMenuItem::new("Sync now")
                                    .disabled(busy)
                                    .on_click(act(|this, _, cx| this.rcli_sync(None, cx))),
                            )
                            .item(PopupMenuItem::new("Resume a conversation…").on_click(act(
                                |this, window, cx| {
                                    this.rcli_open_at(
                                        crate::rcli_view::Section::Conversations,
                                        window,
                                        cx,
                                    )
                                },
                            )))
                            .item(PopupMenuItem::new("Open VPS shell").on_click(act(
                                |this, window, cx| {
                                    if let Some(rcli) = this.rcli() {
                                        this.rcli_interactive(rcli.shell_line(), window, cx);
                                    }
                                },
                            )))
                            .separator()
                            .item(
                                PopupMenuItem::new("rcli panel…")
                                    .on_click(act(Give::rcli_open_panel)),
                            )
                            .item(
                                PopupMenuItem::new("rcli settings…").on_click(|_, window, cx| {
                                    crate::settings_view::open_rcli(window, cx)
                                }),
                            )
                    }),
            )
    }
}
