/* console.c — minimal MPS2 UART0 console.
 *
 * MPS2-AN385 UART0 registers:
 *   DATA   @ base + 0x00
 *   STATE  @ base + 0x04  (bit 0 = TX full, bit 1 = RX full)
 *   CTRL   @ base + 0x08  (bit 0 = TX enable, bit 1 = RX enable)
 *   BAUDDIV @ base + 0x10
 *
 * No printf, no malloc, no newlib buffering.
 */

#include "console.h"
#include <stddef.h>
#include <stdint.h>

/* UART0 base address on MPS2-AN385. */
#define UART0_BASE  0x40004000UL

#define UART_DATA   (*(volatile uint32_t *)(UART0_BASE + 0x00U))
#define UART_STATE  (*(volatile uint32_t *)(UART0_BASE + 0x04U))
#define UART_CTRL   (*(volatile uint32_t *)(UART0_BASE + 0x08U))
#define UART_BAUDDIV (*(volatile uint32_t *)(UART0_BASE + 0x10U))

/* STATE register bits. */
#define UART_STATE_TX_FULL  (1U << 0)
#define UART_STATE_RX_FULL  (1U << 1)

/* CTRL register bits. */
#define UART_CTRL_TX_ENABLE (1U << 0)
#define UART_CTRL_RX_ENABLE (1U << 1)

/* Baud rate divider for 25 MHz → 115200 baud (approx). */
#define UART_BAUD_DIVISOR   15U

void console_init(void)
{
    UART_BAUDDIV = UART_BAUD_DIVISOR;
    UART_CTRL    = UART_CTRL_TX_ENABLE | UART_CTRL_RX_ENABLE;
}

void console_write_byte(char value)
{
    /* Wait until TX not full. */
    while (UART_STATE & UART_STATE_TX_FULL) {
        /* spin */
    }
    UART_DATA = (uint32_t)(unsigned char)value;
}

void console_write(const char *text)
{
    if (text == NULL) {
        return;
    }

    while (*text != '\0') {
        if (*text == '\n') {
            console_write_byte('\r');
        }
        console_write_byte(*text);
        text++;
    }
}

void console_write_line(const char *text)
{
    console_write(text);
    console_write_byte('\r');
    console_write_byte('\n');
}
