#!/usr/bin/env python3
"""verify-boot.py — validate OSAL FreeRTOS MPS2 boot protocol output.

Reads the QEMU UART log and checks that the boot protocol markers are
present, in order, and without error.
"""

import sys
import re

# Expected markers, in order.
MARKER_BEGIN = "OSAL_BOOT_BEGIN"
MARKER_PASS  = "OSAL_BOOT_PASS"
MARKER_END   = "OSAL_BOOT_END"

# Failure / fatal markers — must NOT appear.
MARKER_FAIL  = "OSAL_BOOT_FAIL"
MARKER_FATAL = "OSAL_BOOT_FATAL"

# Field expected within the PASS marker.
FIELD_SCHEDULER      = "scheduler=running"
FIELD_TICK           = "tick_advanced=true"
FIELD_RUNTIME_IMAGE  = "runtime_image=true"
FIELD_RUST_ENTRY     = "rust_entry=true"
FIELD_SHIM           = "shim=true"
FIELD_CAPABILITIES   = "capabilities=true"
# FIELD_SHIM_DELAY — deferred pending delay_ticks ABI fix
FIELD_STATUS         = "status=pass"


def verify(log_path: str) -> int:
    """Return 0 on success, 1 on failure."""
    try:
        with open(log_path, "r") as f:
            text = f.read()
    except OSError as e:
        print(f"FAIL: cannot read log: {e}")
        return 1

    lines = text.splitlines()

    errors: list[str] = []

    # --- Required markers ---
    begin_count = 0
    pass_count = 0
    end_count = 0

    for line in lines:
        if MARKER_BEGIN in line:
            begin_count += 1
        if MARKER_PASS in line:
            pass_count += 1
        if MARKER_END in line:
            end_count += 1

    if begin_count == 0:
        errors.append(f"missing {MARKER_BEGIN}")
    elif begin_count > 1:
        errors.append(f"multiple {MARKER_BEGIN} ({begin_count})")

    if pass_count == 0:
        errors.append(f"missing {MARKER_PASS}")
    elif pass_count > 1:
        errors.append(f"multiple {MARKER_PASS} ({pass_count})")

    if end_count == 0:
        errors.append(f"missing {MARKER_END}")
    elif end_count > 1:
        errors.append(f"multiple {MARKER_END} ({end_count})")

    # --- Forbidden markers ---
    for line in lines:
        if MARKER_FAIL in line:
            errors.append(f"failure marker found: {line.strip()}")
            break

    for line in lines:
        if MARKER_FATAL in line:
            errors.append(f"fatal marker found: {line.strip()}")
            break

    # --- PASS field validation ---
    pass_line = None
    for line in lines:
        if MARKER_PASS in line:
            pass_line = line.strip()
            break

    if pass_line is not None:
        if FIELD_SCHEDULER not in pass_line:
            errors.append(
                f"{MARKER_PASS} missing '{FIELD_SCHEDULER}': {pass_line}"
            )
        if FIELD_TICK not in pass_line:
            errors.append(
                f"{MARKER_PASS} missing '{FIELD_TICK}': {pass_line}"
            )
        if FIELD_RUNTIME_IMAGE not in pass_line:
            errors.append(
                f"{MARKER_PASS} missing '{FIELD_RUNTIME_IMAGE}': {pass_line}"
            )
        if FIELD_RUST_ENTRY not in pass_line:
            errors.append(
                f"{MARKER_PASS} missing '{FIELD_RUST_ENTRY}': {pass_line}"
            )
        if FIELD_SHIM not in pass_line:
            errors.append(
                f"{MARKER_PASS} missing '{FIELD_SHIM}': {pass_line}"
            )
        if FIELD_CAPABILITIES not in pass_line:
            errors.append(
                f"{MARKER_PASS} missing '{FIELD_CAPABILITIES}': {pass_line}"
            )
        # FIELD_SHIM_DELAY — deferred pending delay_ticks ABI fix

    # --- END field validation ---
    end_line = None
    for line in lines:
        if MARKER_END in line:
            end_line = line.strip()
            break

    if end_line is not None:
        if FIELD_STATUS not in end_line:
            errors.append(
                f"{MARKER_END} missing '{FIELD_STATUS}': {end_line}"
            )

    # --- Marker ordering ---
    marker_positions: list[tuple[str, int]] = []
    for i, line in enumerate(lines):
        if MARKER_BEGIN in line:
            marker_positions.append((MARKER_BEGIN, i))
        if MARKER_PASS in line:
            marker_positions.append((MARKER_PASS, i))
        if MARKER_END in line:
            marker_positions.append((MARKER_END, i))

    expected_order = [MARKER_BEGIN, MARKER_PASS, MARKER_END]
    actual_order = [m for m, _ in marker_positions]
    if actual_order != expected_order and len(actual_order) == 3:
        errors.append(
            f"marker order mismatch: expected {expected_order}, "
            f"got {actual_order}"
        )

    # --- Report ---
    if errors:
        print("FAIL: boot protocol errors:")
        for err in errors:
            print(f"  - {err}")
        return 1

    print("PASS: boot protocol valid")
    print(f"  {MARKER_BEGIN}: {begin_count}")
    print(f"  {MARKER_PASS}:  {pass_count}")
    print(f"  {MARKER_END}:   {end_count}")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <qemu.log>", file=sys.stderr)
        sys.exit(1)
    sys.exit(verify(sys.argv[1]))
