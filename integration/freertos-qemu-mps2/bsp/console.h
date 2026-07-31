/* console.h — minimal MPS2 UART0 console. */

#ifndef CONSOLE_H
#define CONSOLE_H

void console_init(void);
void console_write_byte(char value);
void console_write(const char *text);
void console_write_line(const char *text);

#endif /* CONSOLE_H */
