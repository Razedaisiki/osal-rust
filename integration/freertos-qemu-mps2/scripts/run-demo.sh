#!/bin/bash
# run-demo.sh — boot a portable OSAL demo on QEMU and check its protocol.
#
# Deliberately does NOT use verify-boot.py: that is the P7G conformance
# protocol. Demos have their own, much smaller, contract:
#
#   QEMU exit code 0
#   OSAL_DEMO_BEGIN name=<DEMO>
#   OSAL_DEMO_PASS  name=<DEMO>
#   OSAL_DEMO_END   status=pass
#
# and no OSAL_DEMO_FAIL / OSAL_BOOT_FAIL / OSAL_BOOT_FATAL marker.
#
# Environment:
#   DEMO       — demo name (required)
#   BUILD_DIR  — directory holding the ELF (default: <integ>/build/demo)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INTEG_DIR="$(dirname "$SCRIPT_DIR")"

DEMO="${DEMO:-}"
BUILD_DIR="${BUILD_DIR:-$INTEG_DIR/build/demo}"

TIMEOUT_SEC=30

if [ -z "$DEMO" ]; then
    echo "ERROR: DEMO is required (e.g. DEMO=queue)" >&2
    exit 1
fi

ELF="$BUILD_DIR/freertos-qemu-mps2.elf"
LOG="$BUILD_DIR/qemu-${DEMO}.log"

if [ ! -f "$ELF" ]; then
    echo "ERROR: ELF not found at $ELF" >&2
    echo "  Run: make demo DEMO=$DEMO" >&2
    exit 1
fi

echo "=== QEMU Portable Demo: $DEMO ==="
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

echo "--- QEMU output ---"
cat "$LOG"
echo "--- End QEMU output ---"
echo ""

FAILED=0

# 1. QEMU process exit is the final authority.
if [ "$QEMU_EXIT" -ne 0 ]; then
    echo "ERROR: QEMU exited with code $QEMU_EXIT (expected 0)" >&2
    FAILED=1
fi

# 2. Forbidden markers.
for marker in OSAL_DEMO_FAIL OSAL_BOOT_FAIL OSAL_BOOT_FATAL; do
    if grep -Fq "$marker" "$LOG"; then
        echo "ERROR: forbidden marker '$marker' present (see $LOG)" >&2
        FAILED=1
    fi
done

# 3. Required protocol markers.
for marker in \
    "OSAL_DEMO_BEGIN name=${DEMO}" \
    "OSAL_DEMO_PASS name=${DEMO}" \
    "OSAL_DEMO_END status=pass"
do
    if grep -Fq "$marker" "$LOG"; then
        echo "  OK   $marker"
    else
        echo "ERROR: missing '$marker' (see $LOG)" >&2
        FAILED=1
    fi
done

if [ "$FAILED" -ne 0 ]; then
    echo "=== Demo verification FAILED ($DEMO) ==="
    exit 1
fi

echo "=== Demo verification PASSED ($DEMO) ==="
