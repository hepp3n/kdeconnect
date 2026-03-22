mod backend;
mod messages;
mod models;
mod notifications;
mod plugins;
mod portal;
mod ui;

use messages::Message;
use models::Device;

use cosmic::app::Core;
use cosmic::iced::window::Id as SurfaceId;
use cosmic::iced::{Limits, Subscription};
use cosmic::iced_winit::commands::popup::{destroy_popup, get_popup};
use cosmic::{Element, Task, widget};
use std::collections::HashMap;
use tracing::{debug, error, info};

pub struct KdeConnectApplet {
    core: Core,
    popup: Option<SurfaceId>,
    devices: HashMap<String, Device>,
    expanded_device: Option<String>,
    /// Pending pairing requests: device_id → device_name
    pairing_requests: HashMap<String, String>,
}

impl cosmic::Application for KdeConnectApplet {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = "io.github.hepp3n.kdeconnect";

    fn core(&self) -> &Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<cosmic::Action<Self::Message>>) {
        tokio::spawn(async {
            if let Err(e) = backend::initialize().await {
                error!("Backend init failed: {:?}", e);
            }
        });

        let app = KdeConnectApplet {
            core,
            popup: None,
            devices: HashMap::new(),
            expanded_device: None,
            pairing_requests: HashMap::new(),
        };

        (app, Task::none())
    }

    fn on_close_requested(&self, id: SurfaceId) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::TogglePopup => {
                return if let Some(p) = self.popup.take() {
                    destroy_popup(p)
                } else {
                    let new_id = SurfaceId::unique();
                    self.popup.replace(new_id);

                    let mut popup_settings = self.core.applet.get_popup_settings(
                        self.core.main_window_id().unwrap(),
                        new_id,
                        None,
                        None,
                        None,
                    );
                    popup_settings.positioner.size_limits = Limits::NONE
                        .max_width(400.0)
                        .min_width(300.0)
                        .min_height(200.0)
                        .max_height(600.0);

                    Task::batch(vec![
                        get_popup(popup_settings),
                        Task::perform(backend::fetch_devices(), |devices| {
                            cosmic::Action::App(Message::DevicesUpdated(devices))
                        }),
                    ])
                };
            }
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                }
            }
            Message::RefreshDevices => {
                return Task::perform(backend::fetch_devices(), |devices| {
                    cosmic::Action::App(Message::DevicesUpdated(devices))
                });
            }
            Message::DevicesUpdated(devices) => {
                self.devices.clear();
                for device in devices {
                    self.devices.insert(device.id.clone(), device);
                }
            }
            Message::DelayedRefresh => {
                return Task::perform(backend::fetch_devices(), |devices| {
                    cosmic::Action::App(Message::DevicesUpdated(devices))
                });
            }
            Message::ToggleDeviceMenu(ref device_id) => {
                if self.expanded_device.as_ref() == Some(device_id) {
                    self.expanded_device = None;
                } else {
                    self.expanded_device = Some(device_id.clone());
                }
            }
            Message::SendSMS(ref device_id) => {
                // Look up device name for the window title
                let device_name = self
                    .devices
                    .get(device_id)
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| "Unknown Device".to_string());
                let id = device_id.clone();

                info!(
                    "Launching SMS window for device={} name={}",
                    id, device_name
                );

                // Spawn in a thread so the process::Command doesn't block the executor
                std::thread::spawn(move || {
                    match std::process::Command::new("cosmic-ext-connect-sms")
                        .arg(&id)
                        .arg(&device_name)
                        .spawn()
                    {
                        Ok(_) => info!("cosmic-ext-connect-sms launched"),
                        Err(e) => error!("Failed to launch cosmic-ext-connect-sms: {:?}", e),
                    }
                });
            }
            Message::PingDevice(ref device_id) => {
                let id = device_id.clone();
                return Task::perform(
                    async move {
                        backend::ping_device(id).await.ok();
                    },
                    |_| cosmic::Action::App(Message::RefreshDevices),
                );
            }
            Message::RingDevice(ref device_id) => {
                let id = device_id.clone();
                return Task::perform(
                    async move {
                        backend::ring_device(id).await.ok();
                    },
                    |_| cosmic::Action::App(Message::RefreshDevices),
                );
            }
            Message::BrowseDevice(ref device_id) => {
                let id = device_id.clone();
                return Task::perform(
                    async move {
                        backend::browse_device_filesystem(id).await.ok();
                    },
                    |_| cosmic::Action::App(Message::RefreshDevices),
                );
            }
            Message::PairDevice(ref device_id) => {
                let id = device_id.clone();
                return Task::perform(
                    async move {
                        backend::pair_device(id).await.ok();
                    },
                    |_| cosmic::Action::App(Message::RefreshDevices),
                );
            }
            Message::UnpairDevice(ref device_id) => {
                let id = device_id.clone();
                return Task::perform(
                    async move {
                        backend::unpair_device(id).await.ok();
                    },
                    |_| cosmic::Action::App(Message::RefreshDevices),
                );
            }
            Message::SendFiles(ref device_id) => {
                let id = device_id.clone();
                return Task::perform(
                    async move {
                        let files = portal::pick_files("Select files to send", true, None).await;
                        if !files.is_empty() {
                            backend::send_files(id, files).await.ok();
                        }
                    },
                    |_| cosmic::Action::App(Message::RefreshDevices),
                );
            }
            Message::UpdateTransferProgress(progress) => {
                if let Some(ref current_device) = self.expanded_device {
                    if let Some(device) = self.devices.get_mut(current_device) {
                        device.share_progress = if progress < 100 { Some(progress) } else { None };
                    }
                }
            }
            Message::ShareClipboard(ref device_id) => {
                let id = device_id.clone();
                return cosmic::iced::clipboard::read().map(move |content| {
                    cosmic::Action::App(Message::ClipboardReadForDevice(
                        id.clone(),
                        content.unwrap_or_default(),
                    ))
                });
            }
            Message::ClipboardReadForDevice(device_id, content) => {
                if !content.is_empty() {
                    return Task::perform(
                        async move { backend::send_clipboard(device_id, content).await.ok(); },
                        |_| cosmic::Action::App(Message::RefreshDevices),
                    );
                }
            }
            Message::ClipboardReceived(content) => {
                return cosmic::iced::clipboard::write::<cosmic::Action<Message>>(content);
            }
            Message::BatteryUpdated(device_id, level, charging) => {
                if let Some(device) = self.devices.get_mut(&device_id) {
                    device.battery_level = Some(level);
                    device.is_charging = Some(charging);
                    // Also patch the backend cache so the next fetch_devices() preserves it
                    let d = device.clone();
                    tokio::spawn(async move { backend::update_device(device_id, d).await; });
                }
            }
            Message::ConnectivityUpdated(device_id, strength) => {
                if let Some(device) = self.devices.get_mut(&device_id) {
                    device.signal_strength = Some(strength);
                    let d = device.clone();
                    tokio::spawn(async move { backend::update_device(device_id, d).await; });
                }
            }
            Message::AcceptPairing(ref device_id) => {
                self.pairing_requests.remove(device_id);
                let id = device_id.clone();
                return Task::perform(
                    async move {
                        backend::accept_pairing(id).await.ok();
                    },
                    |_| cosmic::Action::App(Message::RefreshDevices),
                );
            }
            Message::RejectPairing(ref device_id) => {
                self.pairing_requests.remove(device_id);
                let id = device_id.clone();
                return Task::perform(
                    async move {
                        backend::reject_pairing(id).await.ok();
                    },
                    |_| cosmic::Action::App(Message::RefreshDevices),
                );
            }
            Message::PairingRequestReceived(device_id, device_name, _device_type) => {
                info!("Pairing request received from {} ({})", device_name, device_id);
                self.pairing_requests.insert(device_id, device_name.clone());

                // Show a system notification so the user is alerted even if they
                // are not looking at the panel. COSMIC's daemon doesn't support
                // action buttons so we just point them to the applet.
                let notif_body = format!(
                    "'{}' wants to pair with this device. Click the KDE Connect applet to accept or decline.",
                    device_name
                );
                tokio::task::spawn_blocking(move || {
                    let _ = notify_rust::Notification::new()
                        .appname("KDE Connect")
                        .summary("Pairing Request")
                        .body(&notif_body)
                        .icon("network-wireless-symbolic")
                        .show();
                });

                // Ensure popup is open so the user sees Accept/Decline immediately.
                if self.popup.is_none() {
                    let new_id = SurfaceId::unique();
                    self.popup.replace(new_id);
                    let mut popup_settings = self.core.applet.get_popup_settings(
                        self.core.main_window_id().unwrap(),
                        new_id,
                        None,
                        None,
                        None,
                    );
                    popup_settings.positioner.size_limits = Limits::NONE
                        .max_width(400.0)
                        .min_width(300.0)
                        .min_height(200.0)
                        .max_height(600.0);
                    return get_popup(popup_settings);
                }
            }
            Message::MprisReceived(device_id, mpris_data) => {
                debug!("MPRIS from {}: {:?}", device_id, mpris_data);
            }
            Message::OpenSettings => {
                std::process::Command::new("cosmic-ext-connect-settings")
                    .spawn()
                    .ok();
            }
            Message::RemoteInput(ref device_id) => {
                debug!("Remote input: {}", device_id);
            }
            Message::LockDevice(ref device_id) => {
                debug!("Lock device: {}", device_id);
            }
            Message::PresenterMode(ref device_id) => {
                debug!("Presenter mode: {}", device_id);
            }
            Message::UseAsMonitor(ref device_id) => {
                debug!("Use as monitor: {}", device_id);
            }
            Message::ShareText(ref device_id) => {
                debug!("Share text: {}", device_id);
            }
            Message::ShareUrl(ref device_id) => {
                debug!("Share URL: {}", device_id);
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        self.core
            .applet
            .icon_button("phone-symbolic")
            .on_press(Message::TogglePopup)
            .into()
    }

    fn view_window(&self, id: SurfaceId) -> Element<'_, Self::Message> {
        let Some(popup_id) = self.popup else {
            return widget::text("").into();
        };
        if id != popup_id {
            return widget::text("").into();
        }
        ui::popup::create_popup_view(
            &self.core,
            &self.devices,
            self.expanded_device.as_ref(),
            Some(&self.pairing_requests),
        )
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        use futures::StreamExt as _;
        Subscription::batch(vec![
            cosmic::iced::time::every(std::time::Duration::from_secs(10))
                .map(|_| Message::RefreshDevices),
            backend::filetransfer_subscription(),
            backend::service_watcher_subscription(),
            // D-Bus event stream — delivers pairing requests and device state
            // changes in real time without waiting for the 10s poll.
            Subscription::run(|| {
                async_stream::stream! {
                    let mut stream = backend::event_stream().await;
                    while let Some(event) = stream.next().await {
                        match event {
                            kdeconnect_dbus_client::ServiceEvent::PairingRequested(id, name) => {
                                yield Message::PairingRequestReceived(id, name, "phone".to_string());
                            }
                            kdeconnect_dbus_client::ServiceEvent::ClipboardReceived(content) => {
                                yield Message::ClipboardReceived(content);
                            }
                            kdeconnect_dbus_client::ServiceEvent::BatteryReceived(id, level, charging) => {
                                yield Message::BatteryUpdated(id, level, charging);
                            }
                            kdeconnect_dbus_client::ServiceEvent::ConnectivityReceived(id, strength) => {
                                yield Message::ConnectivityUpdated(id, strength);
                            }
                            kdeconnect_dbus_client::ServiceEvent::DeviceConnected(id, _)
                            | kdeconnect_dbus_client::ServiceEvent::DevicePaired(id, _)
                            | kdeconnect_dbus_client::ServiceEvent::DeviceDisconnected(id) => {
                                let _ = id;
                                yield Message::RefreshDevices;
                            }
                            _ => {}
                        }
                    }
                }
            }),
        ])
    }
}

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    ctrlc::set_handler(move || std::process::exit(0)).ok();
    cosmic::applet::run::<KdeConnectApplet>(())
}
