use cosmic::{
    Action, Application, ApplicationExt, Element, Task,
    app::Core,
    iced::{Alignment, Length, Subscription},
    widget,
};
use cosmic_ext_connect_applet::{backend, models::Device};
use futures::StreamExt as _;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Plugin metadata
// ---------------------------------------------------------------------------

struct PluginInfo {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    icon: &'static str,
}

const IMPLEMENTED_PLUGINS: &[PluginInfo] = &[
    PluginInfo {
        id: "battery",
        name: "Battery Monitor",
        description: "Display the phone's battery level and charging state in the panel.",
        icon: "battery-full-symbolic",
    },
    PluginInfo {
        id: "clipboard",
        name: "Clipboard Sync",
        description: "Automatically share clipboard content between your desktop and phone.",
        icon: "edit-paste-symbolic",
    },
    PluginInfo {
        id: "connectivity_report",
        name: "Connectivity Report",
        description: "Show mobile signal strength and network type (4G, 5G, etc.).",
        icon: "network-cellular-symbolic",
    },
    PluginInfo {
        id: "contacts",
        name: "Contacts",
        description: "Sync phone contacts so SMS messages show names instead of numbers.",
        icon: "x-office-address-book-symbolic",
    },
    PluginInfo {
        id: "findmyphone",
        name: "Find My Phone",
        description: "Ring your phone at full volume to help locate it.",
        icon: "audio-speakers-symbolic",
    },
    PluginInfo {
        id: "mpris",
        name: "Media Control",
        description: "Control media playback on your phone from the desktop.",
        icon: "media-playback-start-symbolic",
    },
    PluginInfo {
        id: "notification",
        name: "Notifications",
        description: "Receive phone notifications as desktop notifications.",
        icon: "preferences-system-notifications-symbolic",
    },
    PluginInfo {
        id: "ping",
        name: "Ping",
        description: "Send and receive pings to verify connectivity with paired devices.",
        icon: "network-transmit-receive-symbolic",
    },
    PluginInfo {
        id: "runcommand",
        name: "Run Commands",
        description: "Execute predefined commands on the desktop triggered from your phone.",
        icon: "utilities-terminal-symbolic",
    },
    PluginInfo {
        id: "share",
        name: "Share Files",
        description: "Send and receive files and URLs between devices.",
        icon: "document-send-symbolic",
    },
    PluginInfo {
        id: "sms",
        name: "SMS Messages",
        description: "Send and receive SMS text messages from your desktop.",
        icon: "mail-message-new-symbolic",
    },
];

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
        IMPLEMENTED_PLUGINS
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
        };

        let title_task = app.set_window_title(
            "KDE Connect Settings".to_string(),
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

        (app, Task::batch(vec![title_task, load_task]))
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
                // For pair-state changes, clear the selection immediately if the
                // currently selected device is the one that was unpaired — this
                // prevents the right panel from showing stale plugin state while
                // the async fetch is in flight.
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
            widget::button::standard("Paired Devices")
                .on_press(Message::SelectTab(Tab::PairedDevices))
        } else {
            widget::button::text("Paired Devices")
                .on_press(Message::SelectTab(Tab::PairedDevices))
        };

        let available_btn = if self.active_tab == Tab::AvailableDevices {
            widget::button::standard("Available Devices")
                .on_press(Message::SelectTab(Tab::AvailableDevices))
        } else {
            widget::button::text("Available Devices")
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
            widget::text("Paired Devices")
                .size(13)
                .font(cosmic::font::bold()),
        );
        col = col.push(widget::divider::horizontal::default());

        if paired.is_empty() {
            col = col.push(
                widget::container(
                    widget::text("No paired devices.\nUse the Available Devices tab to pair.")
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
                                    "Connected"
                                } else {
                                    "Offline"
                                })
                                .size(11),
                            )
                            .width(Length::Fill),
                    )
                    .push(widget::icon::from_name(status_icon).size(14));

                let btn = if is_selected {
                    widget::button::custom(item)
                        .class(cosmic::theme::Button::Suggested)
                        .on_press(Message::SelectDevice(device_id))
                        .width(Length::Fill)
                } else {
                    widget::button::custom(item)
                        .class(cosmic::theme::Button::Text)
                        .on_press(Message::SelectDevice(device_id))
                        .width(Length::Fill)
                };

                col = col.push(btn);
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
                            widget::button::destructive("Unpair")
                                .on_press(Message::UnpairDevice(unpair_id)),
                        ),
                );
            }
        } else {
            col = col.push(
                widget::text("Plugin Settings")
                    .size(15)
                    .font(cosmic::font::bold()),
            );
        }

        col = col.push(widget::divider::horizontal::default());

        if self.selected_device.is_none() {
            col = col.push(
                widget::container(
                    widget::text("Select a paired device to configure its plugins.").size(14),
                )
                .padding(spacing.space_l),
            );
            return widget::scrollable(col).height(Length::Fill).into();
        }

        for plugin in IMPLEMENTED_PLUGINS {
            let enabled = self.plugin_enabled(plugin.id);
            let plugin_id = plugin.id.to_string();

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
                    widget::text("Available Devices")
                        .size(15)
                        .font(cosmic::font::bold())
                        .width(Length::Fill),
                )
                .push(widget::button::standard("Scan Again").on_press(Message::Refresh)),
        );
        col = col.push(widget::divider::horizontal::default());
        col = col.push(
            widget::text("Devices on the local network that have not been paired yet.").size(13),
        );

        if available.is_empty() {
            col = col.push(
                widget::container(
                    widget::column()
                        .spacing(spacing.space_s)
                        .push(widget::icon::from_name("network-offline-symbolic").size(48))
                        .push(
                            widget::text("No devices found")
                                .size(16)
                                .font(cosmic::font::bold()),
                        )
                        .push(
                            widget::text(
                                "Make sure your device is on the same network and KDE Connect is open.",
                            )
                            .size(13),
                        )
                        .push(widget::button::standard("Scan Again").on_press(Message::Refresh))
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
                        widget::button::standard("Pairing\u{2026}")
                    } else {
                        widget::button::suggested("Pair")
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
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> cosmic::iced::Result {
    let settings = cosmic::app::Settings::default()
        .size(cosmic::iced::Size::new(740.0, 540.0));
    cosmic::app::run::<SettingsApp>(settings, ())
}
