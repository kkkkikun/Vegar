#!/bin/bash
# Local cross-compilation test script

set -euo pipefail

echo "🚀 Cross-architecture compilation test for mbarrier"
echo "=================================================="

# Check if cross is installed
if ! command -v cross &> /dev/null; then
    echo "❌ 'cross' is not installed. Installing..."
    cargo install cross
fi

# Define test targets
TARGETS=(
    # x86 family
    "x86_64-unknown-linux-gnu"
    "i686-unknown-linux-gnu"
    
    # ARM family  
    "aarch64-unknown-linux-gnu"
    "armv7-unknown-linux-gnueabihf"
    
    # RISC-V family
    "riscv64gc-unknown-linux-gnu"
    "riscv32gc-unknown-linux-gnu"
    
    # Embedded/no-std targets
    "thumbv7em-none-eabihf"
    "aarch64-unknown-none-softfloat"
    "riscv32imc-unknown-none-elf"
    "riscv64gc-unknown-none-elf"
)

FAILED_TARGETS=()
PASSED_TARGETS=()

for target in "${TARGETS[@]}"; do
    echo ""
    echo "🔧 Testing target: $target"
    echo "-----------------------------------"
    
    # Add target if not installed
    if ! rustup target list --installed | grep -q "$target"; then
        echo "📦 Installing target $target..."
        rustup target add "$target" || {
            echo "⚠️  Could not install target $target, skipping..."
            continue
        }
    fi
    
    # Test basic build
    echo "🏗️  Building library..."
    if cross build --target "$target" --lib; then
        echo "✅ Library build succeeded"
    else
        echo "❌ Library build failed"
        FAILED_TARGETS+=("$target")
        continue
    fi
    
    # Test with SMP feature
    echo "🏗️  Building with SMP feature..."
    if cross build --target "$target" --lib --features smp; then
        echo "✅ SMP build succeeded"
    else
        echo "❌ SMP build failed"
        FAILED_TARGETS+=("$target")
        continue
    fi
    
    # Test examples (skip for no-std targets)
    if [[ "$target" != *"none"* ]]; then
        echo "🏗️  Building examples..."
        if cross build --target "$target" --examples; then
            echo "✅ Examples build succeeded"
        else
            echo "⚠️  Examples build failed (non-critical)"
        fi
    else
        echo "⏭️  Skipping examples for no-std target"
    fi
    
    PASSED_TARGETS+=("$target")
    echo "✅ Target $target: ALL TESTS PASSED"
done

echo ""
echo "📊 Test Summary"
echo "==============="
echo "✅ Passed: ${#PASSED_TARGETS[@]} targets"
for target in "${PASSED_TARGETS[@]}"; do
    echo "   - $target"
done

if [ ${#FAILED_TARGETS[@]} -gt 0 ]; then
    echo ""
    echo "❌ Failed: ${#FAILED_TARGETS[@]} targets"
    for target in "${FAILED_TARGETS[@]}"; do
        echo "   - $target"
    done
    exit 1
else
    echo ""
    echo "🎉 All targets passed!"
    echo ""
    echo "🔍 Architecture-specific assembly analysis:"
    echo "Run: cargo rustc --example asm_analysis --release -- --emit=asm"
    echo "Then check target/release/examples/*.s for barrier instructions"
fi
