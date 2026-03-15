use async_stream::stream;
use cosmic::{
    Action, Application, ApplicationExt, Element, Task,
    app::Core,
    iced::{Length, Subscription},
    iced_futures::futures::StreamExt,
    widget,
};
use std::collections::HashMap;
use tracing::{debug, error, info, warn};

use super::dbus;
use super::models::{Conversation, Message, ProtocolEvent};
use super::utils;
use super::views;

#[allow(dead_code)]
pub fn run(device_id: String, device_name: String) -> cosmic::iced::Result {
    cosmic::app::run::<SmsWindow>(cosmic::app::Settings::default(), (device_id, device_name))
}

#[derive(Clone, Debug)]
pub enum SmsMessage {
    #[allow(dead_code)]
    LoadConversations,
    #[allow(dead_code)]
    ConversationsLoaded(Vec<Conversation>),
    #[allow(dead_code)]
    ContactsLoaded(HashMap<String, String>),
    SelectThread(String),
    UpdateInput(String),
    UpdateSearch(String),
    SendMessage,
    RefreshThread,
    #[allow(dead_code)]
    CloseWindow,
    ProtocolEventReceived(ProtocolEvent),
    OpenNewChatDialog,
    CloseNewChatDialog,
    UpdateNewChatPhone(String),
    SelectContactForNewChat(String, String),
    CreateNewChat,
}

pub struct SmsWindow {
    core: Core,
    pub device_id: String,
    #[allow(dead_code)]
    pub device_name: String,
    pub conversations: Vec<Conversation>,
    pub contacts: HashMap<String, String>,
    pub selected_thread: Option<String>,
    pub messages: Vec<Message>,
    pub message_input: String,
    pub search_query: String,
    pub show_new_chat_dialog: bool,
    pub new_chat_phone_input: String,
}

impl Application for SmsWindow {
    type Executor = cosmic::executor::Default;
    type Flags = (String, String);
    type Message = SmsMessage;
    const APP_ID: &'static str = "io.github.hepp3n.kdeconnect.sms";

    fn core(&self) -> &Core {
        &self.core
    }
    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, flags: Self::Flags) -> (Self, Task<Action<Self::Message>>) {
        let (device_id, device_name) = flags;
        info!("SMS window init device_id={}", device_id);

        let mut app = Self {
            core,
            device_id: device_id.clone(),
            device_name: device_name.clone(),
            conversations: Vec::new(),
            contacts: HashMap::new(),
            selected_thread: None,
            messages: Vec::new(),
            message_input: String::new(),
            search_query: String::new(),
            show_new_chat_dialog: false,
            new_chat_phone_input: String::new(),
        };

        let title = format!("SMS - {}", device_name);
        let title_task = app.set_window_title(title, app.core.main_window_id().unwrap());

        (app, title_task)
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        let device_id = self.device_id.clone();

        Subscription::run_with(device_id, |device_id| {
            let device_id = device_id.clone();
            stream! {
                info!("SMS event stream started for device={}", device_id);

                if let Err(e) = dbus::initialize().await {
                    error!("SMS D-Bus init failed: {:?}", e);
                    std::future::pending::<()>().await;
                    return;
                }

                let Some(client) = dbus::get_client().await else {
                    warn!("SMS D-Bus no client available, stream idle");
                    std::future::pending::<()>().await;
                    return;
                };

                debug!("SMS event loop entering");

                let cached_contacts = dbus::get_cached_contacts(&device_id).await;
                if !cached_contacts.is_empty() {
                    debug!("yielding {} cached contacts at startup", cached_contacts.len());
                    yield SmsMessage::ContactsLoaded(cached_contacts);
                }

                if let Some(cached_json) = dbus::get_cached_sms(&device_id).await {
                    debug!("yielding cached SMS at startup");
                    let (messages, conversations) = dbus::parse_sms_messages(&cached_json);
                    for msg in messages {
                        yield SmsMessage::ProtocolEventReceived(ProtocolEvent::MessageReceived(msg));
                    }
                    yield SmsMessage::ProtocolEventReceived(ProtocolEvent::ConversationsReceived(conversations));
                }

                loop {
                    debug!("SMS subscribing to events");
                    let mut event_stream = client.listen_for_events().await;

                    // Subscribe FIRST, then request — contacts response is a
                    // fire-and-forget D-Bus signal; if we request before subscribing
                    // the signal arrives while nobody is listening and is lost.
                    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                    dbus::fetch_conversations(&device_id).await;
                    dbus::fetch_contacts(&device_id).await;

                    while let Some(event) = event_stream.next().await {
                        use kdeconnect_dbus_client::ServiceEvent;
                        match event {
                            ServiceEvent::SmsMessagesReceived(json) => {
                                debug!("SmsMessagesReceived len={}", json.len());
                                let (messages, conversations) = dbus::parse_sms_messages(&json);
                                for msg in messages {
                                    yield SmsMessage::ProtocolEventReceived(
                                        ProtocolEvent::MessageReceived(msg)
                                    );
                                }
                                yield SmsMessage::ProtocolEventReceived(
                                    ProtocolEvent::ConversationsReceived(conversations)
                                );
                            }
                            ServiceEvent::ContactsReceived(contacts) => {
                                debug!("ContactsReceived {} entries", contacts.len());
                                yield SmsMessage::ContactsLoaded(contacts);
                            }
                            _ => {}
                        }
                    }

                    warn!("SMS event stream ended, reconnecting in 1s");
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }
        })
    }

    fn update(&mut self, message: Self::Message) -> Task<Action<Self::Message>> {
        match message {
            SmsMessage::LoadConversations => {
                let device_id = self.device_id.clone();
                return cosmic::task::future(async move {
                    dbus::fetch_conversations(&device_id).await;
                    Action::App(SmsMessage::RefreshThread)
                });
            }
            SmsMessage::ConversationsLoaded(conversations) => {
                debug!("ConversationsLoaded: {}", conversations.len());
                self.conversations = conversations;
                self.update_conversation_names();
            }
            SmsMessage::ContactsLoaded(contacts) => {
                debug!("ContactsLoaded: {} contacts", contacts.len());
                self.contacts = contacts;
                self.update_conversation_names();
            }
            SmsMessage::SelectThread(thread_id) => {
                debug!("SelectThread: {}", thread_id);
                self.selected_thread = Some(thread_id.clone());
                self.messages.clear();
                let device_id = self.device_id.clone();
                return cosmic::task::future(async move {
                    dbus::request_conversation_messages(&device_id, &thread_id).await;
                    Action::App(SmsMessage::RefreshThread)
                });
            }
            SmsMessage::UpdateInput(input) => {
                self.message_input = input;
            }
            SmsMessage::UpdateSearch(query) => {
                self.search_query = query;
            }
            SmsMessage::SendMessage => {
                if self.message_input.trim().is_empty() {
                    return Task::none();
                }
                let Some(thread_id) = self.selected_thread.clone() else {
                    return Task::none();
                };
                let Some(conv) = self.conversations.iter().find(|c| c.thread_id == thread_id)
                else {
                    return Task::none();
                };

                let device_id = self.device_id.clone();
                let phone = conv.phone_number.clone();
                let text = self.message_input.clone();
                let now = utils::now_millis();

                self.messages.push(Message {
                    id: format!("sending_{}", now),
                    thread_id: thread_id.clone(),
                    body: text.clone(),
                    address: phone.clone(),
                    date: now,
                    type_: 2,
                    read: true,
                });
                self.messages.sort_by_key(|m| m.date);
                self.message_input.clear();

                return cosmic::task::future(async move {
                    dbus::send_sms(&device_id, &phone, &text).await;
                    Action::App(SmsMessage::RefreshThread)
                });
            }
            SmsMessage::RefreshThread => {}
            SmsMessage::ProtocolEventReceived(event) => {
                debug!("ProtocolEventReceived: {:?}", std::mem::discriminant(&event));
                self.handle_protocol_event(event);
            }
            SmsMessage::OpenNewChatDialog => {
                self.show_new_chat_dialog = true;
            }
            SmsMessage::CloseNewChatDialog => {
                self.show_new_chat_dialog = false;
                self.new_chat_phone_input.clear();
            }
            SmsMessage::UpdateNewChatPhone(phone) => {
                self.new_chat_phone_input = phone;
            }
            SmsMessage::SelectContactForNewChat(phone, _name) => {
                self.new_chat_phone_input = phone;
            }
            SmsMessage::CreateNewChat => {
                let phone = self.new_chat_phone_input.trim().to_string();
                if !phone.is_empty() {
                    let thread_id = format!("new_{}", utils::now_millis());
                    self.conversations.insert(
                        0,
                        Conversation {
                            thread_id: thread_id.clone(),
                            phone_number: phone,
                            contact_name: String::new(),
                            last_message: String::new(),
                            timestamp: utils::now_millis(),
                            unread: false,
                        },
                    );
                    self.show_new_chat_dialog = false;
                    self.new_chat_phone_input.clear();
                    return cosmic::task::message(Action::App(SmsMessage::SelectThread(thread_id)));
                }
            }
            SmsMessage::CloseWindow => std::process::exit(0),
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let content = if self.show_new_chat_dialog {
            views::view_new_chat_dialog(self)
        } else {
            views::view_main(self)
        };
        widget::container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

impl SmsWindow {
    fn handle_protocol_event(&mut self, event: ProtocolEvent) {
        match event {
            ProtocolEvent::ConversationsReceived(conversations) => {
                debug!("ConversationsReceived: {} conversations", conversations.len());

                // Capture selected new_* phone BEFORE we mutate merged
                let pending_new_phone: Option<String> = self
                    .selected_thread
                    .as_ref()
                    .filter(|t| t.starts_with("new_"))
                    .and_then(|sel| self.conversations.iter().find(|c| c.thread_id == *sel))
                    .map(|c| c.phone_number.clone());

                let mut merged = self.conversations.clone();
                for incoming in &conversations {
                    if incoming.thread_id.starts_with("new_") {
                        continue;
                    }

                    if let Some(pos) = merged.iter().position(|c| {
                        c.thread_id.starts_with("new_")
                            && super::utils::phone_numbers_match(
                                &c.phone_number,
                                &incoming.phone_number,
                            )
                    }) {
                        merged[pos] = incoming.clone();
                    } else if let Some(existing) = merged
                        .iter_mut()
                        .find(|c| c.thread_id == incoming.thread_id)
                    {
                        *existing = incoming.clone();
                    } else {
                        merged.push(incoming.clone());
                    }
                }

                merged.retain(|c| {
                    if !c.thread_id.starts_with("new_") {
                        return true;
                    }
                    !conversations.iter().any(|r| {
                        super::utils::phone_numbers_match(&r.phone_number, &c.phone_number)
                    })
                });

                self.conversations = merged;
                self.update_conversation_names();

                // If we had a new_* selected, find its real thread by phone number now
                if let Some(phone) = pending_new_phone {
                    if let Some(real) = self.conversations.iter().find(|c| {
                        !c.thread_id.starts_with("new_")
                            && super::utils::phone_numbers_match(&c.phone_number, &phone)
                    }) {
                        self.selected_thread = Some(real.thread_id.clone());
                    }
                }
            }
            ProtocolEvent::MessageReceived(message) => {
                debug!("MessageReceived thread={}", message.thread_id);
                let is_selected = self.selected_thread.as_deref() == Some(&message.thread_id);

                if is_selected {
                    let already_exists = self.messages.iter().any(|m| {
                        m.id == message.id
                            || (m.id.starts_with("sending_")
                                && m.type_ == 2
                                && message.type_ == 2
                                && m.body == message.body
                                && (m.date - message.date).abs() < 300_000)
                    });

                    if !already_exists {
                        self.messages.push(message.clone());
                    } else {
                        if let Some(existing) = self.messages.iter_mut().find(|m| {
                            m.id.starts_with("sending_") && m.type_ == 2 && m.body == message.body
                        }) {
                            existing.id = message.id.clone();
                            existing.thread_id = message.thread_id.clone();
                            existing.date = message.date;
                        }
                    }
                    self.messages.sort_by_key(|m| m.date);
                }

                if let Some(conv) = self
                    .conversations
                    .iter_mut()
                    .find(|c| c.thread_id == message.thread_id)
                {
                    conv.last_message = message.body;
                    conv.timestamp = message.date;
                }
                self.conversations
                    .sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            }
            ProtocolEvent::Error(e) => error!("SMS protocol error: {}", e),
        }
    }

    fn update_conversation_names(&mut self) {
        for conv in &mut self.conversations {
            if let Some(name) = self
                .contacts
                .iter()
                .find(|(phone, _)| super::utils::phone_numbers_match(phone, &conv.phone_number))
                .map(|(_, name)| name.clone())
            {
                conv.contact_name = name;
            }
        }
    }
}
