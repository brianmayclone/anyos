#!/usr/bin/env bash
set -euo pipefail

SUITE="bookworm"
ARCH="amd64"
MIRROR="http://deb.debian.org/debian"
OUTPUT=""
SIZE="2G"
HOSTNAME="anyos-asl"
ROOT_PASSWORD="root"

usage() {
  cat <<'USAGE'
Usage:
  sudo bash scripts/asl-debootstrap-image.sh --output <disk.raw> [options]

Options:
  --suite <name>          Debian suite (default: bookworm)
  --arch <arch>           Debian architecture (default: amd64)
  --mirror <url>          Debian mirror (default: http://deb.debian.org/debian)
  --size <size>           Raw disk size for truncate (default: 2G)
  --hostname <name>       Guest hostname (default: anyos-asl)
  --root-password <pass>  Guest root password for bring-up (default: root)

Builds a BIOS-bootable raw disk with one ext4 partition, GRUB, a Debian
minbase rootfs, serial console, and a generic Linux kernel. The resulting
image is intended for ASL's seabios-x86_64 profile.
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --suite) SUITE="${2:?missing suite}"; shift 2 ;;
    --arch) ARCH="${2:?missing arch}"; shift 2 ;;
    --mirror) MIRROR="${2:?missing mirror}"; shift 2 ;;
    --output) OUTPUT="${2:?missing output}"; shift 2 ;;
    --size) SIZE="${2:?missing size}"; shift 2 ;;
    --hostname) HOSTNAME="${2:?missing hostname}"; shift 2 ;;
    --root-password) ROOT_PASSWORD="${2:?missing password}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [ -z "$OUTPUT" ]; then
  echo "Missing --output" >&2
  usage >&2
  exit 2
fi

if [ "$(id -u)" -ne 0 ]; then
  echo "This builder needs root for loop devices, mkfs, mount, and chroot." >&2
  exit 1
fi

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required tool: $1" >&2
    exit 1
  fi
}

need debootstrap
need sfdisk
need losetup
need mkfs.ext4
need mount
need umount
need chroot
need grub-install
need update-grub
need blkid

OUTPUT="$(readlink -m "$OUTPUT")"
WORKDIR="$(mktemp -d /tmp/asl-debootstrap.XXXXXX)"
MNT="$WORKDIR/root"
LOOPDEV=""

cleanup() {
  set +e
  if mountpoint -q "$MNT/dev"; then umount "$MNT/dev"; fi
  if mountpoint -q "$MNT/proc"; then umount "$MNT/proc"; fi
  if mountpoint -q "$MNT/sys"; then umount "$MNT/sys"; fi
  if mountpoint -q "$MNT"; then umount "$MNT"; fi
  if [ -n "$LOOPDEV" ]; then losetup -d "$LOOPDEV"; fi
  rm -rf "$WORKDIR"
}
trap cleanup EXIT

mkdir -p "$(dirname "$OUTPUT")" "$MNT"
truncate -s "$SIZE" "$OUTPUT"

sfdisk "$OUTPUT" >/dev/null <<'SFDISK'
label: dos
unit: sectors

start=2048, type=83, bootable
SFDISK

LOOPDEV="$(losetup --find --show --partscan "$OUTPUT")"
PART="${LOOPDEV}p1"
if [ ! -b "$PART" ]; then
  PART="$LOOPDEV"
fi

mkfs.ext4 -F -L ASLROOT "$PART"
ROOT_UUID="$(blkid -s UUID -o value "$PART")"
mount "$PART" "$MNT"

debootstrap \
  --arch="$ARCH" \
  --variant=minbase \
  --include=linux-image-amd64,grub-pc,systemd-sysv,ifupdown,iproute2,isc-dhcp-client,ca-certificates,openssh-server \
  "$SUITE" \
  "$MNT" \
  "$MIRROR"

cat >"$MNT/etc/hostname" <<EOF_HOSTNAME
$HOSTNAME
EOF_HOSTNAME

cat >"$MNT/etc/hosts" <<EOF_HOSTS
127.0.0.1 localhost
127.0.1.1 $HOSTNAME
EOF_HOSTS

cat >"$MNT/etc/fstab" <<EOF_FSTAB
UUID=$ROOT_UUID / ext4 errors=remount-ro 0 1
proc /proc proc defaults 0 0
EOF_FSTAB

cat >"$MNT/etc/default/grub" <<EOF_GRUB
GRUB_DEFAULT=0
GRUB_TIMEOUT=1
GRUB_DISTRIBUTOR=ASL
GRUB_CMDLINE_LINUX_DEFAULT=""
GRUB_CMDLINE_LINUX="console=ttyS0,115200n8 earlyprintk=serial root=UUID=$ROOT_UUID ro"
GRUB_TERMINAL=serial
GRUB_SERIAL_COMMAND="serial --speed=115200 --unit=0 --word=8 --parity=no --stop=1"
EOF_GRUB

mkdir -p "$MNT/etc/systemd/system/serial-getty@ttyS0.service.d"
cat >"$MNT/etc/systemd/system/serial-getty@ttyS0.service.d/asl.conf" <<'EOF_GETTY'
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin root --keep-baud 115200,38400,9600 ttyS0 vt102
EOF_GETTY

printf 'root:%s\n' "$ROOT_PASSWORD" | chroot "$MNT" chpasswd
mount --bind /dev "$MNT/dev"
mount -t proc proc "$MNT/proc"
mount -t sysfs sys "$MNT/sys"
chroot "$MNT" grub-install --target=i386-pc --boot-directory=/boot "$LOOPDEV"
chroot "$MNT" update-grub

sync
echo "ASL debootstrap image created: $OUTPUT"
echo "Root password: $ROOT_PASSWORD"
echo "Use with: aslctl create <name> <image-ref> <owner> --kernel-profile seabios-x86_64"
