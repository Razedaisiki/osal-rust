#!/bin/bash
# build.sh — build the OSAL FreeRTOS QEMU MPS2 boot firmware.
#
# Checks toolchain availability, builds the ELF, verifies symbols,
# and records tool versions.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INTEG_DIR="$(dirname "$SCRIPT_DIR")"
BUILD_DIR="$INTEG_DIR/build"

echo "=== Build: OSAL FreeRTOS QEMU MPS2 Cortex-M3 ==="
echo ""

# ------------------------------------------------------------------
# 1. Check toolchain
# ------------------------------------------------------------------
echo "--- Checking toolchain ---"
for tool in arm-none-eabi-gcc arm-none-eabi-size arm-none-eabi-nm; do
    if ! command -v "$tool" &>/dev/null; then
        echo "ERROR: $tool not found in PATH" >&2
        exit 1
    fi
    echo "  $tool: $(command -v "$tool")"
done

for tool in qemu-system-arm cargo; do
    if ! command -v "$tool" &>/dev/null; then
        echo "ERROR: $tool not found in PATH" >&2
        exit 1
    fi
    echo "  $tool: $(command -v "$tool")"
done

# Verify the thumbv7m-none-eabi Rust target is installed.
if ! rustup target list --installed | grep -q 'thumbv7m-none-eabi'; then
    echo "ERROR: Rust target thumbv7m-none-eabi is not installed" >&2
    echo "  Run: rustup target add thumbv7m-none-eabi" >&2
    exit 1
fi
echo "  Rust target: thumbv7m-none-eabi (installed)"

echo ""

# ------------------------------------------------------------------
# 2. Check third-party sources
# ------------------------------------------------------------------
echo "--- Checking third-party sources ---"
KERNEL_DIR="$INTEG_DIR/../../third_party/freertos-kernel"
if [ ! -f "$KERNEL_DIR/.git" ] && [ ! -d "$KERNEL_DIR/.git" ]; then
    echo "ERROR: FreeRTOS kernel submodule not present at $KERNEL_DIR" >&2
    echo "  Run: git submodule update --init third_party/freertos-kernel" >&2
    exit 1
fi
echo "  Kernel submodule: OK ($KERNEL_DIR)"

MPS2_DIR="$INTEG_DIR/../../third_party/mps2-an385-reference"
if [ ! -f "$MPS2_DIR/startup_gcc.c" ]; then
    echo "ERROR: MPS2 reference files not found at $MPS2_DIR" >&2
    exit 1
fi
echo "  MPS2 reference: OK ($MPS2_DIR)"
echo ""

# ------------------------------------------------------------------
# 3. Prepare build directory (clean old artifacts, then create fresh)
# ------------------------------------------------------------------
echo "--- Preparing build directory ---"
make -C "$INTEG_DIR" clean
mkdir -p "$BUILD_DIR"
echo ""

# ------------------------------------------------------------------
# 4. Record tool versions (write before make all, after mkdir)
# ------------------------------------------------------------------
echo "--- Tool versions ---"
arm-none-eabi-gcc --version | head -1 | tee "$BUILD_DIR/toolchain-gcc.txt"
qemu-system-arm --version | head -1 | tee "$BUILD_DIR/toolchain-qemu.txt"
echo ""

# ------------------------------------------------------------------
# 5. Build
# ------------------------------------------------------------------
echo "--- Building firmware ---"
make -C "$INTEG_DIR" all
echo ""

# ------------------------------------------------------------------
# 5. Symbol check
# ------------------------------------------------------------------
echo "--- Symbol check ---"
make -C "$INTEG_DIR" check-symbols
echo ""

# ------------------------------------------------------------------
# 6. Record ELF size
# ------------------------------------------------------------------
echo "--- ELF size ---"
cat "$BUILD_DIR/freertos-qemu-mps2.size.txt"
echo ""

echo "=== Build complete ==="
echo "ELF: $BUILD_DIR/freertos-qemu-mps2.elf"
echo "MAP: $BUILD_DIR/freertos-qemu-mps2.map"
