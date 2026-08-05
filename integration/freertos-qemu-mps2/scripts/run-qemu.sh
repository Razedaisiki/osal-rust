#!/bin/bash
# run-qemu.sh — run the boot firmware on QEMU, capture output, verify.
#
# Firmware exits via ARM semihosting BKPT 0xAB (SYS_EXIT).
# QEMU must exit with code 0; timeout or crash = failure.
# The verifier double-checks the UART boot protocol.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INTEG_DIR="$(dirname "$SCRIPT_DIR")"
BUILD_DIR="$INTEG_DIR/build"
ELF="$BUILD_DIR/freertos-qemu-mps2.elf"
LOG="$BUILD_DIR/qemu.log"
TIMEOUT_SEC=30

# ------------------------------------------------------------------
# Check ELF exists
# ------------------------------------------------------------------
if [ ! -f "$ELF" ]; then
    echo "ERROR: ELF not found at $ELF" >&2
    echo "  Run: scripts/build.sh" >&2
    exit 1
fi

# ------------------------------------------------------------------
# Run QEMU with timeout
# ------------------------------------------------------------------
echo "=== QEMU Boot: OSAL FreeRTOS MPS2 Cortex-M3 ==="
echo "ELF: $ELF"
echo "Timeout: ${TIMEOUT_SEC}s"
echo ""

set +e
timeout "$TIMEOUT_SEC" qemu-system-arm \
    -machine mps2-an385 \
    -cpu cortex-m3 \
    -kernel "$ELF" \
    -monitor none \
    -nographic \
    -serial stdio \
    -semihosting \
    -no-reboot \
    > "$LOG" 2>&1
QEMU_EXIT=$?
set -e

echo ""
echo "--- QEMU output ---"
cat "$LOG"
echo "--- End QEMU output ---"
echo ""

# QEMU must exit cleanly via semihosting.  Timeout or crash = failure.
if [ "$QEMU_EXIT" -ne 0 ]; then
    echo "ERROR: QEMU exited with code $QEMU_EXIT (expected 0)" >&2
    exit 1
fi

echo "QEMU exited normally (code 0)"

# ------------------------------------------------------------------
# Verify UART boot protocol
# ------------------------------------------------------------------
PROFILE="${PROFILE:-}"
VERIFY_ARGS=("$LOG")
if [ -n "$PROFILE" ]; then
    VERIFY_ARGS+=(--profile "$PROFILE")
fi
echo ""
python3 "$SCRIPT_DIR/verify-boot.py" "${VERIFY_ARGS[@]}"
VERIFY_EXIT=$?

if [ $VERIFY_EXIT -eq 0 ]; then
    echo "=== Boot verification PASSED ==="
    exit 0
else
    echo "=== Boot verification FAILED ==="
    exit 1
fi
