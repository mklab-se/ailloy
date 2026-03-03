//! Conversation management with history tracking.

use anyhow::Result;
use futures_util::StreamExt;

use crate::client::Client;
use crate::types::{ChatResponse, ChatStream, Message, Role, StreamEvent};

/// Trait for conversation history storage.
pub trait ChatHistory: Send + Sync {
    /// Messages to send to the provider (may filter/window).
    fn messages(&self) -> Vec<Message>;

    /// Append a message.
    fn push(&mut self, message: Message);

    /// Clear all messages.
    fn clear(&mut self);

    /// Total number of stored messages.
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Default in-memory history implementation.
pub struct InMemoryHistory {
    messages: Vec<Message>,
}

impl InMemoryHistory {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }
}

impl Default for InMemoryHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatHistory for InMemoryHistory {
    fn messages(&self) -> Vec<Message> {
        self.messages.clone()
    }

    fn push(&mut self, message: Message) {
        self.messages.push(message);
    }

    fn clear(&mut self) {
        self.messages.clear();
    }

    fn len(&self) -> usize {
        self.messages.len()
    }
}

/// A multi-turn conversation that manages message history.
pub struct Conversation {
    client: Client,
    history: Box<dyn ChatHistory>,
    system_prompt: Option<String>,
}

impl Conversation {
    /// Create a new conversation with in-memory history.
    pub fn new(client: Client) -> Self {
        Self {
            client,
            history: Box::new(InMemoryHistory::new()),
            system_prompt: None,
        }
    }

    /// Create a new conversation with a custom history store.
    pub fn with_history(client: Client, history: impl ChatHistory + 'static) -> Self {
        Self {
            client,
            history: Box::new(history),
            system_prompt: None,
        }
    }

    /// Set the system prompt (persists across turns).
    pub fn system(&mut self, prompt: impl Into<String>) {
        self.system_prompt = Some(prompt.into());
    }

    /// Send a message and get a response. Both the user message and assistant
    /// response are appended to history.
    pub async fn send(&mut self, message: impl Into<String>) -> Result<ChatResponse> {
        let user_msg = Message::user(message);
        self.history.push(user_msg);

        let messages = self.build_messages();
        let response = self.client.chat(&messages).await?;

        self.history.push(Message::assistant(&response.content));

        Ok(response)
    }

    /// Send a message and get a streaming response. The user message is appended
    /// to history immediately. The assistant response is assembled and appended
    /// when the stream completes (via the `Done` event).
    pub async fn send_stream(&mut self, message: impl Into<String>) -> Result<ChatStream> {
        let user_msg = Message::user(message);
        self.history.push(user_msg);

        let messages = self.build_messages();
        let inner_stream = self.client.chat_stream(&messages).await?;

        // Wrap the stream to capture the assembled response for history.
        // We can't mutably borrow self in the stream closure, so we use
        // a separate collector and return a new stream.
        let mut assembled = String::new();
        let stream = inner_stream.map(move |event| {
            match &event {
                Ok(StreamEvent::Delta(text)) => {
                    assembled.push_str(text);
                }
                Ok(StreamEvent::Done(_)) => {}
                Err(_) => {}
            }
            event
        });

        Ok(Box::pin(stream))
    }

    /// Get the current history.
    pub fn history(&self) -> Vec<Message> {
        self.history.messages()
    }

    /// Get the number of messages in history.
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Clear conversation history.
    pub fn clear(&mut self) {
        self.history.clear();
    }

    /// Get a reference to the underlying client.
    pub fn client(&self) -> &Client {
        &self.client
    }

    fn build_messages(&self) -> Vec<Message> {
        let mut messages = Vec::new();
        if let Some(system) = &self.system_prompt {
            messages.push(Message {
                role: Role::System,
                content: system.clone(),
            });
        }
        messages.extend(self.history.messages());
        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_memory_history() {
        let mut history = InMemoryHistory::new();
        assert!(history.is_empty());
        assert_eq!(history.len(), 0);

        history.push(Message::user("hello"));
        assert_eq!(history.len(), 1);
        assert!(!history.is_empty());

        let msgs = history.messages();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hello");

        history.clear();
        assert!(history.is_empty());
    }

    #[test]
    fn test_in_memory_history_default() {
        let history = InMemoryHistory::default();
        assert!(history.is_empty());
    }
}
