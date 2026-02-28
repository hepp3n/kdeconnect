// cosmic-connect-applet/src/plugins/sms/app.rs
use cosmic::{
    app::Core,
    iced::{Length, Subscription},
    iced_futures::futures::StreamExt,
    widget, Application, ApplicationExt, Element, Task, Action,
};
use async_stream::stream;
use std::collections::HashMap;

use super::dbus;
use super::models::{Conversation, Message, ProtocolEvent};
use super::utils;
use super::views;

#[allow(dead_code)]
pub fn run(device_id: String, device_name: String) -> cosmic::iced::Result {
    cosmic::app::run::<SmsWindow>(
        cosmic::app::Settings::default(),
        (device_id, device_name),
    )
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
    const APP_ID: &'static str = "com.system76.CosmicConnectSms";

    fn core(&self) -> &Core { &self.core }
    fn core_mut(&mut self) -> &mut Core { &mut self.core }

    fn init(core: Core, flags: Self::Flags) -> (Self, Task<Action<Self::Message>>) {
        let (device_id, device_name) = flags;
        eprintln!("[SMS-APP] init() device_id={}", device_id);

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

        Subscription::run_with_id(
            format!("sms-{}", device_id),
            stream! {
                eprintln!("[SMS-SUB] stream started for device={}", device_id);

                if let Err(e) = dbus::initialize().await {
                    eprintln!("[SMS-SUB] init FAILED: {:?}", e);
                    std::future::pending::<()>().await;
                    return;
                }

                eprintln!("[SMS-SUB] D-Bus init OK, requesting conversations");
                tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                dbus::fetch_conversations(&device_id).await;

                let Some(client) = dbus::get_client().await else {
                    eprintln!("[SMS-SUB] no client, stream idle");
                    std::future::pending::<()>().await;
                    return;
                };

                eprintln!("[SMS-SUB] entering event loop");

                loop {
                    eprintln!("[SMS-SUB] subscribing to events");
                    let mut event_stream = client.listen_for_events().await;

                    while let Some(event) = event_stream.next().await {
                        use kdeconnect_dbus_client::ServiceEvent;
                        if let ServiceEvent::SmsMessagesReceived(json) = event {
                            eprintln!("[SMS-SUB] SmsMessagesReceived len={}", json.len());
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
                    }

                    eprintln!("[SMS-SUB] stream ended, reconnecting in 1s...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }
        )
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
                eprintln!("[SMS-APP] ConversationsLoaded: {}", conversations.len());
                self.conversations = conversations;
                self.update_conversation_names();
            }
            SmsMessage::ContactsLoaded(contacts) => {
                self.contacts = contacts;
                self.update_conversation_names();
            }
            SmsMessage::SelectThread(thread_id) => {
                eprintln!("[SMS-APP] SelectThread: {}", thread_id);
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
                if self.message_input.trim().is_empty() { return Task::none(); }
                let Some(thread_id) = self.selected_thread.clone() else { return Task::none(); };
                let Some(conv) = self.conversations.iter().find(|c| c.thread_id == thread_id) else { return Task::none(); };

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
                eprintln!("[SMS-APP] ProtocolEventReceived: {:?}", std::mem::discriminant(&event));
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
                    self.conversations.insert(0, Conversation {
                        thread_id: thread_id.clone(),
                        phone_number: phone,
                        contact_name: String::new(),
                        last_message: String::new(),
                        timestamp: utils::now_millis(),
                        unread: false,
                    });
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
                eprintln!("[SMS-APP] ConversationsReceived: {} conversations", conversations.len());

                // Merge: preserve new_* threads, update/add real ones
                let mut merged = self.conversations.clone();
                for incoming in &conversations {
                    if incoming.thread_id.starts_with("new_") { continue; }

                    // Check if we have a new_* placeholder for this phone number
                    if let Some(pos) = merged.iter().position(|c| {
                        c.thread_id.starts_with("new_")
                            && super::utils::phone_numbers_match(&c.phone_number, &incoming.phone_number)
                    }) {
                        merged[pos] = incoming.clone();
                    } else if let Some(existing) = merged.iter_mut().find(|c| c.thread_id == incoming.thread_id) {
                        *existing = incoming.clone();
                    } else {
                        merged.push(incoming.clone());
                    }
                }

                // Remove any new_* threads that now have a real counterpart
                merged.retain(|c| {
                    if !c.thread_id.starts_with("new_") { return true; }
                    !conversations.iter().any(|r| super::utils::phone_numbers_match(&r.phone_number, &c.phone_number))
                });

                self.conversations = merged;
                self.update_conversation_names();

                // If selected thread was new_*, update to real thread_id
                if let Some(sel) = &self.selected_thread.clone() {
                    if sel.starts_with("new_") {
                        if let Some(conv) = self.conversations.iter().find(|c| !c.thread_id.starts_with("new_")) {
                            self.selected_thread = Some(conv.thread_id.clone());
                        }
                    }
                }
            }
            ProtocolEvent::MessageReceived(message) => {
                eprintln!("[SMS-APP] MessageReceived thread={}", message.thread_id);
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
                            m.id.starts_with("sending_")
                            && m.type_ == 2
                            && m.body == message.body
                        }) {
                            existing.id = message.id.clone();
                            existing.thread_id = message.thread_id.clone();
                            existing.date = message.date;
                        }
                    }
                    self.messages.sort_by_key(|m| m.date);
                }

                if let Some(conv) = self.conversations.iter_mut()
                    .find(|c| c.thread_id == message.thread_id)
                {
                    conv.last_message = message.body;
                    conv.timestamp = message.date;
                }
                self.conversations.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
            }
            ProtocolEvent::Error(e) => eprintln!("[SMS-APP] error: {}", e),
        }
    }

    fn update_conversation_names(&mut self) {
        for conv in &mut self.conversations {
            if let Some(name) = self.contacts.get(&conv.phone_number) {
                conv.contact_name = name.clone();
            }
        }
    }
}
