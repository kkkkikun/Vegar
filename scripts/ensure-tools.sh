#!/bin/bash
# Script to ensure build tools are available
# Returns 0 if tools are ready, 1 if build failed

set -e

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/tools"
TOOLS_BIN_DIR="${TOOLS_DIR}/target/release"

# Check if tool exists
if [ -x "${TOOLS_BIN_DIR}/axconfig-gen" ]; then
    exit 0
fi

# Tool doesn't exist, build it
echo "Build tool not found, building axconfig-gen from source..." >&2
cd "${TOOLS_DIR}"

# Try to build
if make axconfig-gen >/dev/null 2>&1; then
    echo "axconfig-gen: OK" >&2
    exit 0
else
    # Build failed, try again with error output
    echo "Failed to build axconfig-gen. Retrying with error output..." >&2
    if make axconfig-gen; then
        echo "axconfig-gen: OK" >&2
        exit 0
    else
        echo "Error: Failed to build axconfig-gen" >&2
        exit 1
    fi
fi
