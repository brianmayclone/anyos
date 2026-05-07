#!/usr/bin/env bash
# ASL toolchain profile: Java (JDK + build tools)
#
# Idempotent installer for the Java developer toolchain (ADR-0008).
# Installs OpenJDK 21 (current Debian stable LTS), Maven, Gradle.
#
# Designed to be invoked via:
#     aslctl run <distro> -- bash /path/to/dev-java.sh

set -euo pipefail

if ! command -v apt-get >/dev/null 2>&1; then
    echo "ERROR: this profile requires a Debian/Ubuntu-based distro." >&2
    exit 1
fi

if [[ "${EUID:-0}" -ne 0 ]]; then SUDO="sudo"; else SUDO=""; fi

PACKAGES=(
    openjdk-21-jdk-headless
    maven
    gradle
)

echo "[asl-toolchain:dev-java] refreshing apt index"
$SUDO apt-get update -qq

echo "[asl-toolchain:dev-java] installing: ${PACKAGES[*]}"
$SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "${PACKAGES[@]}"

echo "[asl-toolchain:dev-java] verifying"
java --version
javac --version
mvn --version
gradle --version | head -3

echo "[asl-toolchain:dev-java] done."
