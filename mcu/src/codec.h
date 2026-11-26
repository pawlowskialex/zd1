/* AD193x codec (I2C 0x04, 1-byte register address): ADC for AUX analog, MCLK = 256 fs. */
#ifndef CODEC_H
#define CODEC_H
#include <compiler.h>

int codec_init(void);            /* stock 1.00.00's sequence (no PLL lock to wait for); 0 = OK, else I2C error */
int codec_aux_mode(bool analog); /* reg 14 bit 0: the ADC powered for AUX analog, down for every other input */
int codec_dac_mute(bool mute);   /* reg 4 bit 0 = DAC master mute (+ reg 14's ADC1 mutes); init leaves it set */

#endif
