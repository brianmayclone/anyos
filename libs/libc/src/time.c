/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */

#include <time.h>
#include <sys/time.h>

extern int _syscall(int num, int a1, int a2, int a3, int a4);

#define SYS_TIME    30
#define SYS_UPTIME  31
#define SYS_TICK_HZ 34

static struct tm _tm;

time_t time(time_t *tloc) {
    unsigned char buf[8];
    _syscall(SYS_TIME, (int)buf, 0, 0, 0);
    /* Simple: return uptime ticks as "time" since we lack epoch */
    time_t t = _syscall(SYS_UPTIME, 0, 0, 0, 0);
    if (tloc) *tloc = t;
    return t;
}

clock_t clock(void) {
    return (clock_t)_syscall(SYS_UPTIME, 0, 0, 0, 0);
}

struct tm *localtime(const time_t *timer) {
    /* Fetch real date/time from RTC */
    unsigned char buf[8];
    _syscall(SYS_TIME, (int)buf, 0, 0, 0);
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
        case 'e': /* day of month, space-padded */
                  if (tm->tm_mday < 10 && pos < max-1) s[pos++] = ' ';
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

int gettimeofday(struct timeval *tv, struct timezone *tz) {
    if (tv) {
        unsigned int ticks = (unsigned int)_syscall(SYS_UPTIME, 0, 0, 0, 0);
        unsigned int hz = (unsigned int)_syscall(SYS_TICK_HZ, 0, 0, 0, 0);
        if (hz == 0) hz = 1000;
        tv->tv_sec = ticks / hz;
        tv->tv_usec = (ticks % hz) * (1000000 / hz);
    }
    if (tz) {
        tz->tz_minuteswest = 0;
        tz->tz_dsttime = 0;
    }
    return 0;
}
