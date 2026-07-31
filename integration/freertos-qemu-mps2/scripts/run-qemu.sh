#!/bin/bash
# run-qemu.sh — run the boot firmware on QEMU, capture output, verify.
#
# QEMU 4.2.1 does not reliably exit via semihosting on MPS2, so we use
# a hard timeout.  The verifier parses the output log and checks markers.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INTEG_DIR="$(dirname "$SCRIPT_DIR")"
BUILD_DIR="$INTEG_DIR/build"
ELF="$BUILD_DIR/freertos-qemu-mps2.elf"
LOG="$BUILD_DIR/qemu.log"
TIMEOUT_SEC=15

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

# QEMU exit codes:
#   0   = clean exit (semihosting worked)
#   124 = timeout (expected on QEMU < 6.0 without semihosting exit)
#   other = crash or error
case $QEMU_EXIT in
    0)   echo "QEMU exited normally (code 0)" ;;
    124) echo "QEMU stopped by timeout (expected on QEMU 4.x)" ;;
    *)   echo "QEMU exited with unexpected code: $QEMU_EXIT" ;;
esac

# ------------------------------------------------------------------
# Verify
# ------------------------------------------------------------------
echo ""
python3 "$SCRIPT_DIR/verify-boot.py" "$LOG"
VERIFY_EXIT=$?

if [ $VERIFY_EXIT -eq 0 ]; then
    echo "=== Boot verification PASSED ==="
    exit 0
else
    echo "=== Boot verification FAILED ==="
    exit 1
fi
