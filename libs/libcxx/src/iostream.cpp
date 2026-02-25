/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * Global stream objects: cout, cerr, cin.
 * These route to libc64's FILE* stdout/stderr/stdin.
 */

#include <iostream>

namespace std {

ostream cout(stdout);
ostream cerr(stderr);
istream cin(stdin);

} // namespace std
