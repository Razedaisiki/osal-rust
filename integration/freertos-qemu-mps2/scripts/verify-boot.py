#!/usr/bin/env python3
"""verify-boot.py — validate OSAL FreeRTOS MPS2 boot and object protocols.

Reads the QEMU UART log and checks that the boot protocol markers and
object protocol markers are present, in order, and without error.
"""

import sys

# ------------------------------------------------------------------
# Boot protocol (P7G Step 2 + 3B + 3C)
# ------------------------------------------------------------------
MARKER_BOOT_BEGIN = "OSAL_BOOT_BEGIN"
MARKER_BOOT_PASS  = "OSAL_BOOT_PASS"
MARKER_BOOT_END   = "OSAL_BOOT_END"

MARKER_BOOT_FAIL  = "OSAL_BOOT_FAIL"
MARKER_BOOT_FATAL = "OSAL_BOOT_FATAL"

REQUIRED_PASS_FIELDS = [
    "scheduler=running",
    "tick_advanced=true",
    "runtime_image=true",
    "rust_entry=true",
    "shim=true",
    "capabilities=true",
    "shim_delay=true",
    "allocator=true",
    "runtime_lifecycle=true",
    "runtime_lease=true",
    "mutex=true",
    "heap_recovered=true",
    "lifecycle_cycles=8",
]

FIELD_STATUS = "status=pass"

# ------------------------------------------------------------------
# Object protocol (P7G Step 4)
# ------------------------------------------------------------------
MARKER_OBJECT_BEGIN = "OSAL_OBJECT_BEGIN"
MARKER_OBJECT_PASS  = "OSAL_OBJECT_PASS"
MARKER_OBJECT_END   = "OSAL_OBJECT_END"

MARKER_CASE_PASS = "OSAL_CASE_PASS"
MARKER_CASE_FAIL = "OSAL_CASE_FAIL"

# Each completed sub-step adds its case name here.
REQUIRED_CASES = [
    "harness_native_task",
    "mutex_basic_clone",
    "mutex_non_recursive",
    "mutex_nowait_zero",
    "mutex_finite_timeout",
    "mutex_blocking_wake",
    "mutex_forever_wake",
    "mutex_scheduler_suspended",
    "mutex_runtime_lease",
    "counting_core",
    "counting_overflow",
    "counting_nowait_zero",
    "counting_finite_timeout",
    "counting_clone",
    "counting_blocking_wake",
    "counting_forever_wake",
    "counting_one_release_one_waiter",
    "counting_permit_accounting",
    "binary_core",
    "binary_overflow",
    "binary_nowait_zero",
    "binary_blocking_wake",
    "binary_forever_wake",
    "binary_two_waiters",
    "binary_clone",
    "semaphore_scheduler_suspended",
    "semaphore_runtime_lease",
]

REQUIRED_OBJECT_PASS_FIELDS = [
    "harness=true",
    "helper_self_delete=true",
    "idle_cleanup=true",
    "heap_recovered=true",
    "multi_helper=true",
    "tick_advance=true",
    "mutex=true",
    "mutex_clone=true",
    "mutex_timeout=true",
    "mutex_nowait=true",
    "mutex_blocking=true",
    "mutex_suspended=true",
    "mutex_lease=true",
    "semaphore=true",
    "counting=true",
    "semaphore_timeout=true",
    "semaphore_blocking=true",
    "semaphore_multi_waiter=true",
    "binary=true",
    "semaphore_suspended=true",
    "semaphore_lease=true",
]

OBJECT_FIELD_STATUS = "status=pass"


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

    # ==============================================================
    # Boot protocol
    # ==============================================================

    begin_count = 0
    pass_count = 0
    end_count = 0

    for line in lines:
        if MARKER_BOOT_BEGIN in line:
            begin_count += 1
        if MARKER_BOOT_PASS in line:
            pass_count += 1
        if MARKER_BOOT_END in line:
            end_count += 1

    if begin_count == 0:
        errors.append(f"missing {MARKER_BOOT_BEGIN}")
    elif begin_count > 1:
        errors.append(f"multiple {MARKER_BOOT_BEGIN} ({begin_count})")

    if pass_count == 0:
        errors.append(f"missing {MARKER_BOOT_PASS}")
    elif pass_count > 1:
        errors.append(f"multiple {MARKER_BOOT_PASS} ({pass_count})")

    if end_count == 0:
        errors.append(f"missing {MARKER_BOOT_END}")
    elif end_count > 1:
        errors.append(f"multiple {MARKER_BOOT_END} ({end_count})")

    # Forbidden boot markers
    for line in lines:
        if MARKER_BOOT_FAIL in line:
            errors.append(f"failure marker found: {line.strip()}")
            break

    for line in lines:
        if MARKER_BOOT_FATAL in line:
            errors.append(f"fatal marker found: {line.strip()}")
            break

    # PASS field validation
    pass_line = None
    for line in lines:
        if MARKER_BOOT_PASS in line:
            pass_line = line.strip()
            break

    if pass_line is not None:
        for field in REQUIRED_PASS_FIELDS:
            if field not in pass_line:
                errors.append(
                    f"{MARKER_BOOT_PASS} missing '{field}': {pass_line}"
                )

    # END field validation
    end_line = None
    for line in lines:
        if MARKER_BOOT_END in line:
            end_line = line.strip()
            break

    if end_line is not None:
        if FIELD_STATUS not in end_line:
            errors.append(
                f"{MARKER_BOOT_END} missing '{FIELD_STATUS}': {end_line}"
            )

    # Boot marker ordering
    boot_positions: list[tuple[str, int]] = []
    for i, line in enumerate(lines):
        if MARKER_BOOT_BEGIN in line:
            boot_positions.append((MARKER_BOOT_BEGIN, i))
        if MARKER_BOOT_PASS in line:
            boot_positions.append((MARKER_BOOT_PASS, i))
        if MARKER_BOOT_END in line:
            boot_positions.append((MARKER_BOOT_END, i))

    expected_boot_order = [MARKER_BOOT_BEGIN, MARKER_BOOT_PASS, MARKER_BOOT_END]
    actual_boot_order = [m for m, _ in boot_positions]
    if actual_boot_order != expected_boot_order and len(actual_boot_order) == 3:
        errors.append(
            f"marker order mismatch: expected {expected_boot_order}, "
            f"got {actual_boot_order}"
        )

    # ==============================================================
    # Object protocol (P7G Step 4)
    # ==============================================================

    obj_begin_count = 0
    obj_pass_count = 0
    obj_end_count = 0
    case_pass_lines: list[str] = []

    # Also capture line indices for position checks.
    obj_begin_idx: int | None = None
    obj_pass_idx: int | None = None
    obj_end_idx: int | None = None
    case_pass_indices: list[tuple[int, str]] = []

    for i, line in enumerate(lines):
        if MARKER_OBJECT_BEGIN in line:
            obj_begin_count += 1
            if obj_begin_idx is None:
                obj_begin_idx = i
        if MARKER_OBJECT_PASS in line:
            obj_pass_count += 1
            if obj_pass_idx is None:
                obj_pass_idx = i
        if MARKER_OBJECT_END in line:
            obj_end_count += 1
            if obj_end_idx is None:
                obj_end_idx = i
        if MARKER_CASE_PASS in line:
            case_pass_lines.append(line.strip())
            case_pass_indices.append((i, line.strip()))

    if obj_begin_count == 0:
        errors.append(f"missing {MARKER_OBJECT_BEGIN}")
    elif obj_begin_count > 1:
        errors.append(f"multiple {MARKER_OBJECT_BEGIN} ({obj_begin_count})")

    if obj_pass_count == 0:
        errors.append(f"missing {MARKER_OBJECT_PASS}")
    elif obj_pass_count > 1:
        errors.append(f"multiple {MARKER_OBJECT_PASS} ({obj_pass_count})")

    if obj_end_count == 0:
        errors.append(f"missing {MARKER_OBJECT_END}")
    elif obj_end_count > 1:
        errors.append(f"multiple {MARKER_OBJECT_END} ({obj_end_count})")

    # Forbidden case fail markers
    for line in lines:
        if MARKER_CASE_FAIL in line:
            errors.append(f"case failure marker found: {line.strip()}")
            break

    # Object marker ordering
    obj_positions: list[tuple[str, int]] = []
    for i, line in enumerate(lines):
        if MARKER_OBJECT_BEGIN in line:
            obj_positions.append((MARKER_OBJECT_BEGIN, i))
        if MARKER_OBJECT_PASS in line:
            obj_positions.append((MARKER_OBJECT_PASS, i))
        if MARKER_OBJECT_END in line:
            obj_positions.append((MARKER_OBJECT_END, i))

    expected_obj_order = [MARKER_OBJECT_BEGIN, MARKER_OBJECT_PASS, MARKER_OBJECT_END]
    actual_obj_order = [m for m, _ in obj_positions]
    if actual_obj_order != expected_obj_order and len(actual_obj_order) == 3:
        errors.append(
            f"object marker order mismatch: expected {expected_obj_order}, "
            f"got {actual_obj_order}"
        )

    # Verify object protocol comes after boot protocol
    if boot_positions and obj_positions:
        last_boot_pos = max(pos for _, pos in boot_positions)
        first_obj_pos = min(pos for _, pos in obj_positions)
        if first_obj_pos <= last_boot_pos:
            errors.append(
                "object protocol must follow boot protocol"
            )

    # --- CASE_PASS: extract names, validate position, reject unknown ---
    required_set: set[str] = set(REQUIRED_CASES)
    seen_cases: dict[str, int] = {}          # name → line index

    for idx, cl in case_pass_indices:
        # Must have a name= token
        name_token = None
        for token in cl.split():
            if token.startswith("name="):
                name_token = token
                break

        if name_token is None:
            errors.append(
                f"{MARKER_CASE_PASS} missing 'name=' at line {idx + 1}: {cl}"
            )
            continue

        case_name = name_token.split("=", 1)[1]
        if not case_name:
            errors.append(
                f"{MARKER_CASE_PASS} empty 'name=' at line {idx + 1}: {cl}"
            )
            continue

        # Reject unknown cases
        if case_name not in required_set:
            errors.append(
                f"unknown {MARKER_CASE_PASS} '{case_name}' at line {idx + 1}"
            )
            continue

        # Reject duplicates
        if case_name in seen_cases:
            errors.append(
                f"duplicate {MARKER_CASE_PASS} '{case_name}' "
                f"at lines {seen_cases[case_name] + 1} and {idx + 1}"
            )
            continue

        seen_cases[case_name] = idx

        # Position: must be between OBJECT_BEGIN and OBJECT_PASS
        if obj_begin_idx is not None and idx <= obj_begin_idx:
            errors.append(
                f"{MARKER_CASE_PASS} '{case_name}' at line {idx + 1} "
                f"must follow {MARKER_OBJECT_BEGIN} (line {obj_begin_idx + 1})"
            )
        if obj_pass_idx is not None and idx >= obj_pass_idx:
            errors.append(
                f"{MARKER_CASE_PASS} '{case_name}' at line {idx + 1} "
                f"must precede {MARKER_OBJECT_PASS} (line {obj_pass_idx + 1})"
            )

    # Each required case must appear exactly once
    for required in REQUIRED_CASES:
        if required not in seen_cases:
            errors.append(
                f"{MARKER_CASE_PASS} missing required case '{required}'"
            )

    # --- OBJECT_PASS field validation ---
    obj_pass_line = None
    for line in lines:
        if MARKER_OBJECT_PASS in line:
            obj_pass_line = line.strip()
            break

    if obj_pass_line is not None:
        for field in REQUIRED_OBJECT_PASS_FIELDS:
            if field not in obj_pass_line:
                errors.append(
                    f"{MARKER_OBJECT_PASS} missing '{field}': {obj_pass_line}"
                )

    # --- OBJECT_END status ---
    obj_end_line = None
    for line in lines:
        if MARKER_OBJECT_END in line:
            obj_end_line = line.strip()
            break

    if obj_end_line is not None:
        if OBJECT_FIELD_STATUS not in obj_end_line:
            errors.append(
                f"{MARKER_OBJECT_END} missing '{OBJECT_FIELD_STATUS}': {obj_end_line}"
            )

    # ==============================================================
    # Report
    # ==============================================================
    if errors:
        print("FAIL: boot/object protocol errors:")
        for err in errors:
            print(f"  - {err}")
        return 1

    print("PASS: boot and object protocols valid")
    print(f"  {MARKER_BOOT_BEGIN}: {begin_count}")
    print(f"  {MARKER_BOOT_PASS}:  {pass_count}")
    print(f"  {MARKER_BOOT_END}:   {end_count}")
    print(f"  {MARKER_OBJECT_BEGIN}: {obj_begin_count}")
    print(f"  {MARKER_OBJECT_PASS}:  {obj_pass_count}")
    print(f"  {MARKER_OBJECT_END}:   {obj_end_count}")
    for case in sorted(seen_cases):
        print(f"  {MARKER_CASE_PASS}: name={case}")
    return 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(f"Usage: {sys.argv[0]} <qemu.log>", file=sys.stderr)
        sys.exit(1)
    sys.exit(verify(sys.argv[1]))
