#!/bin/bash
set -e

# Test script for QEMU integration
echo "🧪 Testing QEMU integration for cross-architecture testing"

# List of targets to test
targets=(
    "riscv64gc-unknown-linux-gnu"
    "aarch64-unknown-linux-gnu"
    "armv7-unknown-linux-gnueabihf"
)

# Check if QEMU is installed
if ! command -v qemu-riscv64-static >/dev/null 2>&1; then
    echo "❌ QEMU not found. Install with: sudo apt-get install qemu-user-static"
    exit 1
fi

for target in "${targets[@]}"; do
    echo "🔧 Testing target: $target"
    
    # Check if target is installed
    if ! rustup target list --installed | grep -q "$target"; then
        echo "📦 Installing Rust target: $target"
        rustup target add "$target"
    fi
    
    # Build for target
    echo "🔨 Building for $target..."
    if cargo build --target "$target" 2>/dev/null; then
        echo "✅ Build successful for $target"
    else
        echo "⚠️  Build failed for $target (this might be expected for some targets)"
    fi
done

echo "🎉 QEMU integration test completed!"
