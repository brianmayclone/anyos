/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 */

#ifndef _TIME_H
#define _TIME_H

#include <stddef.h>

#ifndef _TIME_T_DEFINED
#define _TIME_T_DEFINED
typedef long time_t;
#endif
typedef long clock_t;

#define CLOCKS_PER_SEC 100

struct timespec {
    time_t tv_sec;
    long   tv_nsec;
};

struct tm {
    int tm_sec;
    int tm_min;
    int tm_hour;
    int tm_mday;
    int tm_mon;
    int tm_year;
    int tm_wday;
    int tm_yday;
    int tm_isdst;
};

#ifdef __cplusplus
extern "C" {
#endif

time_t time(time_t *tloc);
clock_t clock(void);
time_t mktime(struct tm *tm);
double difftime(time_t time1, time_t time0);
struct tm *localtime(const time_t *timer);
struct tm *gmtime(const time_t *timer);
size_t strftime(char *s, size_t max, const char *format, const struct tm *tm);
int nanosleep(const struct timespec *req, struct timespec *rem);
struct tm *gmtime_r(const time_t *timer, struct tm *result);
struct tm *localtime_r(const time_t *timer, struct tm *result);
time_t timegm(struct tm *tm);
char *ctime(const time_t *timer);
char *ctime_r(const time_t *timer, char *buf);
char *asctime(const struct tm *tm);
char *asctime_r(const struct tm *tm, char *buf);

/* Clock IDs for clock_gettime */
#define CLOCK_REALTIME  0
#define CLOCK_MONOTONIC 1
#define CLOCK_MONOTONIC_COARSE CLOCK_MONOTONIC
#define CLOCK_REALTIME_COARSE  CLOCK_REALTIME

typedef int clockid_t;

int clock_gettime(clockid_t clk_id, struct timespec *tp);

/* Timezone names */
extern char *tzname[2];
void tzset(void);

#ifdef __cplusplus
}
#endif

#endif
