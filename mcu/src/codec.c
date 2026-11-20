#include <gpio.h>
#include "zd1_board.h"
#include "i2c.h"
#include "codec.h"

static int wr(uint8_t reg, uint8_t v) { return i2c_write(I2C_CODEC, reg, 1, &v, 1); }

/* The set-up of stock 1.00.00 (its FUN_80016a34), in its order: reg2 = 0x04 (the DAC's rate field at 2), reg5 = 0xC0
 * (DAC4 L/R muted), reg1 = 0x03 (DAC and ADC clocked from MCLK itself, not from the codec's PLL), reg0 = 0xB8,
 * reg14 = 0xBF, reg4 = 0x01 (master mute). Nothing runs from the PLL, so 1.00.00 neither kicks it nor waits for its
 * lock, and there is no lock time to report. 2.04.05's set-up (FUN_80020db0: the PLL kicked and waited for, reg2 =
 * 0x02, reg14 = 0x7E, reg0 = 0x98) is in docs/mcu/stock-firmware.md, "Codec".
 * 1.00.00 keeps reg 14 (ADC control 0) in RAM and writes the register from it: 0xB3 after the set-up (the ADC powered
 * down, bit 0; ADC2 muted; the rate field at 2). Bits 3:2 (ADC1 L/R mute) are never kept: the mute ORs them in. */
static uint8_t adc0 = 0xB3;
/* PB27 = codec mute, active HIGH: stock (emulator trace) drives it LOW from boot, HIGH around the codec set-up,
 * LOW again after. Left floating it reads high: muted, no sound on any source [HW 2026-10-18]. */
static int init_regs(void);
int codec_init(void) {
    gpio_configure_pin(PIN_CODEC_MUTE, GPIO_DIR_OUTPUT | GPIO_INIT_HIGH);
    int rc = init_regs();
    gpio_set_pin_low(PIN_CODEC_MUTE);
    return rc;
}
static int init_regs(void) {
    int rc;
    adc0 = 0xB3;
    if ((rc = wr(0x02, 0x04)) || (rc = wr(0x05, 0xC0)) || (rc = wr(0x01, 0x03)) || (rc = wr(0x00, 0xB8)) ||
        (rc = wr(0x0E, 0xBF)))
        return rc;
    return wr(0x04, 0x01);
}

/* DAC master mute = reg 4 (DAC Control 2) bit 0. The init sequence leaves it SET (stock does the same); stock clears
 * it only once the amp is unmuted and the source is ready and sets it again on every source switch / standby. Left
 * set, no source plays (the mic probe: reg4 0x01 silent, 0x00 plays [HW 2026-10-18]).
 * 1.00.00 writes reg 14 after it each time: ADC1 unmuted with the DAC (FUN_800169ec: reg4 = 0, reg14 = kept & 0xF3),
 * muted with it (FUN_80016a10: reg4 = 1, reg14 = kept | 0x0C). Not as 1.00.00 on AUX analog (the ADC powered): its
 * mute would silence the input our meter listens to with the amp asleep, and no sound could wake it, so there ADC1
 * stays unmuted. */
int codec_dac_mute(bool mute) {
    int rc = wr(0x04, mute ? 0x01 : 0x00);
    int rc14 = wr(0x0E, mute && (adc0 & 0x01) ? adc0 | 0x0C : adc0 & 0xF3);
    return rc ? rc : rc14;
}

/* The input's choice, 1.00.00's way: AUX analog powers the ADC (FUN_80016998: bit 0 of the kept reg 14 cleared,
 * reg14 = 0xB2), every other input powers it down again (FUN_8001697c: bit 0 set, reg14 = 0xB3). */
int codec_aux_mode(bool analog) {
    adc0 = analog ? adc0 & 0xFE : adc0 | 0x01;
    return wr(0x0E, adc0);
}
