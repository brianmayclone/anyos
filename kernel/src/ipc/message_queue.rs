//! Bounded message queue for inter-process communication.
//!
//! Each queue holds up to [`MAX_QUEUE_DEPTH`] messages of at most [`MAX_MSG_SIZE`] bytes.
//! Sending is non-blocking (returns false if full); receiving is non-blocking (returns None
//! if empty). Thread-safe via an internal spinlock.

use crate::sync::spinlock::Spinlock;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

/// Maximum payload size for a single message in bytes.
const MAX_MSG_SIZE: usize = 256;
/// Maximum number of messages that can be queued before sends are rejected.
const MAX_QUEUE_DEPTH: usize = 64;

/// A single message in the queue, carrying sender identity, type tag, and payload.
#[derive(Clone)]
pub struct Message {
    /// PID of the sending process.
    pub sender_pid: u32,
    /// Application-defined message type identifier.
    pub msg_type: u32,
    /// Variable-length payload (up to [`MAX_MSG_SIZE`] bytes).
    pub data: Vec<u8>,
}

/// Thread-safe bounded message queue protected by a spinlock.
pub struct MessageQueue {
    inner: Spinlock<MessageQueueInner>,
}

struct MessageQueueInner {
    messages: VecDeque<Message>,
    max_depth: usize,
}

impl MessageQueue {
    /// Create a new empty message queue.
    pub fn new() -> Self {
        MessageQueue {
            inner: Spinlock::new(MessageQueueInner {
                messages: VecDeque::new(),
                max_depth: MAX_QUEUE_DEPTH,
            }),
        }
    }

    /// Send a message. Returns false if queue is full.
    pub fn send(&self, msg: Message) -> bool {
        let mut inner = self.inner.lock();
        if inner.messages.len() >= inner.max_depth {
            return false;
        }
        if msg.data.len() > MAX_MSG_SIZE {
            return false;
        }
        inner.messages.push_back(msg);
        true
    }

    /// Receive a message (non-blocking). Returns None if empty.
    pub fn receive(&self) -> Option<Message> {
        let mut inner = self.inner.lock();
        inner.messages.pop_front()
    }

    /// Check if there are pending messages.
    pub fn has_messages(&self) -> bool {
        let inner = self.inner.lock();
        !inner.messages.is_empty()
    }

    /// Return the number of messages currently in the queue.
    pub fn message_count(&self) -> usize {
        let inner = self.inner.lock();
        inner.messages.len()
    }
}
