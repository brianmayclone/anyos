/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

#include <signal.h>
#include <stdlib.h>

static sighandler_t _handlers[16] = { 0 };

sighandler_t signal(int signum, sighandler_t handler) {
    if (signum < 0 || signum >= 16) return SIG_ERR;
    sighandler_t old = _handlers[signum];
    _handlers[signum] = handler;
    return old;
}

int raise(int sig) {
    if (sig < 0 || sig >= 16) return -1;
    if (_handlers[sig] && _handlers[sig] != SIG_DFL && _handlers[sig] != SIG_IGN) {
        _handlers[sig](sig);
    } else if (_handlers[sig] != SIG_IGN) {
        abort();
    }
    return 0;
}
