use crate::messages::Message;
use crate::models::Device;
use cosmic::app::Core;
use cosmic::iced::{Alignment, Length};
use cosmic::{Element, widget};
use std::collections::HashMap;

/// Build the popup view using the real application Core so popup_container
/// has proper applet context, theme, and sizing.
pub fn create_popup_view<'a>(
    core: &'a Core,
    devices: &'a HashMap<String, Device>,
    expanded_device: Option<&'a String>,
    _expanded_player_menu: Option<&'a String>,
) -> Element<'a, Message> {
    let spacing = cosmic::theme::active().cosmic().spacing;
    let mut content = widget::column()
        .spacing(spacing.space_s)
        .padding(spacing.space_s);

    // Header
    content = content.push(
        widget::row()
            .push(
                widget::text("Cosmic Ext Connect")
                    .size(18)
                    .width(Length::Fill),
            )
            .push(widget::button::standard("Settings").on_press(Message::OpenSettings))
            .spacing(spacing.space_xs)
            .align_y(Alignment::Center),
    );

    content = content.push(widget::divider::horizontal::default());

    // Pairing requests
    let mut pairing_requests: Vec<_> = devices
        .values()
        .filter(|d| d.pairing_requests > 0 && !d.is_paired)
        .collect();
    pairing_requests.sort_by(|a, b| a.name.cmp(&b.name));

    if !pairing_requests.is_empty() {
        content = content.push(
            widget::text("Pairing Requests")
                .size(14)
                .font(cosmic::font::bold()),
        );

        for device in pairing_requests {
            let device_id_accept = device.id.clone();
            let device_id_reject = device.id.clone();

            let request_card = widget::container(
                widget::column()
                    .push(
                        widget::row()
                            .push(widget::icon::from_name(device.device_icon()).size(24))
                            .push(
                                widget::column()
                                    .push(widget::text(&device.name).size(14))
                                    .push(widget::text(&device.device_type).size(11))
                                    .spacing(spacing.space_xxxs),
                            )
                            .spacing(spacing.space_s)
                            .align_y(Alignment::Center),
                    )
                    .push(widget::Space::new().height(Length::Fixed(spacing.space_xs as f32)))
                    .push(
                        widget::row()
                            .push(
                                widget::button::suggested("Accept")
                                    .on_press(Message::AcceptPairing(device_id_accept))
                                    .width(Length::Fill),
                            )
                            .push(
                                widget::button::destructive("Reject")
                                    .on_press(Message::RejectPairing(device_id_reject))
                                    .width(Length::Fill),
                            )
                            .spacing(spacing.space_xs),
                    )
                    .spacing(spacing.space_xs),
            )
            .padding(spacing.space_s)
            .class(cosmic::theme::Container::Card)
            .width(Length::Fill);

            content = content.push(request_card);
        }

        content = content.push(widget::divider::horizontal::default());
    }

    // All paired devices — reachable and unreachable — sorted alphabetically
    let mut paired_devices: Vec<_> = devices.values().filter(|d| d.is_paired).collect();
    paired_devices.sort_by(|a, b| a.name.cmp(&b.name));

    if paired_devices.is_empty() {
        content = content.push(
            widget::container(widget::text("No paired devices").size(14))
                .padding(spacing.space_m)
                .width(Length::Fill)
                .center_x(Length::Fill),
        );
    } else {
        content = content.push(widget::text("Devices").size(14).font(cosmic::font::bold()));

        for device in paired_devices {
            content = content.push(create_device_card(device, &spacing, expanded_device));
        }
    }

    let popup_content = widget::container(widget::scrollable(content))
        .width(Length::Fixed(400.0))
        .max_height(700.0)
        .padding(spacing.space_xs)
        .class(cosmic::theme::Container::Dialog);

    // Use the real Core so the popup has proper applet context and theme
    core.applet.popup_container(popup_content).into()
}

fn create_device_card<'a>(
    device: &'a Device,
    spacing: &cosmic::cosmic_theme::Spacing,
    expanded_device: Option<&'a String>,
) -> Element<'a, Message> {
    let is_expanded = expanded_device == Some(&device.id);
    let is_online = device.is_reachable;

    let mut name_row = widget::row()
        .push(widget::icon::from_name(device.device_icon()).size(20))
        .push(widget::text(&device.name).size(14).width(Length::Fill))
        .spacing(spacing.space_xs)
        .align_y(Alignment::Center);

    if !is_online {
        name_row = name_row.push(widget::text("Offline").size(11));
    } else if let Some(level) = device.battery_level {
        name_row = name_row.push(widget::text(format!("{}%", level)).size(11));
    }

    name_row = name_row.push(
        widget::button::icon(widget::icon::from_name(if is_expanded {
            "go-up-symbolic"
        } else {
            "go-down-symbolic"
        }))
        .on_press(Message::ToggleDeviceMenu(device.id.clone()))
        .class(cosmic::theme::Button::Icon),
    );

    let device_button = widget::button::custom(name_row)
        .on_press(Message::ToggleDeviceMenu(device.id.clone()))
        .width(Length::Fill)
        .class(cosmic::theme::Button::Text);

    let mut col = widget::column().push(device_button);

    if is_expanded && is_online {
        let mut menu_items = widget::column().spacing(spacing.space_xxs);

        menu_items = menu_items.push(
            widget::text("Quick Actions")
                .size(12)
                .font(cosmic::font::bold()),
        );
        menu_items = menu_items.push(
            widget::button::text("Ping")
                .on_press(Message::PingDevice(device.id.clone()))
                .width(Length::Fill)
                .class(cosmic::theme::Button::Text),
        );

        if device.has_findmyphone {
            menu_items = menu_items.push(
                widget::button::text("Find my phone")
                    .on_press(Message::RingDevice(device.id.clone()))
                    .width(Length::Fill)
                    .class(cosmic::theme::Button::Text),
            );
        }

        if device.has_clipboard {
            menu_items = menu_items.push(
                widget::button::text("Share clipboard")
                    .on_press(Message::ShareClipboard(device.id.clone()))
                    .width(Length::Fill)
                    .class(cosmic::theme::Button::Text),
            );
        }

        menu_items = menu_items.push(
            widget::button::text("SMS Messages")
                .on_press(Message::SendSMS(device.id.clone()))
                .width(Length::Fill)
                .class(cosmic::theme::Button::Text),
        );

        if device.has_share || device.has_sftp {
            menu_items = menu_items.push(widget::divider::horizontal::light());
            menu_items = menu_items.push(widget::text("Files").size(12).font(cosmic::font::bold()));

            if device.has_share {
                menu_items = menu_items.push(
                    widget::button::text("Send file")
                        .on_press(Message::SendFiles(device.id.clone()))
                        .width(Length::Fill)
                        .class(cosmic::theme::Button::Text),
                );
                menu_items = menu_items.push_maybe(if let Some(progress) = device.share_progress {
                    Some(widget::progress_bar(0.0..=100.0, progress as f32))
                } else {
                    None
                });
            }

            if device.has_sftp {
                menu_items = menu_items.push(
                    widget::button::text("Browse device")
                        .on_press(Message::BrowseDevice(device.id.clone()))
                        .width(Length::Fill)
                        .class(cosmic::theme::Button::Text),
                );
            }
        }

        col = col.push(
            widget::container(menu_items)
                .padding([spacing.space_xs, spacing.space_m])
                .class(cosmic::theme::Container::Background),
        );
    } else if is_expanded && !is_online {
        col = col.push(
            widget::container(widget::text("Device is not reachable").size(12))
                .padding([spacing.space_xs, spacing.space_m])
                .class(cosmic::theme::Container::Background),
        );
    }

    col.into()
}
