# Makefile — OSAL FreeRTOS QEMU MPS2 Cortex-M3 boot firmware.
#
# Usage:
#   make          Build the ELF
#   make clean    Remove build artifacts
#   make verify   Build and run QEMU boot test

# ------------------------------------------------------------------
# Toolchain
# ------------------------------------------------------------------
CC      = arm-none-eabi-gcc
SIZE    = arm-none-eabi-size
NM      = arm-none-eabi-nm
QEMU    = qemu-system-arm

# ------------------------------------------------------------------
# Paths
# ------------------------------------------------------------------
ROOT       = ../..
KERNEL     = $(ROOT)/third_party/freertos-kernel
MPS2_REF   = $(ROOT)/third_party/mps2-an385-reference
BUILD      = build

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
# Project source files
# ------------------------------------------------------------------
PROJECT_SRC  = app/main.c
PROJECT_SRC += app/hooks.c
PROJECT_SRC += bsp/console.c
PROJECT_SRC += bsp/platform.c
PROJECT_SRC += bsp/qemu_exit.c

# ------------------------------------------------------------------
# Third-party source files
# ------------------------------------------------------------------
VENDOR_SRC  = $(KERNEL)/tasks.c
VENDOR_SRC += $(KERNEL)/list.c
VENDOR_SRC += $(KERNEL)/queue.c
VENDOR_SRC += $(KERNEL)/event_groups.c
VENDOR_SRC += $(KERNEL)/portable/MemMang/heap_4.c
VENDOR_SRC += $(KERNEL)/portable/GCC/ARM_CM3/port.c
VENDOR_SRC += $(MPS2_REF)/startup_gcc.c

# ------------------------------------------------------------------
# Objects
# ------------------------------------------------------------------
PROJECT_OBJS = $(PROJECT_SRC:%.c=$(BUILD)/%.o)
VENDOR_OBJS  = $(VENDOR_SRC:%.c=$(BUILD)/vendor/%.o)

ELF  = $(BUILD)/freertos-qemu-mps2.elf
MAP  = $(BUILD)/freertos-qemu-mps2.map
SIZE_FILE = $(BUILD)/freertos-qemu-mps2.size.txt

# ------------------------------------------------------------------
# Targets
# ------------------------------------------------------------------
.PHONY: all clean verify

all: $(ELF) size

$(ELF): $(PROJECT_OBJS) $(VENDOR_OBJS)
	@mkdir -p $(BUILD)
	$(CC) $(PROJECT_CFLAGS) $(LDFLAGS) $^ -o $@

$(BUILD)/%.o: %.c
	@mkdir -p $(dir $@)
	$(CC) $(PROJECT_CFLAGS) $(INCLUDES) -c $< -o $@

$(BUILD)/vendor/%.o: %.c
	@mkdir -p $(dir $@)
	$(CC) $(VENDOR_CFLAGS) $(INCLUDES) -c $< -o $@

size: $(ELF)
	$(SIZE) $(ELF) | tee $(SIZE_FILE)

clean:
	rm -rf $(BUILD)

# ------------------------------------------------------------------
# Symbol check — must always pass.
# ------------------------------------------------------------------
check-symbols: $(ELF)
	@echo "=== Checking required symbols ==="
	@for sym in Reset_Handler main vTaskStartScheduler \
	            xPortPendSVHandler xPortSysTickHandler \
	            vPortSVCHandler boot_task; do \
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
	@echo "=== Symbol check passed ==="

# ------------------------------------------------------------------
# Verify: build + check symbols
# ------------------------------------------------------------------
verify: all check-symbols
	@echo "=== Build + symbol check passed ==="
