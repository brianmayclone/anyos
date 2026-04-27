//! Unit tests for IPC primitives that work without a running scheduler.
//!
//! Tested subsystems:
//!  - `ipc::message_queue` — `MessageQueue` is fully self-contained
//!  - `ipc::event_bus`     — `EventData` construction + channel lifecycle
//!  - `ipc::pipe`          — named pipe create/write/read/close
//!
//! Anonymous pipes and `shared_memory::map_into_current` require a running
//! scheduler and are intentionally omitted here.

use crate::ipc::{event_bus, message_queue, pipe};
use crate::kunit::{TestCase, TestContext, TestSuite};

pub static SUITE: TestSuite = TestSuite {
    name: "ipc",
    cases: &[
        // MessageQueue
        TestCase {
            name: "mq_new_empty",
            run: test_mq_new_empty,
        },
        TestCase {
            name: "mq_send_receive",
            run: test_mq_send_receive,
        },
        TestCase {
            name: "mq_ordering_fifo",
            run: test_mq_fifo,
        },
        TestCase {
            name: "mq_full_rejects",
            run: test_mq_full,
        },
        TestCase {
            name: "mq_oversized_rejects",
            run: test_mq_oversized,
        },
        TestCase {
            name: "mq_receive_empty",
            run: test_mq_receive_empty,
        },
        // EventData
        TestCase {
            name: "event_data_new",
            run: test_event_data_new,
        },
        TestCase {
            name: "event_data_type",
            run: test_event_data_type,
        },
        TestCase {
            name: "event_channel_lifecycle",
            run: test_channel_lifecycle,
        },
        TestCase {
            name: "event_channel_poll_empty",
            run: test_channel_poll_empty,
        },
        // Named pipe
        TestCase {
            name: "pipe_create_open",
            run: test_pipe_create,
        },
        TestCase {
            name: "pipe_write_read",
            run: test_pipe_write_read,
        },
        TestCase {
            name: "pipe_partial_read",
            run: test_pipe_partial_read,
        },
        TestCase {
            name: "pipe_clear",
            run: test_pipe_clear,
        },
        TestCase {
            name: "pipe_create_replaces",
            run: test_pipe_create_replaces,
        },
        TestCase {
            name: "pipe_atomic_backpressure",
            run: test_pipe_atomic_backpressure,
        },
        TestCase {
            name: "pipe_close",
            run: test_pipe_close,
        },
    ],
};

// ── MessageQueue ──────────────────────────────────────────────────────────────

fn make_msg(sender: u32, kind: u32, data: &[u8]) -> message_queue::Message {
    message_queue::Message {
        sender_pid: sender,
        msg_type: kind,
        data: alloc::vec::Vec::from(data),
    }
}

fn test_mq_new_empty(ctx: &mut TestContext) {
    let q = message_queue::MessageQueue::new();
    ctx.expect_false(q.has_messages(), "new queue has no messages");
    ctx.expect_eq(q.message_count(), 0usize, "new queue count == 0");
    ctx.expect_none(q.receive(), "receive on empty queue returns None");
}

fn test_mq_send_receive(ctx: &mut TestContext) {
    let q = message_queue::MessageQueue::new();
    let ok = q.send(make_msg(1, 42, b"hello"));
    ctx.expect_true(ok, "send returns true");
    ctx.expect_true(q.has_messages(), "has_messages after send");
    ctx.expect_eq(q.message_count(), 1usize, "count == 1");

    let msg = q.receive();
    if let Some(m) = ctx.expect_some(msg, "receive returns Some") {
        ctx.expect_eq(m.sender_pid, 1u32, "sender_pid");
        ctx.expect_eq(m.msg_type, 42u32, "msg_type");
        ctx.expect_eq(m.data.as_slice(), b"hello", "data payload");
    }
    ctx.expect_false(q.has_messages(), "empty after receive");
}

fn test_mq_fifo(ctx: &mut TestContext) {
    let q = message_queue::MessageQueue::new();
    for i in 0u32..4 {
        q.send(make_msg(i, i, &[i as u8]));
    }
    ctx.expect_eq(q.message_count(), 4usize, "count == 4");

    for i in 0u32..4 {
        let m = q.receive().expect("should have message");
        ctx.expect_eq(m.sender_pid, i, "FIFO order preserved");
    }
}

fn test_mq_full(ctx: &mut TestContext) {
    let q = message_queue::MessageQueue::new();
    // MAX_QUEUE_DEPTH = 64
    for i in 0..64u32 {
        let ok = q.send(make_msg(i, 0, b"x"));
        ctx.expect_true(ok, "send within depth succeeds");
    }
    // 65th send must be rejected.
    let rejected = q.send(make_msg(99, 0, b"overflow"));
    ctx.expect_false(rejected, "send beyond MAX_QUEUE_DEPTH returns false");
    ctx.expect_eq(q.message_count(), 64usize, "count stays at 64");
}

fn test_mq_oversized(ctx: &mut TestContext) {
    let q = message_queue::MessageQueue::new();
    // MAX_MSG_SIZE = 256
    let big = alloc::vec![0xAAu8; 257];
    let ok = q.send(message_queue::Message {
        sender_pid: 1,
        msg_type: 0,
        data: big,
    });
    ctx.expect_false(ok, "oversized message (>256 bytes) rejected");
    ctx.expect_false(q.has_messages(), "queue still empty after rejection");
}

fn test_mq_receive_empty(ctx: &mut TestContext) {
    let q = message_queue::MessageQueue::new();
    ctx.expect_none(q.receive(), "receive on empty queue is None");
    // Must not panic on repeated receives.
    for _ in 0..3 {
        let _ = q.receive();
    }
    ctx.expect_true(true, "repeated receives on empty queue do not panic");
}

// ── EventData ─────────────────────────────────────────────────────────────────

fn test_event_data_new(ctx: &mut TestContext) {
    let e = event_bus::EventData::new(event_bus::EVT_BOOT_COMPLETE, 1, 2, 3, 4);
    ctx.expect_eq(
        e.words[0],
        event_bus::EVT_BOOT_COMPLETE,
        "words[0] == event_type",
    );
    ctx.expect_eq(e.words[1], 1u32, "words[1] == p1");
    ctx.expect_eq(e.words[2], 2u32, "words[2] == p2");
    ctx.expect_eq(e.words[3], 3u32, "words[3] == p3");
    ctx.expect_eq(e.words[4], 4u32, "words[4] == p4");
}

fn test_event_data_type(ctx: &mut TestContext) {
    let e = event_bus::EventData::new(event_bus::EVT_DEVICE_ATTACHED, 0, 0, 0, 0);
    ctx.expect_eq(
        e.event_type(),
        event_bus::EVT_DEVICE_ATTACHED,
        "event_type() accessor",
    );

    let custom = event_bus::EventData::new(event_bus::EVT_CUSTOM, 99, 0, 0, 0);
    ctx.expect_eq(custom.event_type(), event_bus::EVT_CUSTOM, "EVT_CUSTOM");
}

fn test_channel_lifecycle(ctx: &mut TestContext) {
    // Create a channel, subscribe, check, destroy.
    let ch = event_bus::channel_create(b"kunit_test_ch");
    ctx.expect_ne(ch, 0u32, "channel_create returns nonzero id");

    let sub = event_bus::channel_subscribe(ch, 0xFFFF_FFFF);
    ctx.expect_ne(sub, 0u32, "channel_subscribe returns nonzero sub_id");

    ctx.expect_false(
        event_bus::channel_has_events(ch, sub),
        "no events after subscribe",
    );

    event_bus::channel_unsubscribe(ch, sub);
    event_bus::channel_destroy(ch);
    ctx.expect_true(true, "channel lifecycle completed without panic");
}

fn test_channel_poll_empty(ctx: &mut TestContext) {
    let ch = event_bus::channel_create(b"kunit_poll_empty");
    let sub = event_bus::channel_subscribe(ch, 0xFFFF_FFFF);

    let result = event_bus::channel_poll(ch, sub);
    ctx.expect_none(result, "poll on empty channel returns None");

    event_bus::channel_unsubscribe(ch, sub);
    event_bus::channel_destroy(ch);
}

// ── Named pipe ────────────────────────────────────────────────────────────────

fn test_pipe_create(ctx: &mut TestContext) {
    let id = pipe::create("kunit_pipe_1");
    ctx.expect_ne(id, 0u32, "create returns nonzero id");

    let id2 = pipe::open("kunit_pipe_1");
    ctx.expect_eq(id, id2, "open returns same id as create");

    pipe::close(id);
}

fn test_pipe_write_read(ctx: &mut TestContext) {
    let id = pipe::create("kunit_pipe_rw");
    let payload = b"hello from kunit";

    let written = pipe::write(id, payload);
    ctx.expect_eq(written as usize, payload.len(), "write returns byte count");

    let mut buf = [0u8; 64];
    let read = pipe::read(id, &mut buf) as usize;
    ctx.expect_eq(read, payload.len(), "read returns same byte count");
    ctx.expect_eq(&buf[..read], payload, "read data matches written data");

    pipe::close(id);
}

fn test_pipe_partial_read(ctx: &mut TestContext) {
    let id = pipe::create("kunit_pipe_partial");
    pipe::write(id, b"abcdefgh"); // 8 bytes

    // Read only 4 bytes at a time.
    let mut buf = [0u8; 4];
    let r1 = pipe::read(id, &mut buf) as usize;
    ctx.expect_eq(r1, 4, "first partial read == 4 bytes");
    ctx.expect_eq(&buf, b"abcd", "first 4 bytes correct");

    let r2 = pipe::read(id, &mut buf) as usize;
    ctx.expect_eq(r2, 4, "second partial read == 4 bytes");
    ctx.expect_eq(&buf, b"efgh", "second 4 bytes correct");

    pipe::close(id);
}

fn test_pipe_clear(ctx: &mut TestContext) {
    let id = pipe::create("kunit_pipe_clear");
    pipe::write(id, b"data to clear");
    pipe::clear(id);

    let mut buf = [0u8; 64];
    let n = pipe::read(id, &mut buf);
    ctx.expect_eq(n, 0u32, "read returns 0 after clear");

    pipe::close(id);
}

fn test_pipe_create_replaces(ctx: &mut TestContext) {
    let old_id = pipe::create("kunit_pipe_replace");
    pipe::write(old_id, b"stale");

    let new_id = pipe::create("kunit_pipe_replace");
    ctx.expect_ne(new_id, 0u32, "replacement create returns nonzero id");
    ctx.expect_ne(new_id, old_id, "replacement create returns a fresh id");

    let opened = pipe::open("kunit_pipe_replace");
    ctx.expect_eq(opened, new_id, "open returns replacement id");

    let mut buf = [0u8; 16];
    let stale_read = pipe::read(old_id, &mut buf);
    ctx.expect_eq(stale_read, u32::MAX, "old id is no longer readable");

    let empty_read = pipe::read(new_id, &mut buf);
    ctx.expect_eq(empty_read, 0u32, "replacement starts with an empty buffer");

    pipe::close(new_id);
}

fn test_pipe_atomic_backpressure(ctx: &mut TestContext) {
    let id = pipe::create("kunit_pipe_backpressure");
    let chunk = [b'x'; 4096];
    let mut total = 0usize;

    loop {
        let written = pipe::write(id, &chunk);
        if written == 0 {
            break;
        }
        ctx.expect_eq(written as usize, chunk.len(), "small writes stay atomic");
        total += written as usize;
        if total > 512 * 1024 {
            ctx.expect_true(false, "bounded pipe accepted too much data");
            break;
        }
    }

    ctx.expect_true(total >= 64 * 1024, "pipe accepts a useful burst");
    let mut free_buf = [0u8; 4096];
    let read = pipe::read(id, &mut free_buf);
    ctx.expect_eq(read as usize, free_buf.len(), "read frees buffer space");

    let retry = pipe::write(id, b"abc");
    ctx.expect_eq(retry, 3u32, "small write succeeds after space is freed");

    pipe::close(id);
}

fn test_pipe_close(ctx: &mut TestContext) {
    let id = pipe::create("kunit_pipe_close");
    pipe::close(id);
    // Opening a closed pipe should return 0 (not found).
    let reopened = pipe::open("kunit_pipe_close");
    ctx.expect_eq(reopened, 0u32, "open returns 0 after close");
}
