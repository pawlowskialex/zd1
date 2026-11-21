#include <sysclk.h>
#include <flashc.h>
#include <spi.h>
#include <delay.h>
#include "zd1_board.h"

uint32_t g_rcause;

/* The bootloader's DFU-mode pin state (bl.dis 0x80001654..), which cold-starts this board reliably:
 * all low except PB11/PB17 (USB mux -> MCU). Applied FIRST, on the reset RC clock, as both the bootloader
 * and stock do: with ASF's clocks first, a power-on boot stays dark [HW 2026-10-17]. */
void board_bl_pins(void) {
    static const uint8_t lo[] = {2, 14, 15, 16, 19, 22, 23, 29, 30, 33, 34, 35, 41, 44, 47, 48};
    for (unsigned i = 0; i < sizeof lo; i++)
        gpio_configure_pin(lo[i], GPIO_DIR_OUTPUT | GPIO_INIT_LOW);
    gpio_configure_pin(AVR32_PIN_PB11, GPIO_DIR_OUTPUT | GPIO_INIT_HIGH);
    gpio_configure_pin(AVR32_PIN_PB17, GPIO_DIR_OUTPUT | GPIO_INIT_HIGH);
    gpio_configure_pin(AVR32_PIN_PB19,
                       GPIO_DIR_OUTPUT | GPIO_INIT_LOW); /* stock drives it low right after PB27 (function unknown) */
}

/* Clock bring-up in the bootloader's order (OSC0 -> run from OSC0 -> PLL0 -> run from PLL0) instead of
 * ASF's direct RCSYS -> PLL0 switch. Ends in the same state ASF's sysclk_init gives for conf_clock.h
 * (PLL0 0x3f0f020d = 12*16/2/2 = 48 MHz, FWS=1), so sysclk_get_cpu_hz()/USB stay valid.
 * Bounded waits; a failure returns false, the stage | 0x80 told (the app's main logs it and blinks red on the RC
 * oscillator). */
static void reached(void (*stage)(uint8_t), uint8_t n) {
    if (stage)
        stage(n);
}
bool board_clock_init(void (*stage)(uint8_t)) {
    struct pll_config pll;
    uint32_t t;
    osc_enable(OSC_ID_OSC0); /* BOARD_OSC0_*: 12 MHz crystal -> OSCCTRL0 0x307 */
    for (t = 0; !osc_is_ready(OSC_ID_OSC0) && t < 2000000u; t++) {
    }
    if (!osc_is_ready(OSC_ID_OSC0)) {
        reached(stage, 0x82);
        return false;
    }
    reached(stage, 2);
    sysclk_set_source(SYSCLK_SRC_OSC0); /* run from OSC0 (12 MHz) first, like the bootloader */
    reached(stage, 3);
    pll_config_defaults(&pll, 0); /* conf_clock.h: 12*8/2 = 48 MHz -> PLL0 0x3f0f020d */
    pll_enable(&pll, 0);
    for (t = 0; !pll_is_locked(0) && t < 2000000u; t++) {
    }
    if (!pll_is_locked(0)) {
        reached(stage, 0x84);
        return false;
    }
    reached(stage, 4);
    flash_set_bus_freq(sysclk_get_cpu_hz()); /* FWS=1 above 33 MHz */
    sysclk_set_source(SYSCLK_SRC_PLL0);
    reached(stage, 5);
    return true;
}

/* Keep the amp OFF + muted and the DSP/module held, regardless of boot state. */
void board_safe_gpio(void) {
    gpio_configure_pin(PIN_AMP_EN, GPIO_DIR_OUTPUT | GPIO_INIT_LOW);
    gpio_configure_pin(PIN_AMP_UNMUTE, GPIO_DIR_OUTPUT | GPIO_INIT_LOW);
    gpio_configure_pin(PIN_DSP_PWR, GPIO_DIR_OUTPUT | GPIO_INIT_LOW);
    gpio_configure_pin(PIN_DSP_RST, GPIO_DIR_OUTPUT | GPIO_INIT_LOW);
    gpio_configure_pin(PIN_MOD_PWR, GPIO_DIR_OUTPUT | GPIO_INIT_LOW);
    gpio_configure_pin(PIN_MOD_RST, GPIO_DIR_OUTPUT | GPIO_INIT_LOW);
    gpio_configure_pin(PIN_LED_R, GPIO_DIR_OUTPUT | GPIO_INIT_LOW); /* red LED off until the PWM takes it */
}

/* ---- the module's power (zd1_board.h) ---- */
static const uint32_t spi_pins[] = {AVR32_PIN_PA09, AVR32_PIN_PA11, AVR32_PIN_PA12, AVR32_PIN_PA13};
void board_module_spi(void) {
    static const gpio_map_t pins = {
        {AVR32_SPI0_NPCS_2_0_PIN, AVR32_SPI0_NPCS_2_0_FUNCTION}, /* PA09 */
        {AVR32_SPI0_MISO_0_0_PIN, AVR32_SPI0_MISO_0_0_FUNCTION}, /* PA11 */
        {AVR32_SPI0_MOSI_0_0_PIN, AVR32_SPI0_MOSI_0_0_FUNCTION}, /* PA12 */
        {AVR32_SPI0_SCK_0_0_PIN, AVR32_SPI0_SCK_0_0_FUNCTION},   /* PA13 */
    };
    const spi_options_t opt = {.reg = 2,
                               .baudrate = 3000000,
                               .bits = 8,
                               .spck_delay = 0,
                               .trans_delay = 0,
                               .stay_act = 1,
                               .spi_mode = 3,
                               .modfdis = 1};
    gpio_set_pin_high(PIN_MOD_RST);
    sysclk_enable_pba_module(SYSCLK_SPI0);
    gpio_enable_module(pins, sizeof pins / sizeof pins[0]);
    spi_initMaster(&AVR32_SPI0, &opt);
    spi_selectionMode(&AVR32_SPI0, 0, 0, 0); /* fixed peripheral select */
    spi_setupChipReg(&AVR32_SPI0, &opt, sysclk_get_pba_hz());
    spi_enable(&AVR32_SPI0);
    spi_unselectChip(&AVR32_SPI0, 2); /* every request toggles NPCS2 itself (SPI_PROTOCOL 1.3) */
}
void board_module_off(void) {
    gpio_set_pin_low(PIN_MOD_RST);
    delay_ms(20);
    gpio_set_pin_low(PIN_MOD_PWR);
    for (unsigned i = 0; i < sizeof spi_pins / sizeof spi_pins[0]; i++)
        gpio_configure_pin(spi_pins[i], GPIO_DIR_INPUT);
}

/* External USB routing mux -> MCU device port. Uses the bootloader's DFU-mode pin state exactly
 * (PB11=H PB17=H, PB12/PA19/PA22 = L), which is what every working console session inherited via start-app. */
void board_usb_mux_device(void) {
    gpio_configure_pin(AVR32_PIN_PB11, GPIO_DIR_OUTPUT | GPIO_INIT_HIGH);
    gpio_configure_pin(AVR32_PIN_PB12, GPIO_DIR_OUTPUT | GPIO_INIT_LOW);
    gpio_configure_pin(AVR32_PIN_PA19, GPIO_DIR_OUTPUT | GPIO_INIT_LOW);
    gpio_configure_pin(AVR32_PIN_PA22, GPIO_DIR_OUTPUT | GPIO_INIT_LOW);
    gpio_configure_pin(AVR32_PIN_PB17, GPIO_DIR_OUTPUT | GPIO_INIT_HIGH);
}

/* Cold-boot stage log: one word per stage appended to the last flash page (0x8003FE00, erased by every DFU
 * flash). Word = 0xB0 | boot# | stage | RCAUSE. Read back over DFU (`zd1 dfu boot-log`) or `zd1 peek 0x8003FE00 64`. */
#define BLOG_PAGE 511u
#define BLOG      ((volatile uint32_t *)0x8003FE00u)
void blog(uint8_t stage) {
    static uint8_t boot_no;
    unsigned i, boots = 0;
    for (i = 0; i < 128 && BLOG[i] != 0xFFFFFFFFu; i++)
        if (((BLOG[i] >> 8) & 0xFF) == 1)
            boots++;
    if (i >= 128)
        return; /* page full: stop, never erase in the field */
    if (stage == 1)
        boot_no = (uint8_t)boots;
    flashc_clear_page_buffer();
    BLOG[i] = 0xB0000000u | ((uint32_t)boot_no << 16) | ((uint32_t)stage << 8) | (g_rcause & 0xFF);
    flashc_write_page(BLOG_PAGE);
}
