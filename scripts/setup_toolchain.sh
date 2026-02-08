#!/bin/bash
set -e

echo "Setting up anyOS development toolchain..."

# Rust nightly with required components
rustup install nightly
rustup component add rust-src llvm-tools-preview --toolchain nightly

# Check for NASM
if ! command -v nasm &> /dev/null; then
    echo "Installing NASM..."
    if command -v brew &> /dev/null; then
        brew install nasm
    else
        echo "Please install NASM manually"
        exit 1
    fi
fi

# Check for CMake
if ! command -v cmake &> /dev/null; then
    echo "Installing CMake..."
    if command -v brew &> /dev/null; then
        brew install cmake
    else
        echo "Please install CMake manually"
        exit 1
    fi
fi

# Check for Ninja
if ! command -v ninja &> /dev/null; then
    echo "Installing Ninja..."
    if command -v brew &> /dev/null; then
        brew install ninja
    else
        echo "Please install Ninja manually"
        exit 1
    fi
fi

# Check for QEMU
if ! command -v qemu-system-i386 &> /dev/null; then
    echo "Installing QEMU..."
    if command -v brew &> /dev/null; then
        brew install qemu
    else
        echo "Please install QEMU manually"
        exit 1
    fi
fi

echo ""
echo "Toolchain versions:"
echo "  rustc: $(rustc +nightly --version)"
echo "  nasm:  $(nasm --version)"
echo "  cmake: $(cmake --version | head -1)"
echo "  ninja: $(ninja --version)"
echo "  qemu:  $(qemu-system-i386 --version | head -1)"
echo ""
echo "Toolchain setup complete!"
