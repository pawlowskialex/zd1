/* ZD1 (B&W Zeppelin Air) main board: pin map, safe states, clock bring-up, cold-boot stage log.
 * Pin map: docs/hardware/zd1-main-board.md, "MCU pin map" (confidence tags there). */
#ifndef ZD1_BOARD_H
#define ZD1_BOARD_H
#include <compiler.h>
#include <gpio.h>

/* --- network module (DM870) --- */
#define PIN_MOD_PWR AVR32_PIN_PA04 /* module power, high = on */
#define PIN_MOD_RST AVR32_PIN_PA17 /* module reset, low = reset */
#define PIN_MOD_REQ AVR32_PIN_PA20 /* module request line, high = data waiting (input) */
/* --- audio --- */
#define PIN_DSP_PWR    AVR32_PIN_PB03 /* ADAU1445 power enable */
#define PIN_DSP_RST    AVR32_PIN_PB02 /* ADAU1445 nRESET */
#define PIN_AMP_EN     AVR32_PIN_PB09 /* power-stage supply enable */
#define PIN_AMP_UNMUTE AVR32_PIN_PB16 /* amp unmute, high = playing */
#define PIN_CODEC_MUTE AVR32_PIN_PB27 /* held during codec init */
#define PIN_CS2000_SCL AVR32_PIN_PB13 /* bit-banged I2C */
#define PIN_CS2000_SDA AVR32_PIN_PB14
/* --- inputs --- */
#define PIN_IR         AVR32_PIN_PA03 /* RC6 receiver */
#define PIN_KEY_REAR   AVR32_PIN_PA07 /* rear button, active low */
#define PIN_KEY_ONOFF  AVR32_PIN_PA08 /* ON/OFF key, active low; wake; bootloader ISP pin */
#define PIN_USB_VBUS   AVR32_PIN_PA24 /* rear USB VBUS = PC plugged in */
#define PIN_AUX_DETECT AVR32_PIN_PB00 /* AUX jack inserted */
#define PIN_FAULT      AVR32_PIN_PB04 /* fault input, active low */
#define PIN_KEY_VOLDN  AVR32_PIN_PB05
#define PIN_KEY_VOLUP  AVR32_PIN_PB06
#define PIN_AMP_FAULT  AVR32_PIN_PB18 /* amp fault/clip */
/* --- LED (PWM1/2/3) --- */
#define PIN_LED_R AVR32_PIN_PB20 /* red as a GPIO until the PWM takes it (G PB21, B PB22) */

extern uint32_t g_rcause; /* PM RCAUSE captured at boot */

/* The start, the app's and the preloader's alike: these three, in this order, first thing at reset */
void board_bl_pins(void);                      /* bootloader's DFU-mode pin state, on the reset RC clock */
bool board_clock_init(void (*stage)(uint8_t)); /* OSC0 -> run on OSC0 -> PLL0 48 MHz, bounded waits; each
                                                      stage reached (2-5) or failed (| 0x80) to `stage` if given */
void board_safe_gpio(void);                    /* amp off + muted, DSP/module held, red LED off */

/* The module's power, stock's (docs/protocol/sequences.md), the app's (dm870.c) and the preloader's alike: PA04 high,
 * MOD_SUPPLY_MS, board_module_spi(), MOD_RESET_MS before the first read; off with board_module_off(), and off
 * MOD_OFF_MS before it powers again. The SPI pins are driven only while it is powered and out of reset. */
#define MOD_SUPPLY_MS 1200
#define MOD_RESET_MS  1200 /* 200 ms + stock's T3 1000 ms */
#define MOD_OFF_MS    500
void board_module_spi(void); /* the reset released (PA17), SPI0 on its pins (NPCS2 PA09, mode 3, ~3 MHz) */
void board_module_off(void); /* stock's detach (FUN_8000dcdc): reset, 20 ms, power off; the SPI pins back to inputs */
void board_usb_mux_device(void); /* external USB routing -> MCU device port */
void blog(uint8_t stage);        /* cold-boot stage log (flash page 511) */

#endif
