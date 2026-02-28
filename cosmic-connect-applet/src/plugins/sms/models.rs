// cosmic-connect-applet/src/plugins/sms/models.rs
//! Data models for the SMS feature.

// #[allow(dead_code)] = Placeholder for code that will be used once features are fully integrated

#![allow(dead_code)]

use std::collections::HashMap;

/// Represents an SMS conversation thread.
#[derive(Debug, Clone)]
pub struct Conversation {
    pub thread_id: String,
    pub contact_name: String,
    pub phone_number: String,
    pub last_message: String,
    pub timestamp: i64,
    #[allow(dead_code)] // Used for future read/unread tracking
    pub unread: bool,
}

/// Represents an individual SMS message.
#[derive(Debug, Clone)]
pub struct Message {
    pub id: String,
    pub thread_id: String,
    pub body: String,
    pub address: String,
    pub date: i64,
    /// Message type: 1 = received, 2 = sent
    pub type_: i32,
    #[allow(dead_code)] // Used for future read receipt tracking
    pub read: bool,
}

impl Message {
    /// Returns true if this is a sent message.
    #[inline]
    pub fn is_sent(&self) -> bool {
        self.type_ == 2
    }
}

/// Events received from the native protocol adapter.
#[derive(Debug, Clone)]
pub enum ProtocolEvent {
    MessageReceived(Message),
    ConversationsReceived(Vec<Conversation>),
    #[allow(dead_code)] // Used for error handling in event processing
    Error(String),
}

/// Type alias for contacts map (phone_number -> contact_name).
pub type ContactsMap = HashMap<String, String>;