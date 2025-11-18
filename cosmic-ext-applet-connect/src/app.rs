// SPDX-License-Identifier: GPL-3.0-only

use std::collections::HashMap;
use std::sync::Arc;

use cosmic::app::{Core, Task};
use cosmic::iced::futures::SinkExt;
use cosmic::iced::window::Id;
use cosmic::iced::{self, Limits, Subscription, stream};
use cosmic::iced_winit::commands::popup::{destroy_popup, get_popup};
use cosmic::widget::{self, settings};
use cosmic::{Application, Element};
use kdeconnect_core::KdeConnectCore;
use kdeconnect_core::device::{Device, DeviceId, PairState};
use kdeconnect_core::event::{AppEvent, ConnectionEvent, DeviceState};
use tokio::sync::mpsc;
use tracing::debug;

use crate::config::ConnectConfig;
use crate::{APP_ID, fl};

#[derive(Debug, Clone, Default)]
pub struct State {
    pub battery_level: Option<u8>,
    pub is_charging: Option<bool>,
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
    Pair(DeviceId),
    Unpair(DeviceId),
    SendPing((DeviceId, String)),
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
            .icon_button("display-symbolic")
            .on_press(Message::TogglePopup)
            .into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let mut content_list = widget::list_column().add(widget::settings::flex_item_row(vec![
            widget::text(fl!("applet-name")).into(),
            widget::button::standard("Broadcast").into(),
        ]));

        for device in self.connections.values() {
            content_list = content_list.add(settings::item_row(vec![
                widget::text::monotext(device.name.clone()).into(),
                widget::button::standard("Disconnect").into(),
                if self.is_paired(device.device_id.clone()) {
                    widget::button::standard("Unpair")
                        .on_press(Message::SendEvent(CosmicEvent::Unpair(
                            device.device_id.clone(),
                        )))
                        .into()
                } else {
                    widget::button::standard("Pair")
                        .on_press(Message::SendEvent(CosmicEvent::Pair(
                            device.device_id.clone(),
                        )))
                        .into()
                },
                widget::button::standard("Send Ping")
                    .on_press(Message::SendEvent(CosmicEvent::SendPing((
                        device.device_id.clone(),
                        "Hello From COSMIC APPLET!".to_string(),
                    ))))
                    .into(),
            ]));

            let mut section = settings::section().title(device.device_id.to_string());

            // if let Some(networks) = state.connectivity.as_ref() {
            //     for (_, network) in &networks.signal_strengths {
            //         section = section.add(settings::item(
            //             format!("Network ({})", network.network_type),
            //             widget::text(format!("Signal: {}", network.signal_strength)),
            //         ));
            //     }
            // };

            section = section.add(settings::item(
                "Battery",
                widget::text(format!(
                    "{}% [{}]",
                    self.device_state.battery_level.unwrap_or(0),
                    if self.device_state.is_charging.unwrap_or(false) {
                        "Charging"
                    } else {
                        "Discharging"
                    }
                )),
            ));
            content_list = content_list.add(section);
        }

        self.core
            .applet
            .popup_container(content_list)
            .limits(iced::Limits::NONE.max_width(680.0).max_height(800.0))
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
            },
            Message::UpdatedState(state) => match state {
                DeviceState::Battery { level, charging } => {
                    self.device_state.battery_level = Some(level);
                    self.device_state.is_charging = Some(charging);
                }
            },
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
}
