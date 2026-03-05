//! UI view implementations for the SMS window.

use cosmic::Element;
use cosmic::iced::{Alignment, Length};
use cosmic::widget;

use super::app::{SmsMessage, SmsWindow};
use super::models::Conversation;
use super::utils::{format_timestamp, normalize_phone_number, phone_numbers_match};

/// Main view - conversations list + thread view
pub fn view_main(app: &SmsWindow) -> Element<'_, SmsMessage> {
    let spacing = cosmic::theme::active().cosmic().spacing;

    widget::row()
        .spacing(0)
        .push(view_conversations_list(app, &spacing))
        .push(widget::divider::vertical::default())
        .push(view_thread_panel(app, &spacing))
        .into()
}

/// New chat dialog view
pub fn view_new_chat_dialog(app: &SmsWindow) -> Element<'_, SmsMessage> {
    let spacing = cosmic::theme::active().cosmic().spacing;

    // Left column: title, input, actions
    let left = widget::column()
        .spacing(spacing.space_m)
        .padding(spacing.space_l)
        .push(
            widget::text("Start New Chat")
                .size(20)
                .font(cosmic::font::bold()),
        )
        .push(
            widget::column()
                .spacing(spacing.space_xs)
                .push(widget::text("Enter phone number or contact name:").size(14))
                .push(
                    widget::text_input(
                        "e.g., +1-555-123-4567 or John Doe",
                        &app.new_chat_phone_input,
                    )
                    .on_input(SmsMessage::UpdateNewChatPhone)
                    .width(Length::Fill),
                ),
        )
        .push(view_new_chat_actions(app, &spacing))
        .width(Length::Fixed(340.0));

    // Right column: contacts list
    let right = widget::column()
        .spacing(spacing.space_s)
        .padding(spacing.space_l)
        .push(widget::text("Contacts").size(16).font(cosmic::font::bold()))
        .push(view_contacts_list(app, &spacing))
        .width(Length::Fill);

    let content = widget::row()
        .push(left)
        .push(widget::divider::vertical::default())
        .push(right)
        .height(Length::Fixed(500.0));

    widget::container(content)
        .class(cosmic::theme::Container::Card)
        .width(Length::Fill)
        .into()
}

fn view_new_chat_actions<'a>(
    app: &'a SmsWindow,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> Element<'a, SmsMessage> {
    let start_button_enabled = !app.new_chat_phone_input.trim().is_empty();

    widget::row()
        .spacing(spacing.space_xs)
        .push(widget::button::standard("Cancel").on_press(SmsMessage::CloseNewChatDialog))
        .push(widget::space::horizontal())
        .push(if start_button_enabled {
            widget::button::suggested("Start Chat").on_press(SmsMessage::CreateNewChat)
        } else {
            widget::button::suggested("Start Chat")
        })
        .into()
}

fn view_contacts_list<'a>(
    app: &'a SmsWindow,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> Element<'a, SmsMessage> {
    if app.contacts.is_empty() {
        return widget::text("No contacts available").size(12).into();
    }

    let mut contacts_list = widget::column().spacing(spacing.space_xxs);
    let filtered_contacts = get_filtered_contacts(app);

    if filtered_contacts.is_empty() {
        contacts_list = contacts_list.push(widget::text("No matching contacts").size(12));
    } else {
        for (phone, name) in filtered_contacts.iter() {
            contacts_list = contacts_list.push(
                widget::button::text(format!("{} ({})", name, phone))
                    .on_press(SmsMessage::SelectContactForNewChat(
                        phone.to_string(),
                        name.to_string(),
                    ))
                    .width(Length::Fill),
            );
        }
        contacts_list = contacts_list.push(
            widget::container(
                widget::text(format!(
                    "Showing {} contact{}",
                    filtered_contacts.len(),
                    if filtered_contacts.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                ))
                .size(11),
            )
            .padding([spacing.space_xs, 0, 0, 0]),
        );
    }

    widget::scrollable(contacts_list)
        .height(Length::Fill)
        .into()
}

fn get_filtered_contacts(app: &SmsWindow) -> Vec<(&String, &String)> {
    let mut sorted_contacts: Vec<_> = app.contacts.iter().collect();
    sorted_contacts.sort_by(|a, b| a.1.cmp(b.1));

    let search_term = app.new_chat_phone_input.trim().to_lowercase();

    if search_term.is_empty() {
        sorted_contacts
    } else {
        sorted_contacts
            .into_iter()
            .filter(|(phone, name)| {
                name.to_lowercase().contains(&search_term)
                    || phone.contains(&search_term)
                    || normalize_phone_number(phone).contains(&normalize_phone_number(&search_term))
            })
            .collect()
    }
}

/// Conversations list panel
fn view_conversations_list<'a>(
    app: &'a SmsWindow,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> Element<'a, SmsMessage> {
    let mut content = widget::column().spacing(spacing.space_xs);

    // Start Chat button
    content = content.push(
        widget::container(
            widget::button::suggested("Start Chat")
                .on_press(SmsMessage::OpenNewChatDialog)
                .width(Length::Fill),
        )
        .padding(spacing.space_s),
    );

    // Search input
    content = content.push(
        widget::text_input("Search conversations...", &app.search_query)
            .on_input(SmsMessage::UpdateSearch)
            .padding(spacing.space_s),
    );
    content = content.push(widget::divider::horizontal::default());

    // Filter conversations
    let mut filtered: Vec<_> = app
        .conversations
        .iter()
        .filter(|c| conversation_matches_search(app, c))
        .collect();

    // Sort by timestamp (most recent first)
    filtered.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    if filtered.is_empty() {
        let msg = if app.search_query.is_empty() {
            "No conversations"
        } else {
            "No matching conversations"
        };

        content = content.push(
            widget::container(widget::text(msg).size(14))
                .width(Length::Fill)
                .padding(spacing.space_xl)
                .center_x(Length::Fill),
        );
    } else {
        let mut list = widget::column().spacing(0);

        for conv in filtered {
            list = list.push(view_conversation_item(app, conv, spacing));
            list = list.push(widget::divider::horizontal::light());
        }

        content = content.push(widget::scrollable(list).height(Length::Fill));
    }

    widget::container(content)
        .width(Length::Fixed(300.0))
        .height(Length::Fill)
        .into()
}

fn conversation_matches_search(app: &SmsWindow, conv: &Conversation) -> bool {
    if app.search_query.is_empty() {
        return true;
    }

    let query = app.search_query.to_lowercase();
    conv.contact_name.to_lowercase().contains(&query)
        || conv.phone_number.contains(&app.search_query)
        || normalize_phone_number(&conv.phone_number)
            .contains(&normalize_phone_number(&app.search_query))
}

fn view_conversation_item<'a>(
    app: &'a SmsWindow,
    conv: &'a Conversation,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> Element<'a, SmsMessage> {
    let is_selected = app.selected_thread.as_ref() == Some(&conv.thread_id);

    let display_name =
        get_contact_name(app, &conv.phone_number).unwrap_or_else(|| conv.phone_number.clone());

    let button = widget::button::custom(
        widget::column()
            .push(
                widget::row()
                    .push(
                        widget::text(display_name)
                            .size(14)
                            .font(cosmic::font::bold()),
                    )
                    .push(widget::space::horizontal())
                    .push(widget::text(format_timestamp(conv.timestamp)).size(11))
                    .spacing(spacing.space_xs),
            )
            .push(widget::text(&conv.last_message).size(12))
            .spacing(spacing.space_xxs)
            .padding(spacing.space_s),
    )
    .on_press(SmsMessage::SelectThread(conv.thread_id.clone()))
    .width(Length::Fill);

    if is_selected {
        widget::container(button)
            .class(cosmic::theme::Container::Primary)
            .into()
    } else {
        button.class(cosmic::theme::Button::Text).into()
    }
}

/// Thread panel (messages + input)
fn view_thread_panel<'a>(
    app: &'a SmsWindow,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> Element<'a, SmsMessage> {
    let Some(thread_id) = &app.selected_thread else {
        return widget::container(widget::text("Select a conversation to view messages").size(14))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
    };

    let Some(conv) = app.conversations.iter().find(|c| c.thread_id == *thread_id) else {
        return widget::container(widget::text("Conversation not found").size(14))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
    };

    let mut content = widget::column().spacing(0);

    // Header
    content = content.push(view_thread_header(app, conv, spacing));
    content = content.push(widget::divider::horizontal::default());

    // Messages
    content = content.push(view_messages_list(app, spacing));
    content = content.push(widget::divider::horizontal::default());

    // Input
    content = content.push(view_message_input(app, spacing));

    widget::container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn view_thread_header<'a>(
    app: &'a SmsWindow,
    conv: &'a Conversation,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> Element<'a, SmsMessage> {
    let display_name =
        get_contact_name(app, &conv.phone_number).unwrap_or_else(|| conv.phone_number.clone());

    widget::container(
        widget::column()
            .push(
                widget::text(display_name)
                    .size(16)
                    .font(cosmic::font::bold()),
            )
            .push(widget::text(&conv.phone_number).size(12))
            .spacing(spacing.space_xxs)
            .padding(spacing.space_s),
    )
    .class(cosmic::theme::Container::Card)
    .width(Length::Fill)
    .into()
}

fn view_messages_list<'a>(
    app: &'a SmsWindow,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> Element<'a, SmsMessage> {
    let mut messages_column = widget::column()
        .spacing(spacing.space_m)
        .padding(spacing.space_m);

    if app.messages.is_empty() {
        messages_column = messages_column.push(
            widget::container(
                widget::column()
                    .push(widget::text("Waiting for messages...").size(14))
                    .push(
                        widget::text("Messages will appear as they arrive from your phone")
                            .size(12),
                    )
                    .spacing(spacing.space_xs)
                    .align_x(Alignment::Center),
            )
            .width(Length::Fill)
            .center_x(Length::Fill)
            .padding(spacing.space_xl),
        );
    } else {
        for msg in &app.messages {
            messages_column = messages_column.push(view_message_bubble(app, msg, spacing));
        }
    }

    widget::scrollable(messages_column)
        .height(Length::Fill)
        .direction(cosmic::iced::widget::scrollable::Direction::Vertical(
            cosmic::iced::widget::scrollable::Scrollbar::new()
                .anchor(cosmic::iced::widget::scrollable::Anchor::End),
        ))
        .into()
}

fn view_message_bubble<'a>(
    app: &'a SmsWindow,
    msg: &'a super::models::Message,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> Element<'a, SmsMessage> {
    let is_sent = msg.is_sent();
    let mut message_content = widget::column().spacing(spacing.space_xxs);

    // Show sender label only for received messages
    if !is_sent {
        let phone_number =
            get_current_conversation_phone(app).unwrap_or_else(|| msg.address.clone());

        let sender_label = get_contact_name(app, &phone_number).unwrap_or(phone_number);

        message_content = message_content.push(
            widget::text(sender_label)
                .size(11)
                .font(cosmic::font::bold()),
        );
    }

    message_content = message_content
        .push(widget::text(&msg.body).size(14))
        .push(widget::text(format_timestamp(msg.date)).size(11))
        .padding(spacing.space_s);

    let message_bubble = if is_sent {
        widget::container(message_content)
            .class(cosmic::theme::Container::Primary)
            .max_width(500.0)
    } else {
        widget::container(message_content)
            .class(cosmic::theme::Container::Transparent)
            .max_width(500.0)
    };

    if is_sent {
        widget::row()
            .push(widget::space::horizontal())
            .push(message_bubble)
            .width(Length::Fill)
            .into()
    } else {
        widget::row()
            .push(message_bubble)
            .width(Length::Fill)
            .into()
    }
}

fn view_message_input<'a>(
    app: &'a SmsWindow,
    spacing: &cosmic::cosmic_theme::Spacing,
) -> Element<'a, SmsMessage> {
    widget::row()
        .push(
            widget::text_input("Type a message...", &app.message_input)
                .on_input(SmsMessage::UpdateInput)
                .on_submit(|_| SmsMessage::SendMessage)
                .padding(spacing.space_s)
                .width(Length::Fill),
        )
        .push(widget::button::suggested("Send").on_press(SmsMessage::SendMessage))
        .spacing(spacing.space_xs)
        .padding(spacing.space_s)
        .align_y(Alignment::Center)
        .into()
}

// Helper functions

fn get_contact_name(app: &SmsWindow, phone_number: &str) -> Option<String> {
    app.contacts
        .iter()
        .find(|(contact_phone, _)| phone_numbers_match(phone_number, contact_phone))
        .map(|(_, name)| name.clone())
}

fn get_current_conversation_phone(app: &SmsWindow) -> Option<String> {
    let thread_id = app.selected_thread.as_ref()?;
    app.conversations
        .iter()
        .find(|c| c.thread_id == *thread_id)
        .map(|c| c.phone_number.clone())
}
