/*
 * Copyright (c) 2024-2026 Christian Moeller
 * SPDX-License-Identifier: MIT
 *
 * libc64 — x86_64 time functions.
 */

#include <time.h>
#include <sys/time.h>

#include <sys/syscall.h>

extern long _syscall(long num, long a1, long a2, long a3, long a4, long a5);

static struct tm _tm;

time_t time(time_t *tloc) {
    unsigned char buf[8];
    _syscall(SYS_TIME, (long)buf, 0, 0, 0, 0);
    /* Return uptime ticks as "time" since we lack epoch */
    time_t t = (time_t)_syscall(SYS_UPTIME, 0, 0, 0, 0, 0);
    if (tloc) *tloc = t;
    return t;
}

clock_t clock(void) {
    return (clock_t)_syscall(SYS_UPTIME, 0, 0, 0, 0, 0);
}

struct tm *localtime(const time_t *timer) {
    /* Fetch real date/time from RTC */
    unsigned char buf[8];
    _syscall(SYS_TIME, (long)buf, 0, 0, 0, 0);
    _tm.tm_year = (buf[0] | (buf[1] << 8)) - 1900;
    _tm.tm_mon = buf[2] - 1;
    _tm.tm_mday = buf[3];
    _tm.tm_hour = buf[4];
    _tm.tm_min = buf[5];
    _tm.tm_sec = buf[6];
    _tm.tm_wday = 0;
    _tm.tm_yday = 0;
    _tm.tm_isdst = 0;
    return &_tm;
}

struct tm *gmtime(const time_t *timer) {
    return localtime(timer);
}

struct tm *localtime_r(const time_t *timer, struct tm *result) {
    struct tm *t = localtime(timer);
    if (t && result) *result = *t;
    return result;
}

struct tm *gmtime_r(const time_t *timer, struct tm *result) {
    struct tm *t = gmtime(timer);
    if (t && result) *result = *t;
    return result;
}

static int _snprint_int(char *buf, size_t left, int val, int width) {
    char tmp[16];
    int neg = 0, len = 0;
    unsigned int v = (unsigned int)val;
    if (val < 0) { neg = 1; v = (unsigned int)-val; }
    do { tmp[len++] = '0' + (v % 10); v /= 10; } while (v);
    int total = len > width ? len : width;
    if (neg) total++;
    if ((size_t)total >= left) return -1;
    int pos = 0;
    if (neg) buf[pos++] = '-';
    for (int i = 0; i < width - len; i++) buf[pos++] = '0';
    for (int i = len - 1; i >= 0; i--) buf[pos++] = tmp[i];
    return pos;
}

static const char *_wday_name[] = {"Sunday","Monday","Tuesday","Wednesday","Thursday","Friday","Saturday"};
static const char *_wday_abbr[] = {"Sun","Mon","Tue","Wed","Thu","Fri","Sat"};
static const char *_mon_name[] = {"January","February","March","April","May","June","July","August","September","October","November","December"};
static const char *_mon_abbr[] = {"Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"};

size_t strftime(char *s, size_t max, const char *format, const struct tm *tm) {
    if (!s || max == 0 || !format || !tm) return 0;
    size_t pos = 0;
    for (const char *f = format; *f && pos < max - 1; f++) {
        if (*f != '%') { s[pos++] = *f; continue; }
        f++;
        if (!*f) break;
        int n;
        const char *str;
        switch (*f) {
        case 'Y': n = _snprint_int(s+pos, max-pos, tm->tm_year+1900, 4); if (n<0) goto done; pos+=n; break;
        case 'm': n = _snprint_int(s+pos, max-pos, tm->tm_mon+1, 2); if (n<0) goto done; pos+=n; break;
        case 'd': n = _snprint_int(s+pos, max-pos, tm->tm_mday, 2); if (n<0) goto done; pos+=n; break;
        case 'H': n = _snprint_int(s+pos, max-pos, tm->tm_hour, 2); if (n<0) goto done; pos+=n; break;
        case 'M': n = _snprint_int(s+pos, max-pos, tm->tm_min, 2); if (n<0) goto done; pos+=n; break;
        case 'S': n = _snprint_int(s+pos, max-pos, tm->tm_sec, 2); if (n<0) goto done; pos+=n; break;
        case 'A': str = (tm->tm_wday>=0&&tm->tm_wday<7)?_wday_name[tm->tm_wday]:"?";
                  for (;*str&&pos<max-1;) s[pos++]=*str++; break;
        case 'a': str = (tm->tm_wday>=0&&tm->tm_wday<7)?_wday_abbr[tm->tm_wday]:"?";
                  for (;*str&&pos<max-1;) s[pos++]=*str++; break;
        case 'B': str = (tm->tm_mon>=0&&tm->tm_mon<12)?_mon_name[tm->tm_mon]:"?";
                  for (;*str&&pos<max-1;) s[pos++]=*str++; break;
        case 'b': case 'h':
                  str = (tm->tm_mon>=0&&tm->tm_mon<12)?_mon_abbr[tm->tm_mon]:"?";
                  for (;*str&&pos<max-1;) s[pos++]=*str++; break;
        case 'e': if (tm->tm_mday < 10 && pos < max-1) s[pos++] = ' ';
                  n = _snprint_int(s+pos, max-pos, tm->tm_mday, 1); if (n<0) goto done; pos+=n; break;
        case 'j': n = _snprint_int(s+pos, max-pos, tm->tm_yday+1, 3); if (n<0) goto done; pos+=n; break;
        case 'p': str = (tm->tm_hour>=12)?"PM":"AM";
                  for (;*str&&pos<max-1;) s[pos++]=*str++; break;
        case 'I': { int h = tm->tm_hour%12; if(h==0)h=12;
                  n = _snprint_int(s+pos, max-pos, h, 2); if(n<0) goto done; pos+=n; } break;
        case 'n': s[pos++] = '\n'; break;
        case 't': s[pos++] = '\t'; break;
        case '%': s[pos++] = '%'; break;
        default: if (pos+1<max-1) { s[pos++]='%'; s[pos++]=*f; } break;
        }
    }
done:
    s[pos] = '\0';
    return pos;
}

time_t mktime(struct tm *tm) {
    if (!tm) return (time_t)-1;
    int y = tm->tm_year + 1900;
    int m = tm->tm_mon + 1;
    int d = tm->tm_mday;
    if (m <= 2) { y--; m += 12; }
    long days = 365L * y + y/4 - y/100 + y/400 + (153*(m-3)+2)/5 + d - 719469;
    return (time_t)(days * 86400 + tm->tm_hour * 3600 + tm->tm_min * 60 + tm->tm_sec);
}

double difftime(time_t time1, time_t time0) {
    return (double)(time1 - time0);
}

time_t timegm(struct tm *tm) {
    return mktime(tm);
}

/* asctime — format struct tm as "Day Mon DD HH:MM:SS YYYY\n" */
static char _asctime_buf[26];

char *asctime_r(const struct tm *tm, char *buf) {
    if (!tm || !buf) return (void*)0;
    const char *wday = (tm->tm_wday >= 0 && tm->tm_wday < 7) ? _wday_abbr[tm->tm_wday] : "???";
    const char *mon = (tm->tm_mon >= 0 && tm->tm_mon < 12) ? _mon_abbr[tm->tm_mon] : "???";
    int year = tm->tm_year + 1900;
    int pos = 0;
    for (int i = 0; i < 3 && wday[i]; i++) buf[pos++] = wday[i];
    buf[pos++] = ' ';
    for (int i = 0; i < 3 && mon[i]; i++) buf[pos++] = mon[i];
    buf[pos++] = ' ';
    buf[pos++] = (tm->tm_mday / 10) ? ('0' + tm->tm_mday / 10) : ' ';
    buf[pos++] = '0' + tm->tm_mday % 10;
    buf[pos++] = ' ';
    buf[pos++] = '0' + tm->tm_hour / 10;
    buf[pos++] = '0' + tm->tm_hour % 10;
    buf[pos++] = ':';
    buf[pos++] = '0' + tm->tm_min / 10;
    buf[pos++] = '0' + tm->tm_min % 10;
    buf[pos++] = ':';
    buf[pos++] = '0' + tm->tm_sec / 10;
    buf[pos++] = '0' + tm->tm_sec % 10;
    buf[pos++] = ' ';
    buf[pos++] = '0' + (year / 1000) % 10;
    buf[pos++] = '0' + (year / 100) % 10;
    buf[pos++] = '0' + (year / 10) % 10;
    buf[pos++] = '0' + year % 10;
    buf[pos++] = '\n';
    buf[pos] = '\0';
    return buf;
}

char *asctime(const struct tm *tm) {
    return asctime_r(tm, _asctime_buf);
}

char *ctime_r(const time_t *timer, char *buf) {
    if (!timer) return (void*)0;
    struct tm result;
    struct tm *tm = localtime_r(timer, &result);
    if (!tm) return (void*)0;
    return asctime_r(tm, buf);
}

char *ctime(const time_t *timer) {
    return ctime_r(timer, _asctime_buf);
}

int gettimeofday(struct timeval *tv, struct timezone *tz) {
    if (tv) {
        unsigned long ticks = (unsigned long)_syscall(SYS_UPTIME, 0, 0, 0, 0, 0);
        unsigned long hz = (unsigned long)_syscall(SYS_TICK_HZ, 0, 0, 0, 0, 0);
        if (hz == 0) hz = 1000;
        tv->tv_sec = (long)(ticks / hz);
        tv->tv_usec = (long)((ticks % hz) * (1000000 / hz));
    }
    if (tz) {
        tz->tz_minuteswest = 0;
        tz->tz_dsttime = 0;
    }
    return 0;
}
