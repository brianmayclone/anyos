use crate::sync::spinlock::Spinlock;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

const MAX_MSG_SIZE: usize = 256;
const MAX_QUEUE_DEPTH: usize = 64;

#[derive(Clone)]
pub struct Message {
    pub sender_pid: u32,
    pub msg_type: u32,
    pub data: Vec<u8>,
}

pub struct MessageQueue {
    inner: Spinlock<MessageQueueInner>,
}

struct MessageQueueInner {
    messages: VecDeque<Message>,
    max_depth: usize,
}

impl MessageQueue {
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

    pub fn message_count(&self) -> usize {
        let inner = self.inner.lock();
        inner.messages.len()
    }
}
