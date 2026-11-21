/* ADAU1445 data from the stock apps: each model's download (program RAM, config, param RAM, start) and the runtime
 * tables, which are the same bytes in every model's app. Generated into dsp_data.c by mcu/tools/gen_dsp_data.py. */
#ifndef DSP_DATA_H
#define DSP_DATA_H
#include <stdint.h>

typedef struct {
    uint16_t subaddr;
    uint16_t len;
    const uint8_t *data;
} dsp_rec_t;
/* the Zeppelin Air's (ZD1 2.04.05; the ZD1L's 2.04.35 sends the same bytes) */
extern const dsp_rec_t dsp_download_zd1[];
extern const unsigned dsp_download_zd1_steps;
/* the A7's (2.04.77): the same input stage, another output stage and voicing (docs/models/a7.md, "DSP") */
extern const dsp_rec_t dsp_download_a7[];
extern const unsigned dsp_download_a7_steps;
extern const uint32_t dsp_volume_coef[72];
#define DSP_BASS_PRESETS 5
extern const uint32_t dsp_bass_coef[DSP_BASS_PRESETS][5];

#endif
