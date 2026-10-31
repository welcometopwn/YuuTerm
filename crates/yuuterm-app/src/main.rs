#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod store;
mod terminal_ui;

use eframe::egui::{self, Color32, RichText};
use std::{collections::BTreeMap, sync::Arc};
use yuuterm_core::{Profile, Protocol, SshAuth};
use yuuterm_ssh::{Command, Engine, LiveSession};

struct SessionUi {
    live: LiveSession,
    render: terminal_ui::RenderState,
    path_input: String,
    applied_path: String,
}

struct YuuTerm {
    prefs: store::Preferences,
    filter: String,
    draft: Profile,
    editor: bool,
    settings: bool,
    files: bool,
    panes: usize,
    tabs: Vec<u64>,
    active: Option<u64>,
    notice: String,
    load_failed: bool,
    engine: Result<Engine, String>,
    live: BTreeMap<u64, SessionUi>,
    pane_profiles: [Option<u64>; 4],
    focused_pane: usize,
    pending_paste: Option<terminal_ui::PendingPaste>,
    #[cfg(feature = "capture")]
    capture_requested: bool,
    #[cfg(feature = "capture")]
    capture_fixture: bool,
}

impl YuuTerm {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (prefs, notice, load_failed) = match store::load() {
            Ok(p) => (p, "Ready".into(), false),
            Err(e) => (
                store::Preferences::fresh(),
                format!("Settings were not loaded: {e} — original file preserved."),
                true,
            ),
        };
        let app = Self {
            prefs,
            notice,
            load_failed,
            filter: String::new(),
            draft: Profile::default(),
            editor: false,
            settings: false,
            files: false,
            panes: if cfg!(feature = "capture") {
                std::env::var("YUUTERM_CAPTURE_PANES")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .filter(|n| [1, 2, 4].contains(n))
                    .unwrap_or(1)
            } else {
                1
            },
            tabs: Vec::new(),
            active: None,
            engine: store::path().map_err(|e| e.to_string()).and_then(|p| {
                Engine::new(p.with_file_name("host-keys.json")).map_err(|e| e.to_string())
            }),
            live: BTreeMap::new(),
            pane_profiles: [None; 4],
            focused_pane: 0,
            pending_paste: None,
            #[cfg(feature = "capture")]
            capture_requested: false,
            #[cfg(feature = "capture")]
            capture_fixture: false,
        };
        #[cfg(feature = "capture")]
        let app = app.with_capture_fixture(&cc.egui_ctx);
        app.apply_theme(&cc.egui_ctx);
        app
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        let mut v = egui::Visuals::dark();
        v.panel_fill = if self.prefs.mocha {
            Color32::from_rgb(30, 30, 46)
        } else {
            Color32::from_rgb(23, 24, 27)
        };
        v.window_fill = v.panel_fill;
        v.extreme_bg_color = if self.prefs.mocha {
            Color32::from_rgb(24, 24, 37)
        } else {
            Color32::from_rgb(16, 17, 19)
        };
        v.override_text_color = Some(if self.prefs.mocha {
            Color32::from_rgb(205, 214, 244)
        } else {
            Color32::from_rgb(218, 220, 225)
        });
        v.selection.bg_fill = if self.prefs.mocha {
            Color32::from_rgb(88, 75, 125)
        } else {
            Color32::from_rgb(49, 71, 85)
        };
        ctx.set_visuals(v);
        ctx.global_style_mut(|style| {
            for (text_style, size) in [
                (egui::TextStyle::Small, 11.0),
                (egui::TextStyle::Body, 14.0),
                (egui::TextStyle::Button, 14.0),
            ] {
                if let Some(font) = style.text_styles.get_mut(&text_style) {
                    font.size = size;
                }
            }
        });
    }

    #[cfg(feature = "capture")]
    fn with_capture_fixture(mut self, ctx: &egui::Context) -> Self {
        if let Ok(path) = std::env::var("YUUTERM_CAPTURE_PROFILE") {
            let load = || -> Result<Profile, String> {
                use std::io::Read;
                let f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                let mut bytes = Vec::new();
                f.take(1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > 1024 * 1024 {
                    return Err("Capture profile is oversized".into());
                }
                let p: Profile = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if p.host != "127.0.0.1" || p.username != "fixture" || p.id != 1 {
                    return Err("Capture mode accepts only the isolated loopback fixture".into());
                }
                Ok(p)
            };
            match load() {
                Ok(profile) => {
                    let trust = std::path::Path::new(&path).with_file_name("host-keys.json");
                    self.engine = Engine::new(trust).map_err(|e| e.to_string());
                    self.prefs = store::Preferences::fresh();
                    self.prefs.profiles.push(profile.clone());
                    self.load_failed = true;
                    self.capture_fixture = true;
                    self.files = true;
                    self.select_profile(profile.id);
                    self.connect(profile, ctx);
                    self.notice =
                        "Local SSH/SFTP protocol fixture · Capture only · Settings saving disabled"
                            .into();
                }
                Err(e) => self.notice = e,
            }
        }
        self
    }

    fn persist(&mut self) {
        if self.load_failed {
            self.notice = "Saving is blocked because the original settings could not be loaded. Repair or back up that file first.".into();
            return;
        }
        self.notice = match store::save(&self.prefs) {
            Ok(()) => "Saved locally · Connection metadata only".into(),
            Err(e) => format!("Could not save settings: {e}"),
        };
    }

    fn new_profile(&mut self) {
        self.draft = Profile::default();
        self.editor = true;
    }

    fn editor(&mut self, ctx: &egui::Context) {
        let mut open = self.editor;
        egui::Window::new("Session settings").open(&mut open).resizable(false).default_width(400.0).show(ctx, |ui| {
            ui.label("Name"); ui.text_edit_singleline(&mut self.draft.name);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for protocol in Protocol::ALL {
                    if ui.selectable_label(self.draft.protocol == protocol, protocol.label()).clicked() {
                        self.draft.protocol = protocol;
                        self.draft.port = protocol.default_port();
                    }
                }
            });
            ui.add_space(8.0);
            ui.label(if self.draft.protocol == Protocol::Serial { "COM device" } else { "Host / IP address" });
            ui.text_edit_singleline(&mut self.draft.host);
            if self.draft.protocol != Protocol::Serial {
                ui.horizontal(|ui| { ui.label("Port"); ui.add(egui::DragValue::new(&mut self.draft.port).range(1..=65535)); });
                ui.label("Username"); ui.text_edit_singleline(&mut self.draft.username);
            }
            if self.draft.protocol == Protocol::Telnet { ui.colored_label(Color32::from_rgb(240,190,110), "TELNET sends data without encryption."); }
            if self.draft.protocol == Protocol::Ssh {
                ui.add_space(8.0);
                ui.label("Authentication");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.draft.ssh.auth,SshAuth::WindowsAgent,"Windows SSH agent");
                    ui.selectable_value(&mut self.draft.ssh.auth,SshAuth::KeyFile,"Private-key file");
                });
                if self.draft.ssh.auth == SshAuth::WindowsAgent {
                    ui.label("Identity fingerprint (optional SHA256:…)");
                    ui.text_edit_singleline(&mut self.draft.ssh.identity_fingerprint);
                    ui.collapsing("Agent connection", |ui| { ui.text_edit_singleline(&mut self.draft.ssh.agent_pipe); });
                } else {
                    ui.label("Private-key path"); ui.text_edit_singleline(&mut self.draft.ssh.key_file);
                    ui.small("Encrypted keys: load into the Windows agent for now.");
                }
            }
            ui.collapsing("Terminal appearance", |ui| {
                let mut override_font = self.draft.font_size.is_some();
                if ui.checkbox(&mut override_font, "Override global font size").changed() {
                    self.draft.font_size = override_font.then_some(self.prefs.font_size);
                }
                if let Some(size) = &mut self.draft.font_size { ui.add(egui::Slider::new(size, 8.0..=32.0).text("Font size")); }
                ui.small(format!("Effective: {} pt", self.draft.effective_font_size(self.prefs.font_size)));
            });
            ui.add_space(12.0);
            ui.weak("SSH agent/key authentication is available. Passwords and other protocols are still under development.");
            let validation = self.draft.validate();
            if let Err(e) = validation { ui.small(e); }
            if ui.add_enabled(validation.is_ok() && !self.load_failed, egui::Button::new("Save session")).clicked() {
                if self.draft.id == 0 {
                    self.draft.id = self.prefs.profiles.iter().map(|p| p.id).max().unwrap_or(0).saturating_add(1);
                    self.prefs.profiles.push(self.draft.clone());
                } else if let Some(p) = self.prefs.profiles.iter_mut().find(|p| p.id == self.draft.id) { *p = self.draft.clone(); }
                self.persist();
                self.editor = false;
            }
        });
        self.editor &= open;
    }

    fn select_profile(&mut self, id: u64) {
        for (index, pane) in self.pane_profiles.iter_mut().enumerate() {
            if index != self.focused_pane && *pane == Some(id) {
                *pane = None;
            }
        }
        if !self.tabs.contains(&id) {
            self.tabs.push(id);
        }
        self.active = Some(id);
        self.pane_profiles[self.focused_pane] = Some(id);
    }

    fn connect(&mut self, profile: Profile, ctx: &egui::Context) {
        let wake_ctx = ctx.clone();
        let result = match &self.engine {
            Ok(engine) => engine
                .connect(
                    profile.clone(),
                    Arc::new(move || wake_ctx.request_repaint()),
                )
                .map_err(|e| e.to_string()),
            Err(e) => Err(e.clone()),
        };
        match result {
            Ok(live) => {
                self.live.insert(
                    profile.id,
                    SessionUi {
                        live,
                        render: Default::default(),
                        path_input: String::new(),
                        applied_path: String::new(),
                    },
                );
            }
            Err(e) => self.notice = e,
        }
    }

    fn workspace(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for id in self.tabs.clone() {
                if let Some(p) = self.prefs.profiles.iter().find(|p| p.id == id)
                    && ui
                        .selectable_label(self.active == Some(id), &p.name)
                        .clicked()
                {
                    self.select_profile(id);
                }
            }
            if !self.tabs.is_empty() && ui.small_button("Close tab").clicked() {
                if let Some(id) = self.active {
                    self.live.remove(&id);
                    for pane in &mut self.pane_profiles {
                        if *pane == Some(id) {
                            *pane = None;
                        }
                    }
                    self.tabs.retain(|p| *p != id);
                }
                self.active = None;
            }
        });
        ui.separator();
        let columns = if self.panes == 1 { 1 } else { 2 };
        let rows = if self.panes == 4 { 2 } else { 1 };
        let size = ui.available_size();
        let cell = egui::vec2(
            (size.x - 12.0) / columns as f32,
            (size.y - 12.0) / rows as f32,
        );
        let blocked = self.pending_paste.is_some()
            || self
                .live
                .values()
                .any(|s| s.live.snapshot().trust.is_some());
        for row in 0..rows {
            ui.horizontal(|ui| {
                for col in 0..columns {
                    let index = row * columns + col;
                    ui.push_id(index, |ui| {
                        egui::Frame::new()
                            .fill(ui.visuals().extreme_bg_color)
                            .inner_margin(10)
                            .corner_radius(6)
                            .show(ui, |ui| {
                                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                    ui.set_min_size(cell - egui::vec2(20.0, 20.0));
                                    ui.set_max_size(cell - egui::vec2(20.0, 20.0));
                                    let profile = self.pane_profiles[index]
                                        .and_then(|id| {
                                            self.prefs.profiles.iter().find(|p| p.id == id)
                                        })
                                        .cloned();
                                    let mut start = false;
                                    if let Some(profile) = &profile {
                                        ui.horizontal_wrapped(|ui| {
                                            if ui
                                                .selectable_label(
                                                    self.focused_pane == index,
                                                    &profile.name,
                                                )
                                                .clicked()
                                            {
                                                self.focused_pane = index;
                                                self.active = Some(profile.id);
                                            }
                                            if let Some(session) = self.live.get(&profile.id) {
                                                let snapshot = session.live.snapshot();
                                                ui.small(&snapshot.endpoint);
                                                if !snapshot.finished
                                                    && ui.small_button("Disconnect").clicked()
                                                {
                                                    session.live.disconnect();
                                                }
                                                if snapshot.finished {
                                                    start = ui.small_button("Reconnect").clicked();
                                                }
                                            } else {
                                                start = ui
                                                    .add_enabled(
                                                        profile.protocol == Protocol::Ssh,
                                                        egui::Button::new("Connect"),
                                                    )
                                                    .clicked();
                                            }
                                        });
                                        if start {
                                            self.connect(profile.clone(), ui.ctx());
                                        }
                                        if let Some(session) = self.live.get_mut(&profile.id) {
                                            let snapshot = session.live.snapshot();
                                            ui.label(&snapshot.status);
                                            if snapshot.connected
                                                && ui.small_button("Copy visible output").clicked()
                                            {
                                                let mut text = String::new();
                                                for row in snapshot
                                                    .terminal
                                                    .cells
                                                    .chunks(snapshot.terminal.cols as usize)
                                                {
                                                    let line: String = row
                                                        .iter()
                                                        .filter(|c| !c.wide_continuation)
                                                        .map(|c| {
                                                            if c.text.is_empty() {
                                                                " "
                                                            } else {
                                                                c.text.as_str()
                                                            }
                                                        })
                                                        .collect();
                                                    text.push_str(line.trim_end());
                                                    text.push('\n');
                                                }
                                                ui.ctx().copy_text(text);
                                            }
                                            ui.scope(|ui| {
                                                if blocked {
                                                    ui.disable();
                                                }
                                                if let Some(paste) = terminal_ui::show(
                                                    ui,
                                                    &session.live,
                                                    profile
                                                        .effective_font_size(self.prefs.font_size),
                                                    &mut session.render,
                                                    &mut self.notice,
                                                ) {
                                                    self.pending_paste = Some(paste);
                                                }
                                                if session.render.clicked {
                                                    self.focused_pane = index;
                                                    self.active = Some(profile.id);
                                                }
                                            });
                                        } else {
                                            ui.weak(format!(
                                                "{} · {} · Not connected",
                                                profile.protocol.label(),
                                                profile.host
                                            ));
                                            if profile.protocol != Protocol::Ssh {
                                                ui.small(
                                                    "This protocol adapter is not implemented yet.",
                                                );
                                            }
                                        }
                                    } else {
                                        ui.label(RichText::new("Your workspace").size(20.0));
                                        ui.weak("Select this pane, then choose a saved session.");
                                        if ui
                                            .selectable_label(
                                                self.focused_pane == index,
                                                "Use this pane",
                                            )
                                            .clicked()
                                        {
                                            self.focused_pane = index;
                                            self.active = None;
                                        }
                                        if ui.button("+ New session").clicked() {
                                            self.new_profile();
                                        }
                                    }
                                });
                            });
                    });
                }
            });
            ui.add_space(4.0);
        }
    }

    fn file_browser(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.active else {
            ui.weak("Select an SSH session first.");
            return;
        };
        let Some(session) = self.live.get_mut(&id) else {
            ui.weak("Connect the selected SSH session to browse files.");
            return;
        };
        let snapshot = session.live.snapshot();
        ui.label(&snapshot.endpoint);
        if snapshot.path != session.applied_path {
            session.path_input = snapshot.path.clone();
            session.applied_path = snapshot.path.clone();
        }
        let field = ui.add(
            egui::TextEdit::singleline(&mut session.path_input)
                .desired_width(f32::INFINITY)
                .hint_text("Remote path"),
        );
        let enter = (field.has_focus() || field.lost_focus())
            && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let mut browse = None;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(snapshot.connected, egui::Button::new("Go"))
                .clicked()
                || (enter && snapshot.connected)
            {
                browse = Some(session.path_input.clone());
            }
            if ui
                .add_enabled(snapshot.connected, egui::Button::new("Up"))
                .clicked()
            {
                browse = Some(format!("{}/..", snapshot.path.trim_end_matches('/')));
            }
            if ui
                .add_enabled(snapshot.connected, egui::Button::new("Refresh"))
                .clicked()
            {
                browse = Some(snapshot.path.clone());
            }
        });
        ui.small(&snapshot.files_status);
        egui::ScrollArea::vertical().show(ui, |ui| {
            for entry in snapshot.files.iter() {
                let display: String = entry
                    .name
                    .chars()
                    .map(|c| {
                        if c.is_control() {
                            c.escape_default().to_string()
                        } else {
                            c.to_string()
                        }
                    })
                    .collect();
                let label = format!(
                    "{}  {}",
                    if entry.directory {
                        "DIR"
                    } else if entry.symlink {
                        "LINK"
                    } else {
                        "FILE"
                    },
                    display
                );
                let response = ui.selectable_label(false, label);
                if response.double_clicked() && entry.directory && snapshot.connected {
                    browse = Some(format!(
                        "{}/{}",
                        snapshot.path.trim_end_matches('/'),
                        entry.name
                    ));
                }
                response.context_menu(|ui| {
                    if ui.button("Copy file path").clicked() {
                        ui.ctx().copy_text(format!(
                            "{}/{}",
                            snapshot.path.trim_end_matches('/'),
                            entry.name
                        ));
                        ui.close();
                    }
                    ui.label(format!(
                        "Size: {}",
                        entry
                            .size
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "unknown".into())
                    ));
                    ui.label(format!(
                        "Mode: {}",
                        entry
                            .permissions
                            .map(|p| format!("{:04o}", p & 0o7777))
                            .unwrap_or_else(|| "unknown".into())
                    ));
                    ui.weak("Read-only preview; file changes are not implemented yet.");
                });
            }
        });
        if let Some(path) = browse
            && let Err(e) = session.live.send(Command::Browse(path))
        {
            self.notice = e.into();
        }
    }

    fn connection_dialogs(&mut self, ctx: &egui::Context) {
        for (id, session) in &self.live {
            if let Some(trust) = session.live.snapshot().trust {
                egui::Window::new("Verify SSH host key")
                    .id(egui::Id::unique(("trust", id)))
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label(&trust.host);
                        ui.label(&trust.algorithm);
                        ui.monospace(&trust.fingerprint);
                        ui.label(
                            "Compare this fingerprint with a trusted source before approving.",
                        );
                        ui.small(
                            "Saved in YuuTerm's separate host-key store. Changed keys are blocked.",
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Trust and save").clicked() {
                                session.live.trust(true);
                            }
                            if ui.button("Reject").clicked() {
                                session.live.trust(false);
                            }
                        });
                    });
            }
        }
        if let Some(paste) = &self.pending_paste {
            let mut choice = None;
            egui::Window::new("Confirm terminal paste").collapsible(false).resizable(true).show(ctx,|ui| {
                ui.label("This paste contains line breaks or control characters and may execute commands.");
                egui::ScrollArea::vertical().max_height(240.0).show(ui,|ui| { ui.monospace(paste.text.escape_debug().to_string()); });
                ui.horizontal(|ui| { if ui.button("Paste to original session").clicked(){choice=Some(true);}
 if ui.button("Cancel").clicked(){choice=Some(false);} });
            });
            if let Some(send) = choice {
                let pending = self.pending_paste.take().unwrap();
                if send {
                    if let Some(session) = self
                        .live
                        .values()
                        .find(|s| s.live.id == pending.live_id && s.live.snapshot().connected)
                    {
                        if let Err(e) = terminal_ui::paste(
                            &session.live,
                            &session.live.snapshot().terminal,
                            &pending.text,
                        ) {
                            self.notice = e.into();
                        }
                    } else {
                        self.notice =
                            "Original connection is no longer active; paste was discarded.".into();
                    }
                }
            }
        }
    }
}

impl eframe::App for YuuTerm {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(5.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("YuuTerm").strong().size(19.0));
                ui.add_space(20.0);
                if ui.button("+ New session").clicked() {
                    self.new_profile();
                }
                ui.separator();
                ui.menu_button("View", |ui| {
                    ui.weak("Workspace layout");
                    for (n, label) in [
                        (1, "Single pane"),
                        (2, "2 panes · Side by side"),
                        (4, "4 panes · Grid"),
                    ] {
                        if ui.selectable_value(&mut self.panes, n, label).clicked() {
                            self.focused_pane = self.focused_pane.min(n - 1);
                            ui.close();
                        }
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Settings").clicked() {
                        self.settings = !self.settings;
                    }
                    ui.add_enabled(false, egui::Button::new("Broadcast: off"))
                        .on_hover_text("Available after terminal integration; always opt-in.");
                });
            });
            ui.add_space(5.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let connected = self
                    .live
                    .values()
                    .filter(|s| s.live.snapshot().connected)
                    .count();
                ui.small(format!("{connected} SSH connected"));
                ui.separator();
                ui.small(&self.notice);
            });
        });
        egui::Panel::left("sidebar")
            .default_size(240.0)
            .min_size(190.0)
            .max_size(400.0)
            .show(ui, |ui| {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.files, false, "Sessions");
                    ui.selectable_value(&mut self.files, true, "Files");
                });
                ui.add_space(12.0);
                if self.files {
                    self.file_browser(ui);
                } else {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text("Find a session…")
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(10.0);
                    let mut edit = None;
                    let mut selected = None;
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for p in &self.prefs.profiles {
                            if !format!("{} {}", p.name, p.host)
                                .to_lowercase()
                                .contains(&self.filter.to_lowercase())
                            {
                                continue;
                            }
                            ui.horizontal(|ui| {
                                if ui
                                    .selectable_label(
                                        self.active == Some(p.id),
                                        format!("{}   {}", p.protocol.label(), p.name),
                                    )
                                    .clicked()
                                {
                                    selected = Some(p.id);
                                }
                                if ui.small_button("Edit").clicked() {
                                    edit = Some(p.clone());
                                }
                            });
                        }
                    });
                    if let Some(id) = selected {
                        self.select_profile(id);
                    }
                    if let Some(p) = edit {
                        self.draft = p;
                        self.editor = true;
                    }
                    if self.prefs.profiles.is_empty() {
                        ui.weak("Your saved sessions will appear here.");
                    }
                }
            });
        egui::CentralPanel::default().show(ui, |ui| self.workspace(ui));
        if self.editor {
            self.editor(&ctx);
        }
        if self.settings {
            let mut open = self.settings;
            egui::Window::new("Appearance")
                .open(&mut open)
                .resizable(false)
                .show(&ctx, |ui| {
                    ui.label("App theme");
                    let mut changed = ui
                        .selectable_value(&mut self.prefs.mocha, false, "Neutral dark")
                        .changed();
                    changed |= ui
                        .selectable_value(&mut self.prefs.mocha, true, "Catppuccin Mocha")
                        .changed();
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut self.prefs.font_size, 8.0..=32.0)
                                .text("Default terminal font size"),
                        )
                        .changed();
                    ui.weak("Session font overrides take precedence over this default.");
                    if changed {
                        self.apply_theme(&ctx);
                    }
                    if ui.button("Save appearance").clicked() {
                        self.persist();
                    }
                });
            self.settings = open;
        }
        self.connection_dialogs(&ctx);
        #[cfg(feature = "capture")]
        if let Ok(path) = std::env::var("YUUTERM_CAPTURE") {
            let ready = !self.capture_fixture
                || self.live.values().any(|s| {
                    let v = s.live.snapshot();
                    v.finished
                        || (v.connected
                            && v.files.len() == 2
                            && v.terminal
                                .cells
                                .iter()
                                .map(|c| c.text.as_str())
                                .collect::<String>()
                                .contains("Ready"))
                });
            if ready && !self.capture_requested && ctx.cumulative_pass_nr() > 2 {
                self.capture_requested = true;
                let close_ctx = ctx.clone();
                ctx.request_screenshot(move |image| {
                    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                    if let Err(e) = image::save_buffer(
                        &path,
                        &bytes,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    ) {
                        eprintln!("Screenshot failed: {e}");
                    }
                    close_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                });
            }
            ctx.request_repaint();
        }
    }
}

fn main() -> eframe::Result {
    // The old prototype must not bypass the desktop's settings-writer lease.
    let _lease = yuuterm_core::config::path()
        .and_then(|path| yuuterm_core::instance::acquire(&path))
        .and_then(|lease| {
            lease.ok_or_else(|| std::io::Error::other("This YuuTerm data folder is already in use"))
        })
        .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
    eframe::run_native(
        "YuuTerm",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1200.0, 760.0])
                .with_min_inner_size([850.0, 540.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(YuuTerm::new(cc)))),
    )
}
