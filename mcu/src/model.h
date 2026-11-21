/* Which speaker this is (docs/models/README.md): the Zeppelin Air (ZD1), the Zeppelin Air with Lightning (ZD1L) or the
 * A7. One app serves them all; what differs for it is the DSP's setup (the A7's crossover and voicing: dsp.c) and the
 * way to a new MCU firmware (mcufw.c). Told by the MCU's bootloader, which the factory put there and nothing of ours
 * writes: B&W's DFU bootloader (and our preloader behind it) on a ZD1; on the ZD1L and the A7 B&W's "Net HBL", which
 * writes the module's HWID into its persistent parameters at every start and so carries it ("0xBE000000" in the
 * ZD1L's: docs/mcu/bootloader-hbl.md). */
#ifndef MODEL_H
#define MODEL_H
#include <stdint.h>
#include "zd1host_gen.h"

/* the host link's `Model` (protocol/host) */
typedef enum {
    MODEL_UNKNOWN = ZD1H_MODEL_UNKNOWN, /* an HBL without a HWID we know: the DSP is not set up, the amp stays off */
    MODEL_ZD1 = ZD1H_MODEL_ZD1,
    MODEL_ZD1L = ZD1H_MODEL_ZD1L,
    MODEL_A7 = ZD1H_MODEL_A7,
} model_t;

void model_detect(void); /* at the start, before the DSP's setup; logs what it found */
model_t model(void);
const char *model_name(model_t m);
uint8_t model_hbl_hwid(void); /* the HWID in B&W's bootloader (0: none, or no HBL) */
/* the USB product string, as the model's stock app names it ("Zeppelin Air", "A7"): what the computer calls it */
const char *model_usb_name(void);
uint8_t model_usb_name_len(void);

#endif
