#include <time.h>
#include <sys/time.h>

extern int _syscall(int num, int a1, int a2, int a3, int a4);

#define SYS_TIME   30
#define SYS_UPTIME 31

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

size_t strftime(char *s, size_t max, const char *format, const struct tm *tm) {
    (void)format; (void)tm;
    if (max > 0) s[0] = '\0';
    return 0;
}

int gettimeofday(struct timeval *tv, struct timezone *tz) {
    if (tv) {
        tv->tv_sec = _syscall(SYS_UPTIME, 0, 0, 0, 0) / 100;
        tv->tv_usec = (_syscall(SYS_UPTIME, 0, 0, 0, 0) % 100) * 10000;
    }
    if (tz) {
        tz->tz_minuteswest = 0;
        tz->tz_dsttime = 0;
    }
    return 0;
}
