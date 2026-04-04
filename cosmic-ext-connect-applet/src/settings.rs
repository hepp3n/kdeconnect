#[macro_use]
extern crate cosmic_ext_connect_applet;

use cosmic::widget::button::Catalog;
use cosmic::{
    Action, Application, ApplicationExt, Element, Task,
    app::Core,
    iced::{Alignment, Length, Subscription},
    widget,
};
use cosmic_ext_connect_applet::{backend, models::Device};
use futures::StreamExt as _;
use std::collections::HashMap;

/// A desktop command stored as JSON: {id, name, command}
type LocalCommand = serde_json::Value;

fn run_commands_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(format!("{}/.config/kdeconnect/runcommand.json", home))
}

fn load_run_commands() -> Vec<LocalCommand> {
    match std::fs::read_to_string(run_commands_path()) {
        Ok(json) => serde_json::from_str::<Vec<LocalCommand>>(&json).unwrap_or_default(),
        Err(_) => vec![],
    }
}

fn save_run_commands(commands: &[LocalCommand]) {
    let path = run_commands_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(commands) {
        let _ = std::fs::write(path, json);
    }
}

// ---------------------------------------------------------------------------
// Plugin metadata
// ---------------------------------------------------------------------------

struct PluginInfo {
    id: &'static str,
    name: String,
    description: String,
    icon: &'static str,
}

fn implemented_plugins() -> Vec<PluginInfo> {
    vec![
    PluginInfo {
        id: "battery",
        name: fl!("plugin-battery-name"),
        description: fl!("plugin-battery-desc"),
        icon: "battery-full-symbolic",
    },
    PluginInfo {
        id: "clipboard",
        name: fl!("plugin-clipboard-name"),
        description: fl!("plugin-clipboard-desc"),
        icon: "edit-paste-symbolic",
    },
    PluginInfo {
        id: "connectivity_report",
        name: fl!("plugin-connectivity-name"),
        description: fl!("plugin-connectivity-desc"),
        icon: "network-cellular-symbolic",
    },
    PluginInfo {
        id: "contacts",
        name: fl!("plugin-contacts-name"),
        description: fl!("plugin-contacts-desc"),
        icon: "x-office-address-book-symbolic",
    },
    PluginInfo {
        id: "findmyphone",
        name: fl!("plugin-findmyphone-name"),
        description: fl!("plugin-findmyphone-desc"),
        icon: "audio-speakers-symbolic",
    },
    PluginInfo {
        id: "mpris",
        name: fl!("plugin-mpris-name"),
        description: fl!("plugin-mpris-desc"),
        icon: "media-playback-start-symbolic",
    },
    PluginInfo {
        id: "notification",
        name: fl!("plugin-notifications-name"),
        description: fl!("plugin-notifications-desc"),
        icon: "preferences-system-notifications-symbolic",
    },
    PluginInfo {
        id: "ping",
        name: fl!("plugin-ping-name"),
        description: fl!("plugin-ping-desc"),
        icon: "network-transmit-receive-symbolic",
    },
    PluginInfo {
        id: "runcommand",
        name: fl!("plugin-runcommand-name"),
        description: fl!("plugin-runcommand-desc"),
        icon: "utilities-terminal-symbolic",
    },
    PluginInfo {
        id: "share",
        name: fl!("plugin-share-name"),
        description: fl!("plugin-share-desc"),
        icon: "document-send-symbolic",
    },
    PluginInfo {
        id: "sms",
        name: fl!("plugin-sms-name"),
        description: fl!("plugin-sms-desc"),
        icon: "mail-message-new-symbolic",
    },
    ]
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tab {
    PairedDevices,
    AvailableDevices,
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Message {
    DevicesLoaded(Vec<Device>),
    /// Fired when persisted plugin states are read back for a device.
    /// Payload is (device_id, list-of-disabled-plugin-ids).
    PluginStatesLoaded(String, Vec<String>),
    SelectTab(Tab),
    SelectDevice(String),
    TogglePlugin(String, bool),
    Refresh,
    PairDevice(String),
    UnpairDevice(String),
    /// Fired by the D-Bus event subscription whenever a device connects or pairs.
    ServiceEvent(kdeconnect_dbus_client::ServiceEvent),
    // Run Command management
    RunCommandsLoaded(Vec<LocalCommand>),
    NewRunCommandName(String),
    NewRunCommandCommand(String),
    AddRunCommand,
    DeleteRunCommand(String),
}

// ---------------------------------------------------------------------------
// Application model
// ---------------------------------------------------------------------------

pub struct SettingsApp {
    core: Core,
    active_tab: Tab,
    devices: Vec<Device>,
    selected_device: Option<String>,
    /// device_id → (plugin_id → enabled)
    plugin_states: HashMap<String, HashMap<String, bool>>,
    pairing_in_progress: HashMap<String, bool>,
    /// Desktop commands manageable from the Run Command section
    run_commands: Vec<LocalCommand>,
    new_cmd_name: String,
    new_cmd_command: String,
}

impl SettingsApp {
    fn plugin_enabled(&self, plugin_id: &str) -> bool {
        self.selected_device
            .as_ref()
            .and_then(|did| self.plugin_states.get(did))
            .and_then(|map| map.get(plugin_id))
            .copied()
            .unwrap_or(true)
    }

    fn default_plugin_map() -> HashMap<String, bool> {
        implemented_plugins()
            .iter()
            .map(|p| (p.id.to_string(), true))
            .collect()
    }

    fn load_plugin_states_task(device_id: String) -> Task<Action<Message>> {
        Task::perform(
            async move {
                let disabled = backend::get_disabled_plugins(device_id.clone()).await;
                (device_id, disabled)
            },
            |(did, disabled)| Action::App(Message::PluginStatesLoaded(did, disabled)),
        )
    }
}

impl Application for SettingsApp {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = "io.github.M4L-C0ntent.kdeconnect.settings";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Action<Self::Message>>) {
        let mut app = Self {
            core,
            active_tab: Tab::PairedDevices,
            devices: Vec::new(),
            selected_device: None,
            plugin_states: HashMap::new(),
            pairing_in_progress: HashMap::new(),
            run_commands: Vec::new(),
            new_cmd_name: String::new(),
            new_cmd_command: String::new(),
        };

        let title_task = app.set_window_title(
            fl!("settings-title").to_string(),
            app.core.main_window_id().unwrap(),
        );

        let load_task = Task::perform(
            async {
                if let Err(e) = backend::initialize().await {
                    eprintln!("[settings] backend init failed: {:?}", e);
                }
                backend::fetch_devices().await
            },
            |devices| Action::App(Message::DevicesLoaded(devices)),
        );

        let cmds_task = Task::perform(
            async { load_run_commands() },
            |cmds| Action::App(Message::RunCommandsLoaded(cmds)),
        );

        (app, Task::batch(vec![title_task, load_task, cmds_task]))
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        // Subscribe to D-Bus service events so the UI updates immediately when
        // a device connects, pairs, or disconnects — no polling needed.
        Subscription::run(|| {
            async_stream::stream! {
                let mut stream = backend::event_stream().await;
                while let Some(event) = stream.next().await {
                    yield Message::ServiceEvent(event);
                }
            }
        })
    }

    fn update(&mut self, message: Self::Message) -> Task<Action<Self::Message>> {
        match message {
            Message::DevicesLoaded(devices) => {
                if let Some(sel) = self.selected_device.clone() {
                    // Clear selection if the device is gone or no longer paired.
                    let still_paired = devices.iter().any(|d| d.id == sel && d.is_paired);
                    if !still_paired {
                        self.plugin_states.remove(&sel);
                        self.selected_device = None;
                    }
                }
                let prev = self.selected_device.clone();
                if self.selected_device.is_none() {
                    self.selected_device =
                        devices.iter().find(|d| d.is_paired).map(|d| d.id.clone());
                }
                for d in &devices {
                    if d.is_paired {
                        self.pairing_in_progress.remove(&d.id);
                    }
                }
                self.devices = devices;

                // Load plugin states if we auto-selected a new device
                if self.selected_device != prev {
                    if let Some(did) = self.selected_device.clone() {
                        return Self::load_plugin_states_task(did);
                    }
                }
            }

            Message::PluginStatesLoaded(device_id, disabled) => {
                let mut map = Self::default_plugin_map();
                for pid in disabled {
                    map.insert(pid, false);
                }
                self.plugin_states.insert(device_id, map);
            }

            Message::SelectTab(tab) => {
                let trigger_broadcast = tab == Tab::AvailableDevices;
                self.active_tab = tab;
                if trigger_broadcast {
                    return Task::perform(
                        async { backend::fetch_devices().await },
                        |devices| Action::App(Message::DevicesLoaded(devices)),
                    );
                }
            }

            Message::SelectDevice(id) => {
                let changed = self.selected_device.as_deref() != Some(&id);
                self.selected_device = Some(id.clone());
                if changed {
                    // Show defaults immediately, then load persisted state
                    self.plugin_states
                        .entry(id.clone())
                        .or_insert_with(Self::default_plugin_map);
                    return Self::load_plugin_states_task(id);
                }
            }

            Message::TogglePlugin(plugin_id, enabled) => {
                if let Some(ref device_id) = self.selected_device.clone() {
                    self.plugin_states
                        .entry(device_id.clone())
                        .or_insert_with(Self::default_plugin_map)
                        .insert(plugin_id.clone(), enabled);

                    let did = device_id.clone();
                    let pid = plugin_id;
                    return Task::perform(
                        async move {
                            if let Err(e) =
                                backend::set_plugin_enabled(did, pid, enabled).await
                            {
                                eprintln!("[settings] set_plugin_enabled failed: {:?}", e);
                            }
                        },
                        |_| Action::App(Message::Refresh),
                    );
                }
            }

            Message::Refresh => {
                return Task::perform(
                    async { backend::fetch_devices().await },
                    |devices| Action::App(Message::DevicesLoaded(devices)),
                );
            }

            Message::PairDevice(device_id) => {
                self.pairing_in_progress.insert(device_id.clone(), true);
                let id = device_id;
                return Task::perform(
                    async move {
                        backend::pair_device(id).await.ok();
                        backend::fetch_devices().await
                    },
                    |devices| Action::App(Message::DevicesLoaded(devices)),
                );
            }

            Message::UnpairDevice(device_id) => {
                if self.selected_device.as_deref() == Some(&device_id) {
                    self.selected_device = None;
                }
                self.plugin_states.remove(&device_id);
                let id = device_id;
                return Task::perform(
                    async move {
                        backend::unpair_device(id).await.ok();
                        backend::fetch_devices().await
                    },
                    |devices| Action::App(Message::DevicesLoaded(devices)),
                );
            }

            // A D-Bus service event arrived — refresh device list immediately
            // so paired/connected state changes appear without waiting for polling.
            Message::ServiceEvent(event) => {
                match &event {
                    kdeconnect_dbus_client::ServiceEvent::DeviceDisconnected(id) => {
                        if self.selected_device.as_deref() == Some(id.as_str()) {
                            // Keep selection — device is just offline, not unpaired.
                        }
                    }
                    kdeconnect_dbus_client::ServiceEvent::DevicePaired(id, device) => {
                        if !device.is_paired
                            && self.selected_device.as_deref() == Some(id.as_str())
                        {
                            self.selected_device = None;
                            self.plugin_states.remove(id);
                        }
                    }
                    _ => {}
                }
                return Task::perform(
                    async { backend::fetch_devices().await },
                    |devices| Action::App(Message::DevicesLoaded(devices)),
                );
            }
            Message::RunCommandsLoaded(cmds) => {
                self.run_commands = cmds;
            }
            Message::NewRunCommandName(s) => {
                self.new_cmd_name = s;
            }
            Message::NewRunCommandCommand(s) => {
                self.new_cmd_command = s;
            }
            Message::AddRunCommand => {
                let name = self.new_cmd_name.trim().to_string();
                let cmd = self.new_cmd_command.trim().to_string();
                if !name.is_empty() && !cmd.is_empty() {
                    self.run_commands.push(serde_json::json!({
                        "id": uuid_v4(),
                        "name": name,
                        "command": cmd,
                    }));
                    self.new_cmd_name.clear();
                    self.new_cmd_command.clear();
                    save_run_commands(&self.run_commands);
                }
            }
            Message::DeleteRunCommand(id) => {
                self.run_commands.retain(|c| c["id"].as_str() != Some(&id));
                save_run_commands(&self.run_commands);
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let spacing = cosmic::theme::active().cosmic().spacing;

        let content: Element<'_, Message> = match self.active_tab {
            Tab::PairedDevices => widget::row()
                .push(self.view_paired_sidebar(&spacing))
                .push(widget::divider::vertical::default())
                .push(self.view_plugin_panel(&spacing))
                .height(Length::Fill)
                .into(),
            Tab::AvailableDevices => self.view_available_devices(&spacing),
        };

        widget::column()
            .push(self.view_tab_bar(&spacing))
            .push(widget::divider::horizontal::default())
            .push(content)
            .height(Length::Fill)
            .into()
    }
}

// ---------------------------------------------------------------------------
// View helpers
// ---------------------------------------------------------------------------

impl SettingsApp {
    fn view_tab_bar<'a>(
        &'a self,
        spacing: &cosmic::cosmic_theme::Spacing,
    ) -> Element<'a, Message> {
        let paired_btn = if self.active_tab == Tab::PairedDevices {
            widget::button::standard(fl!("settings-tab-paired"))
                .on_press(Message::SelectTab(Tab::PairedDevices))
        } else {
            widget::button::text(fl!("settings-tab-paired"))
                .on_press(Message::SelectTab(Tab::PairedDevices))
        };

        let available_btn = if self.active_tab == Tab::AvailableDevices {
            widget::button::standard(fl!("settings-tab-available"))
                .on_press(Message::SelectTab(Tab::AvailableDevices))
        } else {
            widget::button::text(fl!("settings-tab-available"))
                .on_press(Message::SelectTab(Tab::AvailableDevices))
        };

        widget::row()
            .spacing(spacing.space_xs)
            .padding([spacing.space_xs, spacing.space_m])
            .align_y(Alignment::Center)
            .push(paired_btn)
            .push(available_btn)
            .push(widget::Space::new().width(Length::Fill))
            .into()
    }

    fn view_paired_sidebar<'a>(
        &'a self,
        spacing: &cosmic::cosmic_theme::Spacing,
    ) -> Element<'a, Message> {
        let paired: Vec<&Device> = self.devices.iter().filter(|d| d.is_paired).collect();

        let mut col = widget::column()
            .spacing(spacing.space_xxs)
            .padding(spacing.space_s)
            .width(Length::Fixed(220.0));

        col = col.push(
            widget::text(fl!("paired-devices-header"))
                .size(13)
                .font(cosmic::font::bold()),
        );
        col = col.push(widget::divider::horizontal::default());

        if paired.is_empty() {
            col = col.push(
                widget::container(
                    widget::text(fl!("paired-devices-none"))
                        .size(12),
                )
                .padding(spacing.space_s),
            );
        } else {
            for device in paired {
                let device_id = device.id.clone();
                let is_selected = self.selected_device.as_deref() == Some(&device.id);
                let status_icon = if device.is_reachable {
                    "network-wireless-symbolic"
                } else {
                    "network-offline-symbolic"
                };

                let item = widget::row()
                    .spacing(spacing.space_s)
                    .align_y(Alignment::Center)
                    .push(widget::icon::from_name(device.device_icon()).size(20))
                    .push(
                        widget::column()
                            .spacing(2)
                            .push(widget::text(&device.name).size(13))
                            .push(
                                widget::text(if device.is_reachable {
                                    fl!("paired-devices-connected")
                                } else {
                                    fl!("paired-devices-offline")
                                })
                                .size(11),
                            )
                            .width(Length::Fill),
                    )
                    .push(widget::icon::from_name(status_icon).size(14));

                let btn = widget::button::custom(item)
                    .class(cosmic::theme::Button::Custom {
                        active: Box::new(|focused, theme| {
                            let mut s = theme.active(focused, false, &cosmic::theme::Button::Text);
                            s.border_radius = cosmic::iced_core::border::Radius::from(0.0);
                            s
                        }),
                        hovered: Box::new(|focused, theme| {
                            let mut s = theme.hovered(focused, false, &cosmic::theme::Button::Text);
                            s.border_radius = cosmic::iced_core::border::Radius::from(0.0);
                            s
                        }),
                        disabled: Box::new(|theme| {
                            let mut s = theme.disabled(&cosmic::theme::Button::Text);
                            s.border_radius = cosmic::iced_core::border::Radius::from(0.0);
                            s
                        }),
                        pressed: Box::new(|focused, theme| {
                            let mut s = theme.pressed(focused, false, &cosmic::theme::Button::Text);
                            s.border_radius = cosmic::iced_core::border::Radius::from(0.0);
                            s
                        }),
                    })
                    .on_press(Message::SelectDevice(device_id))
                    .width(Length::Fill);

                if is_selected {
                    col = col.push(
                        widget::container(btn)
                            .class(cosmic::theme::Container::Primary)
                            .width(Length::Fill),
                    );
                } else {
                    col = col.push(btn);
                }
            }
        }

        widget::scrollable(col).height(Length::Fill).into()
    }

    fn view_plugin_panel<'a>(
        &'a self,
        spacing: &cosmic::cosmic_theme::Spacing,
    ) -> Element<'a, Message> {
        let mut col = widget::column()
            .spacing(spacing.space_s)
            .padding(spacing.space_m)
            .width(Length::Fill);

        if let Some(ref device_id) = self.selected_device {
            if let Some(device) = self.devices.iter().find(|d| &d.id == device_id) {
                let unpair_id = device_id.clone();
                col = col.push(
                    widget::row()
                        .spacing(spacing.space_s)
                        .align_y(Alignment::Center)
                        .push(widget::icon::from_name(device.device_icon()).size(20))
                        .push(
                            widget::text(&device.name)
                                .size(15)
                                .font(cosmic::font::bold())
                                .width(Length::Fill),
                        )
                        .push(
                            widget::button::destructive(fl!("paired-devices-unpair"))
                                .on_press(Message::UnpairDevice(unpair_id)),
                        ),
                );
            }
        } else {
            col = col.push(
                widget::text(fl!("paired-plugins-header"))
                    .size(15)
                    .font(cosmic::font::bold()),
            );
        }

        col = col.push(widget::divider::horizontal::default());

        if self.selected_device.is_none() {
            col = col.push(
                widget::container(
                    widget::text(fl!("paired-plugins-hint")).size(14),
                )
                .padding(spacing.space_l),
            );
            return widget::scrollable(col).height(Length::Fill).into();
        }

        for plugin in implemented_plugins() {
            let enabled = self.plugin_enabled(plugin.id);
            let plugin_id = plugin.id.to_string();
            let is_runcommand = plugin.id == "runcommand";

            let row = widget::row()
                .spacing(spacing.space_m)
                .align_y(Alignment::Center)
                .push(
                    widget::container(widget::icon::from_name(plugin.icon).size(24))
                        .width(Length::Fixed(40.0))
                        .align_x(Alignment::Center),
                )
                .push(
                    widget::column()
                        .spacing(2)
                        .push(
                            widget::text(plugin.name)
                                .size(14)
                                .font(cosmic::font::bold()),
                        )
                        .push(widget::text(plugin.description).size(12))
                        .width(Length::Fill),
                )
                .push(
                    widget::toggler(enabled)
                        .on_toggle(move |v| Message::TogglePlugin(plugin_id.clone(), v)),
                );

            col = col.push(
                widget::container(row)
                    .padding([spacing.space_s, spacing.space_m])
                    .class(cosmic::theme::Container::Card)
                    .width(Length::Fill),
            );

            // Show command management inline below the runcommand toggle
            if is_runcommand && enabled {
                col = col.push(self.view_run_commands_section(spacing));
            }
        }

        widget::scrollable(col).height(Length::Fill).into()
    }

    fn view_available_devices<'a>(
        &'a self,
        spacing: &cosmic::cosmic_theme::Spacing,
    ) -> Element<'a, Message> {
        let available: Vec<&Device> = self
            .devices
            .iter()
            .filter(|d| !d.is_paired && d.is_reachable)
            .collect();

        let mut col = widget::column()
            .spacing(spacing.space_s)
            .padding(spacing.space_m)
            .width(Length::Fill);

        col = col.push(
            widget::row()
                .spacing(spacing.space_s)
                .align_y(Alignment::Center)
                .push(
                    widget::text(fl!("available-devices-header"))
                        .size(15)
                        .font(cosmic::font::bold())
                        .width(Length::Fill),
                )
                .push(widget::button::standard(fl!("settings-scan-again")).on_press(Message::Refresh)),
        );
        col = col.push(widget::divider::horizontal::default());
        col = col.push(
            widget::text(fl!("available-devices-hint")).size(13),
        );

        if available.is_empty() {
            col = col.push(
                widget::container(
                    widget::column()
                        .spacing(spacing.space_s)
                        .push(widget::icon::from_name("network-offline-symbolic").size(48))
                        .push(
                            widget::text(fl!("available-devices-none"))
                                .size(16)
                                .font(cosmic::font::bold()),
                        )
                        .push(
                            widget::text(fl!("available-devices-none-hint"))
                            .size(13),
                        )
                        .push(widget::button::standard(fl!("settings-scan-again")).on_press(Message::Refresh))
                        .align_x(Alignment::Center),
                )
                .padding([spacing.space_xl, spacing.space_m])
                .width(Length::Fill)
                .align_x(Alignment::Center),
            );
        } else {
            for device in available {
                let device_id = device.id.clone();
                let in_progress = *self.pairing_in_progress.get(&device.id).unwrap_or(&false);

                let card = widget::row()
                    .spacing(spacing.space_m)
                    .align_y(Alignment::Center)
                    .push(widget::icon::from_name(device.device_icon()).size(32))
                    .push(
                        widget::column()
                            .spacing(2)
                            .push(
                                widget::text(&device.name)
                                    .size(14)
                                    .font(cosmic::font::bold()),
                            )
                            .push(widget::text(&device.id).size(11))
                            .width(Length::Fill),
                    )
                    .push(if in_progress {
                        widget::button::standard(fl!("available-devices-pairing"))
                    } else {
                        widget::button::suggested(fl!("available-devices-pair"))
                            .on_press(Message::PairDevice(device_id))
                    });

                col = col.push(
                    widget::container(card)
                        .padding([spacing.space_s, spacing.space_m])
                        .class(cosmic::theme::Container::Card)
                        .width(Length::Fill),
                );
            }
        }

        widget::scrollable(col).height(Length::Fill).into()
    }

    fn view_run_commands_section<'a>(
        &'a self,
        spacing: &cosmic::cosmic_theme::Spacing,
    ) -> Element<'a, Message> {
        let mut col = widget::column()
            .spacing(spacing.space_xs)
            .padding([spacing.space_xs, spacing.space_m]);

        col = col.push(
            widget::text(fl!("run-commands-manage-header"))
                .size(13)
                .font(cosmic::font::bold()),
        );

        // Existing commands
        for cmd in &self.run_commands {
            let name = cmd["name"].as_str().unwrap_or("");
            let command = cmd["command"].as_str().unwrap_or("");
            let delete_id = cmd["id"].as_str().unwrap_or("").to_string();
            let row = widget::row()
                .spacing(spacing.space_s)
                .align_y(Alignment::Center)
                .push(
                    widget::column()
                        .push(widget::text(name).size(13).font(cosmic::font::bold()))
                        .push(widget::text(command).size(11))
                        .width(Length::Fill),
                )
                .push(
                    widget::button::destructive(fl!("run-commands-delete"))
                        .on_press(Message::DeleteRunCommand(delete_id)),
                );
            col = col.push(
                widget::container(row)
                    .padding([spacing.space_xs, spacing.space_s])
                    .class(cosmic::theme::Container::Background)
                    .width(Length::Fill),
            );
        }

        // Add new command
        col = col.push(
            widget::text(fl!("run-commands-add-header"))
                .size(12)
                .font(cosmic::font::bold()),
        );
        col = col.push(
            widget::text_input(fl!("run-commands-name-placeholder"), &self.new_cmd_name)
                .on_input(Message::NewRunCommandName)
                .width(Length::Fill),
        );
        col = col.push(
            widget::text_input(fl!("run-commands-command-placeholder"), &self.new_cmd_command)
                .on_input(Message::NewRunCommandCommand)
                .width(Length::Fill),
        );
        col = col.push(
            widget::button::suggested(fl!("run-commands-add-button"))
                .on_press(Message::AddRunCommand),
        );

        widget::container(col)
            .padding([spacing.space_xs, spacing.space_m])
            .class(cosmic::theme::Container::Background)
            .width(Length::Fill)
            .into()
    }
}

/// Simple UUID v4 generator (no external crate needed).
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        t,
        t >> 16,
        t & 0xfff,
        0x8000 | (t & 0x3fff),
        t as u64 * 0xdeadbeef,
    )
}

fn main() -> cosmic::iced::Result {
    let settings = cosmic::app::Settings::default()
        .size(cosmic::iced::Size::new(740.0, 540.0));
    cosmic::app::run::<SettingsApp>(settings, ())
}
