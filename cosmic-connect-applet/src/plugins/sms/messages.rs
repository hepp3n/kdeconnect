// cosmic-connect-applet/src/plugins/sms/messages.rs
//! Message types for the SMS window application.

// #[allow(dead_code)] = Placeholder for code that will be used once features are fully integrated

#![allow(dead_code)]

use super::emoji::EmojiCategory;
use super::models::{ContactsMap, Conversation, ProtocolEvent};

/// All possible messages that the SMS window can receive and process.
#[derive(Debug, Clone)]
pub enum SmsMessage {
    // Conversation management
    LoadConversations,
    ConversationsLoaded(Vec<Conversation>),
    ContactsLoaded(ContactsMap),
    SelectThread(String),
    
    // Message input
    UpdateInput(String),
    UpdateSearch(String),
    SendMessage,
    #[allow(dead_code)] // May be used in future for manual refresh
    RefreshThread,
    
    // Window control
    #[allow(dead_code)] // Will be used when window close event is hooked up
    CloseWindow,
    
    // Protocol events (replacing D-Bus signals)
    ProtocolEventReceived(ProtocolEvent),
    
    // New chat dialog
    OpenNewChatDialog,
    CloseNewChatDialog,
    UpdateNewChatPhone(String),
    /// Select a contact for new chat: (phone, name)
    SelectContactForNewChat(String, String),
    /// Start a chat with a specific phone number
    StartChatWithNumber(String),
    
    // Emoji picker
    ToggleEmojiPicker,
    SelectEmojiCategory(EmojiCategory),
    InsertEmoji(String),
}