// SPDX-License-Identifier: GPL-3.0-only

use ashpd::desktop::file_chooser::SelectedFiles;
use cosmic::app::{Core, Task};
use cosmic::applet::column::Column;
use cosmic::applet::{column, menu_button, padded_control};
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::alignment::Vertical;
use cosmic::iced::futures::SinkExt;
use cosmic::iced::window::Id;
use cosmic::iced::{Length, Limits, Pixels, Subscription, stream};
use cosmic::iced_core::text::LineHeight;
use cosmic::iced_widget::horizontal_rule;
use cosmic::iced_winit::commands::popup::{destroy_popup, get_popup};
use cosmic::widget::{button, container, horizontal_space, icon, row, text};
use cosmic::{Application, Apply, Element, Renderer, Theme, theme};
use kdeconnect_core::KdeConnectCore;
use kdeconnect_core::device::{Device, DeviceId, DeviceState, PairState};
use kdeconnect_core::event::{AppEvent, ConnectionEvent};
use percent_encoding::percent_decode;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::debug;

use crate::config::ConnectConfig;
use crate::{APP_ID, fl};

#[derive(Debug, Clone, Default)]
pub struct State {
    pub connectivity: Option<(String, i32)>,
    pub battery_level: Option<u8>,
    pub is_charging: Option<bool>,
    pub selected_files: Vec<String>,
}

pub struct CosmicConnect {
    core: Core,
    popup: Option<Id>,
    config: ConnectConfig,
    event_sender: Option<Arc<mpsc::UnboundedSender<AppEvent>>>,
    connections: HashMap<DeviceId, Device>,
    device_state: State,
}

#[derive(Debug, Clone)]
pub enum CosmicEvent {
    Broadcasting,
    Pair(DeviceId),
    Unpair(DeviceId),
    SendPing((DeviceId, String)),
    SendFiles((DeviceId, Vec<String>)),
    Stop,
}

#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    UpdateConfig(ConnectConfig),
    KdeConnectStarted(Arc<mpsc::UnboundedSender<AppEvent>>),
    NewConnection((DeviceId, Device)),
    ClipboardReceived(String),
    Disconnected(DeviceId),
    SendEvent(CosmicEvent),
    UpdatedState(DeviceState),
    SelectFiles,
    CollectFiles(Vec<String>),
    SendFiles(DeviceId),
    OpenAbout,
}

impl Application for CosmicConnect {
    type Executor = cosmic::executor::Default;

    type Flags = ();

    type Message = Message;

    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Self::Message>) {
        let config = ConnectConfig::config();

        let app = CosmicConnect {
            core,
            popup: None,
            config,
            event_sender: None,
            connections: HashMap::new(),
            device_state: State::default(),
        };

        (app, Task::none())
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let mut subscriptions = Vec::new();

        let kdeconnect = Subscription::run_with_id(
            1,
            stream::channel(100, |mut output| async move {
                let (mut core, mut connection_rx) = KdeConnectCore::new()
                    .await
                    .expect("creating kdeconnect core instance");

                let sender = core.take_events();

                tokio::spawn(async move {
                    core.run_event_loop().await;
                });

                let _ = output.send(Message::KdeConnectStarted(sender)).await;

                while let Some(event) = connection_rx.recv().await {
                    match event {
                        ConnectionEvent::ClipboardReceived(content) => {
                            let _ = output.send(Message::ClipboardReceived(content)).await;
                        }
                        ConnectionEvent::Connected((device_id, device)) => {
                            let _ = output
                                .send(Message::NewConnection((device_id, device)))
                                .await;
                        }
                        ConnectionEvent::DevicePaired((device_id, device)) => {
                            let _ = output
                                .send(Message::NewConnection((device_id, device)))
                                .await;
                        }
                        ConnectionEvent::Disconnected(device_id) => {
                            let _ = output.send(Message::Disconnected(device_id)).await;
                        }
                        ConnectionEvent::StateUpdated(state) => {
                            let _ = output.send(Message::UpdatedState(state)).await;
                        }
                    }
                }
            }),
        );

        subscriptions.push(kdeconnect);

        let config = self
            .core()
            .watch_config::<ConnectConfig>(Self::APP_ID)
            .map(|update| Message::UpdateConfig(update.config));

        subscriptions.push(config);

        Subscription::batch(subscriptions)
    }

    fn view(&self) -> Element<'_, Self::Message> {
        self.core
            .applet
            .icon_button("dev.heppen.CosmicExtConnect")
            .on_press(Message::TogglePopup)
            .into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let Spacing {
            space_xxs, space_s, ..
        } = theme::active().cosmic().spacing;

        let content = self.device_list_col(self.connections.values().collect::<Vec<&Device>>());

        self.core
            .applet
            .popup_container(padded_control(container(content)).padding([space_xxs, space_s]))
            .into()
    }

    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::TogglePopup => {
                return if let Some(p) = self.popup.take() {
                    destroy_popup(p)
                } else {
                    let new_id = Id::unique();
                    self.popup.replace(new_id);
                    let mut popup_settings = self.core.applet.get_popup_settings(
                        self.core.main_window_id().unwrap(),
                        new_id,
                        None,
                        None,
                        None,
                    );
                    popup_settings.positioner.size_limits = Limits::NONE
                        .max_width(372.0)
                        .min_width(300.0)
                        .min_height(200.0)
                        .max_height(1080.0);
                    get_popup(popup_settings)
                };
            }
            Message::PopupClosed(id) => {
                if self.popup.as_ref() == Some(&id) {
                    self.popup = None;
                }
            }
            Message::UpdateConfig(config) => {
                self.config = config;
            }
            Message::KdeConnectStarted(sender) => {
                self.event_sender = Some(sender);
            }
            Message::NewConnection((device_id, device)) => {
                self.connections.entry(device_id).or_insert(device);
            }
            Message::ClipboardReceived(content) => {
                debug!("Clipboard content received: {}", content);
                return cosmic::iced::clipboard::write(content);
            }
            Message::Disconnected(device_id) => {
                self.connections.remove_entry(&device_id);
            }
            Message::SendEvent(event) => match event {
                CosmicEvent::Broadcasting => {
                    if let Some(sender) = self.event_sender.as_ref() {
                        let _ = sender.send(AppEvent::Broadcasting);
                    }
                }
                CosmicEvent::Pair(device_id) => {
                    if let Some(sender) = self.event_sender.as_ref() {
                        let _ = sender.send(AppEvent::Pair(device_id));
                    }
                }
                CosmicEvent::Unpair(device_id) => {
                    if let Some(sender) = self.event_sender.as_ref() {
                        let _ = sender.send(AppEvent::Unpair(device_id));
                    }
                }
                CosmicEvent::SendPing((device_id, msg)) => {
                    if let Some(sender) = self.event_sender.as_ref() {
                        let _ = sender.send(AppEvent::Ping((device_id, msg)));
                    }
                }
                CosmicEvent::SendFiles((device_id, files_list)) => {
                    if let Some(sender) = self.event_sender.as_ref() {
                        let _ = sender.send(AppEvent::SendFiles((device_id, files_list)));
                    }
                }
                CosmicEvent::Stop => {
                    if let Some(sender) = self.event_sender.as_ref() {
                        let _ = sender.send(AppEvent::StopKdeConnect);
                    }

                    self.event_sender = None;
                    self.connections.clear();
                    self.device_state = State::default();
                }
            },
            Message::UpdatedState(state) => match state {
                DeviceState::Battery { level, charging } => {
                    self.device_state.battery_level = Some(level);
                    self.device_state.is_charging = Some(charging);
                }
                DeviceState::Connectivity(connectivity) => {
                    self.device_state.connectivity = Some(connectivity);
                }
            },
            Message::OpenAbout => {
                let _ = open::that_detached("https://github.com/hepp3n/kdeconnect");
            }
            Message::SelectFiles => {
                return Task::future(async move {
                    let files = SelectedFiles::open_file()
                        .title("Select file or files to send")
                        .accept_label("Select")
                        .modal(true)
                        .multiple(true)
                        .send()
                        .await
                        .unwrap()
                        .response()
                        .unwrap();

                    let paths = files
                        .uris()
                        .iter()
                        .map(|u| {
                            percent_decode(u.path().as_bytes())
                                .decode_utf8()
                                .unwrap()
                                .to_string()
                        })
                        .collect::<Vec<String>>();

                    cosmic::Action::App(Message::CollectFiles(paths))
                });
            }
            Message::CollectFiles(files) => {
                self.device_state.selected_files = files;
            }
            Message::SendFiles(device_id) => {
                let files_list = self.device_state.selected_files.clone();

                self.device_state.selected_files.clear();

                return Task::done(cosmic::Action::App(Message::SendEvent(
                    CosmicEvent::SendFiles((device_id, files_list)),
                )));
            }
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced_runtime::Appearance> {
        Some(cosmic::applet::style())
    }
}

impl CosmicConnect {
    fn is_paired(&self, device_id: DeviceId) -> bool {
        self.connections
            .get(&device_id)
            .is_some_and(|state| state.pair_state == PairState::Paired)
    }

    fn device_list_col(
        &self,
        devices: Vec<&Device>,
    ) -> column::Column<'static, Message, Theme, Renderer> {
        let Spacing { space_xxs, .. } = theme::active().cosmic().spacing;

        let title = format!("#{}", devices.len());

        let battery_status = format!(
            "{}% [{}]",
            self.device_state.battery_level.unwrap_or(0),
            if self.device_state.is_charging.unwrap_or(false) {
                "Charging"
            } else {
                "Discharging"
            }
        );

        if !devices.is_empty() {
            devices.iter().cloned().enumerate().fold(
                column::Column::new()
                    .width(Length::Fill)
                    .push(Self::device_list_header(title)),
                |col, device| {
                    col.push(
                        container(
                            row()
                                .push(menu_button(
                                    text::title4(device.1.name.clone())
                                        .width(Length::Fill)
                                        .align_y(Vertical::Center)
                                        .line_height(LineHeight::Absolute(Pixels::from(60))),
                                ))
                                .push(
                                    button::icon(icon::from_name("mail-send-symbolic"))
                                        .on_press(Message::SendEvent(CosmicEvent::SendPing((
                                            device.1.device_id.clone(),
                                            fl!("ping-message"),
                                        ))))
                                        .apply(container)
                                        .center(Length::Fill)
                                        .width(Length::Fixed(60.0))
                                        .height(Length::Fixed(60.0)),
                                )
                                .push(if self.is_paired(device.1.device_id.clone()) {
                                    button::icon(icon::from_name("network-disconnected-symbolic"))
                                        .on_press(Message::SendEvent(CosmicEvent::Unpair(
                                            device.1.device_id.clone(),
                                        )))
                                        .class(theme::Button::Destructive)
                                        .apply(container)
                                        .center(Length::Fill)
                                        .width(Length::Fixed(60.0))
                                        .height(Length::Fixed(60.0))
                                } else {
                                    button::icon(icon::from_name("list-add-symbolic"))
                                        .on_press(Message::SendEvent(CosmicEvent::Pair(
                                            device.1.device_id.clone(),
                                        )))
                                        .class(theme::Button::Suggested)
                                        .apply(container)
                                        .center(Length::Fill)
                                        .width(Length::Fixed(60.0))
                                        .height(Length::Fixed(60.0))
                                }),
                        )
                        .center_y(Length::Fixed(60.)),
                    )
                    .push(horizontal_rule(Pixels::from(1)))
                    .push(horizontal_rule(Pixels::from(1)))
                    .push_maybe(self.device_state.connectivity.as_ref().map(|networks| {
                        menu_button(
                            row()
                                .push(text::body(format!("Network ({})", networks.0)))
                                .push(horizontal_space())
                                .push(text::body(format!("Signal: {}", networks.1))),
                        )
                        .width(Length::Fill)
                    }))
                    .push(
                        menu_button(
                            row()
                                .push(text::body("Battery"))
                                .push(horizontal_space())
                                .push(text::body(battery_status.clone())),
                        )
                        .width(Length::Fill),
                    )
                    .push(
                        menu_button(
                            row()
                                .spacing(space_xxs)
                                .push(text::body("Send files"))
                                .push(horizontal_space())
                                .push(
                                    button::icon(icon::from_name("list-add-symbolic"))
                                        .on_press(Message::SelectFiles)
                                        .class(theme::Button::Standard)
                                        .apply(container)
                                        .center(Length::Fill)
                                        .width(Length::Fixed(32.0))
                                        .height(Length::Fixed(32.0)),
                                )
                                .push_maybe(if !self.device_state.selected_files.is_empty() {
                                    Some(
                                        button::icon(icon::from_name("mail-send-symbolic"))
                                            .on_press(Message::SendFiles(
                                                device.1.device_id.clone(),
                                            ))
                                            .class(theme::Button::Suggested)
                                            .apply(container)
                                            .center(Length::Fill)
                                            .width(Length::Fixed(32.0))
                                            .height(Length::Fixed(32.0)),
                                    )
                                } else {
                                    None
                                }),
                        )
                        .width(Length::Fill),
                    )
                    .push_maybe(
                        if !self.device_state.selected_files.is_empty() {
                            Some(self.device_state.selected_files.clone().into_iter().fold(
                                column::Column::new().width(Length::Fill),
                                |col, file_path| col.push(menu_button(text::body(file_path))),
                            ))
                        } else {
                            None
                        },
                    )
                },
            )
        } else {
            Self::device_list_header(title)
        }
    }

    fn device_list_header(title: String) -> Column<'static, Message, Theme, Renderer> {
        column::Column::new()
            .push(
                container(
                    row()
                        .spacing(4)
                        .push(
                            button::suggested("About")
                                .on_press(Message::OpenAbout)
                                .apply(container)
                                .center(Length::Fill)
                                .height(Length::Fixed(60.0)),
                        )
                        .push(
                            button::standard("Broadcast")
                                .on_press(Message::SendEvent(CosmicEvent::Broadcasting))
                                .apply(container)
                                .center(Length::Fill)
                                .height(Length::Fixed(60.0)),
                        )
                        .push(
                            button::destructive("Disconnect")
                                .on_press(Message::SendEvent(CosmicEvent::Stop))
                                .apply(container)
                                .center(Length::Fill)
                                .height(Length::Fixed(60.0)),
                        )
                        .push(menu_button(
                            text::title4(title)
                                .align_y(Vertical::Center)
                                .line_height(LineHeight::Absolute(Pixels::from(60))),
                        )),
                )
                .center_y(Length::Fixed(60.0)),
            )
            .push(horizontal_rule(Pixels::from(2)))
    }
}
