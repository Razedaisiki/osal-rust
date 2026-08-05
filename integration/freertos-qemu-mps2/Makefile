# Makefile — OSAL FreeRTOS QEMU MPS2 Cortex-M3 boot firmware.
#
# Usage:
#   make          Build the ELF
#   make clean    Remove build artifacts
#   make verify   Build firmware and check symbols

# ------------------------------------------------------------------
# Toolchain
# ------------------------------------------------------------------
CC      = arm-none-eabi-gcc
AR      = arm-none-eabi-ar
CARGO   = cargo
SIZE    = arm-none-eabi-size
NM      = arm-none-eabi-nm
QEMU    = qemu-system-arm

# ------------------------------------------------------------------
# Paths
# ------------------------------------------------------------------
ROOT       = ../..
KERNEL     = $(ROOT)/third_party/freertos-kernel
MPS2_REF   = $(ROOT)/third_party/mps2-an385-reference
RUST_DIR   = rust
BUILD      = build

# The Rust staticlib is built with cargo into its own target directory.
RUST_TARGET_DIR = $(RUST_DIR)/target
RUST_LIB        = $(RUST_TARGET_DIR)/thumbv7m-none-eabi/debug/libosal_integration_freertos_qemu_mps2.a

# ------------------------------------------------------------------
# Flags — project source (strict)
# ------------------------------------------------------------------
PROJECT_CFLAGS  = -mthumb
PROJECT_CFLAGS += -mcpu=cortex-m3
PROJECT_CFLAGS += -ffreestanding
PROJECT_CFLAGS += -ffunction-sections
PROJECT_CFLAGS += -fdata-sections
PROJECT_CFLAGS += -fno-common
PROJECT_CFLAGS += -std=gnu11
PROJECT_CFLAGS += -Wall
PROJECT_CFLAGS += -Wextra
PROJECT_CFLAGS += -Werror
PROJECT_CFLAGS += -Wshadow
PROJECT_CFLAGS += -Wundef

# ------------------------------------------------------------------
# Flags — third-party source (relaxed — no -Werror)
# ------------------------------------------------------------------
VENDOR_CFLAGS   = -mthumb
VENDOR_CFLAGS  += -mcpu=cortex-m3
VENDOR_CFLAGS  += -ffreestanding
VENDOR_CFLAGS  += -ffunction-sections
VENDOR_CFLAGS  += -fdata-sections
VENDOR_CFLAGS  += -fno-common
VENDOR_CFLAGS  += -std=gnu11
VENDOR_CFLAGS  += -Wall
VENDOR_CFLAGS  += -Wextra
VENDOR_CFLAGS  += -Wshadow

# ------------------------------------------------------------------
# Include paths
# ------------------------------------------------------------------
INCLUDES  = -I config
INCLUDES += -I bsp
INCLUDES += -I app
INCLUDES += -I $(KERNEL)/include
INCLUDES += -I $(KERNEL)/portable/GCC/ARM_CM3
INCLUDES += -I $(MPS2_REF)
INCLUDES += -I $(MPS2_REF)/CMSIS

# ------------------------------------------------------------------
# Linker
# ------------------------------------------------------------------
LDFLAGS  = -T link/mps2_m3.ld
LDFLAGS += -nostartfiles
LDFLAGS += -specs=nano.specs
LDFLAGS += -specs=nosys.specs
LDFLAGS += -Wl,--gc-sections
LDFLAGS += -Wl,-Map=$(BUILD)/freertos-qemu-mps2.map

# ------------------------------------------------------------------
# Objects — all flat in build/
# ------------------------------------------------------------------
PROJECT_OBJS = $(BUILD)/main.o $(BUILD)/hooks.o $(BUILD)/console.o \
               $(BUILD)/platform.o $(BUILD)/qemu_exit.o \
               $(BUILD)/startup.o $(BUILD)/test_task.o
VENDOR_OBJS  = $(BUILD)/tasks.o $(BUILD)/list.o $(BUILD)/queue.o \
               $(BUILD)/event_groups.o $(BUILD)/heap_4.o \
               $(BUILD)/port.o

ELF  = $(BUILD)/freertos-qemu-mps2.elf
MAP  = $(BUILD)/freertos-qemu-mps2.map
SIZE_FILE = $(BUILD)/freertos-qemu-mps2.size.txt

# ------------------------------------------------------------------
# Targets
# ------------------------------------------------------------------
.PHONY: all clean verify

.DEFAULT_GOAL := all

# Ensure build directory exists before compiling.
$(BUILD):
	@mkdir -p $(BUILD)

.PHONY: rust-staticlib

all: $(ELF) size

# Feature flags for the Rust staticlib.
#   make                          → suite-aggregate (default)
#   make CARGO_FEATURES=suite-queue-blocking → queue-blocking
#   make CARGO_FEATURES=suite-task           → task
CARGO_FEATURES ?= default

# Task profile: enable integration diagnostics in both the sys shim and
# the integration C files (test_task.c provides the observer definitions).
ifeq ($(CARGO_FEATURES),suite-task)
PROJECT_CFLAGS += -DOSAL_FREERTOS_INTEGRATION_DIAGNOSTICS=1
endif

# Pass the ARM cross-compiler and FreeRTOS include paths so the
# -sys crate build.rs compiles the C shim with arm-none-eabi-gcc.
RUST_BUILD_ENV  = CC_thumbv7m_none_eabi=$(CC)
RUST_BUILD_ENV += AR_thumbv7m_none_eabi=$(AR)

# Base CFLAGS for the C shim — extended per profile below.
RUST_BASE_CFLAGS = -mthumb -mcpu=cortex-m3 -ffreestanding -ffunction-sections -fdata-sections

# Task profile: enable integration diagnostics observers in the sys shim.
ifeq ($(CARGO_FEATURES),suite-task)
RUST_BUILD_ENV += CFLAGS_thumbv7m_none_eabi="$(RUST_BASE_CFLAGS) -DOSAL_FREERTOS_INTEGRATION_DIAGNOSTICS=1"
else
RUST_BUILD_ENV += CFLAGS_thumbv7m_none_eabi="$(RUST_BASE_CFLAGS)"
endif
RUST_BUILD_ENV += OSAL_FREERTOS_KERNEL_INCLUDE=$(abspath $(KERNEL)/include)
RUST_BUILD_ENV += OSAL_FREERTOS_CONFIG_INCLUDE=$(abspath config)
RUST_BUILD_ENV += OSAL_FREERTOS_PORT_INCLUDE=$(abspath $(KERNEL)/portable/GCC/ARM_CM3)

CARGO_BUILD_LOG := $(abspath $(BUILD)/cargo-build.log)

# Always invoke cargo — let it decide whether rebuilding is needed.
# Avoids silent stale-Rust-in-ELF when src/lib.rs changed across runs.
rust-staticlib: | $(BUILD)
	@set -eu; \
	cd $(RUST_DIR); \
	FEATURE_FLAGS=$$(if [ "$(CARGO_FEATURES)" = "default" ]; then echo ""; else echo "--no-default-features --features $(CARGO_FEATURES)"; fi); \
	env $(RUST_BUILD_ENV) $(CARGO) build -vv $$FEATURE_FLAGS \
	    >"$(CARGO_BUILD_LOG)" 2>&1 || { \
	        status=$$?; \
	        cat "$(CARGO_BUILD_LOG)"; \
	        exit $$status; \
	    }; \
	cat "$(CARGO_BUILD_LOG)"
	@echo "  cargo-build.log written"

$(ELF): $(PROJECT_OBJS) $(VENDOR_OBJS) rust-staticlib
	@test -f "$(RUST_LIB)" || { echo "ERROR: Rust staticlib not found: $(RUST_LIB)" >&2; exit 1; }
	$(CC) $(PROJECT_CFLAGS) $(LDFLAGS) $(PROJECT_OBJS) $(VENDOR_OBJS) $(RUST_LIB) -o $@

# Project: object in build/, source in same-named subdirectory or .
$(BUILD)/main.o: app/main.c | $(BUILD)
	$(CC) $(PROJECT_CFLAGS) $(INCLUDES) -c $< -o $@
$(BUILD)/hooks.o: app/hooks.c | $(BUILD)
	$(CC) $(PROJECT_CFLAGS) $(INCLUDES) -c $< -o $@
$(BUILD)/console.o: bsp/console.c | $(BUILD)
	$(CC) $(PROJECT_CFLAGS) $(INCLUDES) -c $< -o $@
$(BUILD)/platform.o: bsp/platform.c | $(BUILD)
	$(CC) $(PROJECT_CFLAGS) $(INCLUDES) -c $< -o $@
$(BUILD)/qemu_exit.o: bsp/qemu_exit.c | $(BUILD)
	$(CC) $(PROJECT_CFLAGS) $(INCLUDES) -c $< -o $@
$(BUILD)/startup.o: bsp/startup.c | $(BUILD)
	$(CC) $(PROJECT_CFLAGS) $(INCLUDES) -c $< -o $@
$(BUILD)/test_task.o: app/test_task.c | $(BUILD)
	$(CC) $(PROJECT_CFLAGS) $(INCLUDES) -c $< -o $@

# Vendor: object in build/, source in third_party/ via absolute include path
$(BUILD)/tasks.o: $(KERNEL)/tasks.c | $(BUILD)
	$(CC) $(VENDOR_CFLAGS) $(INCLUDES) -c $< -o $@
$(BUILD)/list.o: $(KERNEL)/list.c | $(BUILD)
	$(CC) $(VENDOR_CFLAGS) $(INCLUDES) -c $< -o $@
$(BUILD)/queue.o: $(KERNEL)/queue.c | $(BUILD)
	$(CC) $(VENDOR_CFLAGS) $(INCLUDES) -c $< -o $@
$(BUILD)/event_groups.o: $(KERNEL)/event_groups.c | $(BUILD)
	$(CC) $(VENDOR_CFLAGS) $(INCLUDES) -c $< -o $@
$(BUILD)/heap_4.o: $(KERNEL)/portable/MemMang/heap_4.c | $(BUILD)
	$(CC) $(VENDOR_CFLAGS) $(INCLUDES) -c $< -o $@
$(BUILD)/port.o: $(KERNEL)/portable/GCC/ARM_CM3/port.c | $(BUILD)
	$(CC) $(VENDOR_CFLAGS) $(INCLUDES) -c $< -o $@
size: $(ELF)
	$(SIZE) $(ELF) | tee $(SIZE_FILE)

clean:
	rm -rf $(BUILD)
	cd $(RUST_DIR) && $(CARGO) clean

# ------------------------------------------------------------------
# Symbol check — must always pass.
# ------------------------------------------------------------------
check-symbols:
	@echo "=== Checking required symbols ==="
	@for sym in Reset_Handler main vTaskStartScheduler \
	            xPortPendSVHandler xPortSysTickHandler \
	            vPortSVCHandler boot_task \
	            osal_rust_smoke_entry \
	            osal_test_object_entry \
	            osal_test_harness_set_phase \
	            osal_test_harness_set_result \
	            osal_test_harness_record_start \
	            osal_test_harness_record_end \
	            osal_freertos_probe_capabilities \
	            osal_freertos_scheduler_state \
	            osal_freertos_tick_snapshot \
	            osal_freertos_delay_ticks \
	            osal_freertos_heap_alloc \
	            osal_freertos_heap_dealloc \
	            osal_test_task_spawn \
	            osal_test_task_exit \
	            osal_test_console_line \
	            harness_smoke_helper \
	            pvPortMalloc vPortFree; do \
	    if $(NM) $(ELF) | grep -q " [tT] $$sym"; then \
	        echo "  OK   $$sym"; \
	    else \
	        echo "  MISS $$sym"; exit 1; \
	    fi; \
	done
	@for sym in main_blinky main_full; do \
	    if $(NM) $(ELF) | grep -q " [tT] $$sym"; then \
	        echo "  UNEXPECTED $$sym"; exit 1; \
	    fi; \
	done
	@echo "=== Linker script symbols ==="
	@for sym in _sidata _sdata _edata _sbss _ebss; do \
	    if $(NM) $(ELF) | grep -q " $$sym$$"; then \
	        echo "  OK   $$sym"; \
	    else \
	        echo "  MISS $$sym"; exit 1; \
	    fi; \
	done
	@echo "=== Symbol check passed ==="

# ------------------------------------------------------------------
# Verify: build + check symbols
# ------------------------------------------------------------------
verify: all check-symbols
	@echo "=== Build + symbol check passed ==="
