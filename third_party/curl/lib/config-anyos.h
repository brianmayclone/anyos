/*
 * config-anyos.h — curl configuration for anyOS (i686-elf, freestanding)
 *
 * Minimal HTTP-only build using anyOS TCP/UDP syscalls via libc socket layer.
 * No SSL/TLS, no threading, no IPv6.
 */

#ifndef HEADER_CURL_CONFIG_ANYOS_H
#define HEADER_CURL_CONFIG_ANYOS_H

#define CURL_STATICLIB 1

#ifndef CURL_OS
#define CURL_OS "anyos"
#endif

#define STDC_HEADERS 1

/* Sizes (i686 = 32-bit) */
#define SIZEOF_INT 4
#define SIZEOF_LONG 4
#define SIZEOF_OFF_T 4
#define SIZEOF_CURL_OFF_T 8
#define SIZEOF_SIZE_T 4
#define SIZEOF_TIME_T 4

/* recv() */
#define HAVE_RECV 1
#define RECV_TYPE_ARG1 int
#define RECV_TYPE_ARG2 void *
#define RECV_TYPE_ARG3 size_t
#define RECV_TYPE_ARG4 int
#define RECV_TYPE_RETV ssize_t

/* send() */
#define HAVE_SEND 1
#define SEND_TYPE_ARG1 int
#define SEND_TYPE_ARG2 void *
#define SEND_QUAL_ARG2 const
#define SEND_TYPE_ARG3 size_t
#define SEND_TYPE_ARG4 int
#define SEND_TYPE_RETV ssize_t

/* Headers we have */
#define HAVE_ARPA_INET_H 1
#define HAVE_ASSERT_H 1
#define HAVE_BOOL_T 1
#define HAVE_ERRNO_H 1
#define HAVE_FCNTL_H 1
#define HAVE_NETDB_H 1
#define HAVE_NETINET_IN_H 1
#define HAVE_NETINET_TCP_H 1
#define HAVE_POLL_H 1
#define HAVE_SIGNAL_H 1
#define HAVE_STDINT_H 1
#define HAVE_STDLIB_H 1
#define HAVE_STRING_H 1
#define HAVE_SYS_IOCTL_H 1
#define HAVE_SYS_SELECT_H 1
#define HAVE_SYS_SOCKET_H 1
#define HAVE_SYS_STAT_H 1
#define HAVE_SYS_TIME_H 1
#define HAVE_SYS_TYPES_H 1
#define HAVE_TIME_H 1
#define HAVE_UNISTD_H 1
#define HAVE_INTTYPES_H 1
#define HAVE_STRINGS_H 1
#define HAVE_LIMITS_H 1

/* Functions we have */
#define HAVE_SOCKET 1
#define HAVE_SELECT 1
#define HAVE_GETADDRINFO 1
#define HAVE_FREEADDRINFO 1
#define HAVE_FTRUNCATE 1
#define HAVE_STRDUP 1
#define HAVE_STRTOLL 1
/* No strerror_r — use strerror() fallback */
#define HAVE_SNPRINTF 1
#define HAVE_MEMRCHR 1
#define HAVE_INET_PTON 1
#define HAVE_INET_NTOP 1
#define HAVE_BASENAME 1
#define HAVE_CLOSE 1
#define HAVE_FCNTL 1

/* Struct fields */
#define HAVE_STRUCT_TIMEVAL 1
#define HAVE_LONGLONG 1

/* Disable everything except HTTP + FTP */
#define CURL_DISABLE_DICT 1
#define CURL_DISABLE_GOPHER 1
#define CURL_DISABLE_IMAP 1
#define CURL_DISABLE_LDAP 1
#define CURL_DISABLE_LDAPS 1
#define CURL_DISABLE_MQTT 1
#define CURL_DISABLE_POP3 1
#define CURL_DISABLE_RTSP 1
#define CURL_DISABLE_SMB 1
#define CURL_DISABLE_SMTP 1
#define CURL_DISABLE_TELNET 1
#define CURL_DISABLE_TFTP 1

/* Disable features we cannot support */
#define CURL_DISABLE_NTLM 1
#define CURL_DISABLE_KERBEROS_AUTH 1
#define CURL_DISABLE_NEGOTIATE_AUTH 1
#define CURL_DISABLE_AWS 1
#define CURL_DISABLE_DOH 1
#define CURL_DISABLE_NETRC 1
#define CURL_DISABLE_PROXY 1
#define CURL_DISABLE_ALTSVC 1
#define CURL_DISABLE_HSTS 1
#define CURL_DISABLE_WEBSOCKETS 1
#define CURL_DISABLE_FORM_API 1
#define CURL_DISABLE_MIME 1
/* BINDLOCAL needed for FTP PORT (Curl_if2ip) */
#define CURL_DISABLE_VERBOSE_STRINGS 1
#define CURL_DISABLE_HEADERS_API 1
#define CURL_DISABLE_SHUFFLE_DNS 1
#define CURL_DISABLE_SOCKETPAIR 1
#define CURL_DISABLE_THREADED_RESOLVER 1

/* No SSL/TLS */
/* (no USE_OPENSSL, USE_BEARSSL, etc.) */

/* No compression */
/* (no HAVE_LIBZ, etc.) */

/* No IPv6 */
/* (no USE_IPV6) */

/* Use our own printf (curl has an internal one) */
#define HAVE_VARIADIC_MACROS_C99 1
#define HAVE_VARIADIC_MACROS_GCC 1

/* Non-blocking I/O */
#define HAVE_FCNTL_O_NONBLOCK 1

/* Misc */
#define HAVE_PIPE 1
#define HAVE_CLOCK_GETTIME 1
#define OS "anyos"
#define PACKAGE "curl"
#define PACKAGE_NAME "curl"
#define PACKAGE_STRING "curl 8.12.0"
#define PACKAGE_VERSION "8.12.0"
#define PACKAGE_BUGREPORT "anyos"

/* File system access for cookie jar, etc. */
#define HAVE_FCHMOD 1

#endif /* HEADER_CURL_CONFIG_ANYOS_H */
