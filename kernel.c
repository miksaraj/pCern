#include <stddef.h>
#include <stdint.h>

// Basic checkss to make sure cross-compiler is used correctly
#if defined(__linux__)
#error "This code must be compiled with a cross-compiler"
#elif !defined(__i386__)
#error "This code must be compiled with an x86-elf compiler"
#endif

/* VGA provides support for 16 colors */
#define BLACK 0x0
#define GREEN 0x2
#define RED 0x4
#define YELLOW 0xE
#define WHITE_COLOR 0xF

// x86's VGA textmode buffer. To display text, write data to this memory location
volatile uint16_t* vga_buffer = (uint16_t*)0xB8000;

// Default VGA textmode buffer has size of 80x25 chars
const int VGA_COLS = 80;
const int VGA_ROWS = 25;

int term_col = 0;
int term_row = 0;
uint8_t term_color = 0x0F // Black bg, white fg

// Initiate term by clearing it
void term_init()
{
    // Clear textmode buffer
    for (int col = 0; col < VGA_COLS; ++col)
    {
        for (int row = 0; row < VGA_ROWS; ++row)
        {
            // VGA textmode buffer has size (VGA_COLS * VGA_ROWS)
            const size_t index = (VGA_COLS * row) + col;

            /*
             * VGA buffer entries take the binary form BBBB FFFF CCCC CCCC, where:
             * - B = bg color
             * - F = fg color
             * - C = ASCII char
             */
            vga_buffer[index] = ((uint16_t)term_color << 8) | ' ';
        }
    }
}

void inc_row()
{
    term_col = 0;
    ++term_row;
}

void term_putc(char c)
{
    switch (c)
    {
        case '\n': inc_row(); break;
        default:
        {
            const size_t index = (VGA_COLS * term_row) + term_col;
            vga_buffer[index] = ((uint16_t)term_color << 8) | c;
            ++term_col;
            break;
        }
    }

    if (term_col >= VGA_COLS) { inc_row(); } // If we get past last column, increment row
    if (term_row >= VGA_ROWS) { term_init(); term_col = 0; term_row = 0; } // Reset term if we get past last row
}

void term_print(const char* str)
{
    for (size_t i = 0; str[i] != '\0'; ++i) { term_putc(str[i]); }
}

void kernel_main()
{
    term_init();

    term_print("Hello from pCern - the minimal C pikokernel!\n");
    term_print("If you are seeing this, we are finally working...\n");

    inc_row();

    term_print("Hanging now. Goodbye!\n");
    return;
}
