#!/bin/bash
# Build libcurl for anyOS (i686 freestanding cross-compile with BearSSL)
#
# Output: third_party/curl/lib/libcurl.a
set +e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CC="i686-elf-gcc"
SRCDIR="$ROOT/third_party/curl/lib"
OBJDIR="$SRCDIR/obj"
OUTPUT="$SRCDIR/libcurl.a"

mkdir -p "$OBJDIR" "$OBJDIR/vtls" "$OBJDIR/vauth" "$OBJDIR/vquic" "$OBJDIR/vssh"

CFLAGS="-O2 -ffreestanding -nostdlib -fno-builtin -m32 -std=c99 -w"
CFLAGS="$CFLAGS -isystem $ROOT/libs/libc/include"
CFLAGS="$CFLAGS -I$ROOT/third_party/curl/include"
CFLAGS="$CFLAGS -I$ROOT/third_party/curl/lib"
CFLAGS="$CFLAGS -I$ROOT/third_party/bearssl/inc"
CFLAGS="$CFLAGS -DHAVE_CONFIG_H -DCURL_STATICLIB -DBUILDING_LIBCURL"

SUCCESS=0
FAIL=0
ERRORS=""

compile_file() {
    local src="$1"
    local obj="$2"
    local out
    out=$($CC $CFLAGS -c "$src" -o "$obj" 2>&1)
    local ret=$?
    if [ $ret -ne 0 ]; then
        FAIL=$((FAIL+1))
        local fname=$(basename "$src")
        local fatal=$(echo "$out" | grep -m1 "fatal error")
        local err=$(echo "$out" | grep -m1 "error:")
        if [ -n "$fatal" ]; then
            ERRORS="$ERRORS$fname: $fatal\n"
        elif [ -n "$err" ]; then
            ERRORS="$ERRORS$fname: $err\n"
        else
            ERRORS="$ERRORS$fname: UNKNOWN ERROR\n"
        fi
    else
        SUCCESS=$((SUCCESS+1))
    fi
}

echo "=== Building libcurl for anyOS (i686) ==="

# Core files
for f in altsvc.c amigaos.c asyn-ares.c asyn-thread.c base64.c bufq.c bufref.c \
  cf-h1-proxy.c cf-h2-proxy.c cf-haproxy.c cf-https-connect.c cf-socket.c \
  cfilters.c conncache.c connect.c content_encoding.c cookie.c \
  curl_addrinfo.c curl_des.c curl_endian.c curl_fnmatch.c curl_get_line.c \
  curl_gethostname.c curl_gssapi.c curl_memrchr.c curl_multibyte.c \
  curl_ntlm_core.c curl_range.c curl_rtmp.c curl_sasl.c curl_sha512_256.c \
  curl_sspi.c curl_threads.c curl_trc.c cw-out.c dict.c dllmain.c doh.c \
  dynbuf.c dynhds.c easy.c easygetopt.c easyoptions.c escape.c file.c \
  fileinfo.c fopen.c formdata.c ftp.c ftplistparser.c getenv.c getinfo.c \
  gopher.c hash.c headers.c hmac.c hostasyn.c hostip.c hostip4.c hostip6.c \
  hostsyn.c hsts.c http.c http1.c http2.c http_aws_sigv4.c http_chunks.c \
  http_digest.c http_negotiate.c http_ntlm.c http_proxy.c httpsrr.c idn.c \
  if2ip.c imap.c inet_ntop.c inet_pton.c krb5.c ldap.c llist.c macos.c \
  md4.c md5.c memdebug.c mime.c mprintf.c mqtt.c multi.c netrc.c \
  nonblock.c noproxy.c openldap.c parsedate.c pingpong.c pop3.c progress.c \
  psl.c rand.c rename.c request.c rtsp.c select.c sendf.c setopt.c \
  sha256.c share.c slist.c smb.c smtp.c socketpair.c socks.c \
  socks_gssapi.c socks_sspi.c speedcheck.c splay.c strcase.c strdup.c \
  strerror.c strparse.c strtok.c strtoofft.c system_win32.c telnet.c \
  tftp.c timediff.c timeval.c transfer.c url.c urlapi.c version.c \
  version_win32.c warnless.c ws.c; do
  compile_file "$SRCDIR/$f" "$OBJDIR/${f%.c}.o"
done

# vtls
for f in bearssl.c cipher_suite.c gtls.c hostcheck.c keylog.c mbedtls.c \
  mbedtls_threadlock.c openssl.c rustls.c schannel.c schannel_verify.c \
  sectransp.c vtls.c vtls_scache.c vtls_spack.c x509asn1.c; do
  compile_file "$SRCDIR/vtls/$f" "$OBJDIR/vtls/${f%.c}.o"
done

# vauth
for f in cleartext.c cram.c digest.c digest_sspi.c gsasl.c krb5_gssapi.c \
  krb5_sspi.c ntlm.c ntlm_sspi.c oauth2.c spnego_gssapi.c spnego_sspi.c vauth.c; do
  compile_file "$SRCDIR/vauth/$f" "$OBJDIR/vauth/${f%.c}.o"
done

echo "SUCCESS: $SUCCESS, FAIL: $FAIL"
if [ -n "$ERRORS" ]; then
    echo ""
    echo "=== ERRORS ==="
    echo -e "$ERRORS" | sort -u
fi

if [ $FAIL -eq 0 ]; then
    echo "Creating libcurl.a..."
    i686-elf-ar rcs "$OUTPUT" $OBJDIR/*.o $OBJDIR/vtls/*.o $OBJDIR/vauth/*.o
    echo "=== Done: $OUTPUT ($(du -h "$OUTPUT" | cut -f1)) ==="
fi
