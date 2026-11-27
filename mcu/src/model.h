/* Which speaker this is: the Zeppelin Air (ZD1), the Zeppelin Air with Lightning (ZD1L) or the A7. One app serves
 * them; what differs for it is the way to a new MCU firmware (mcufw.c): B&W's DFU bootloader on USB on a ZD1, the
 * module on the others. Told by the MCU's bootloader, which the factory put there and nothing of ours writes: B&W's
 * DFU bootloader (and our preloader behind it) on a ZD1; on the ZD1L and the A7 B&W's "Net HBL", which writes the
 * module's HWID into its persistent parameters at every start and so carries it ("0xBE000000" in the ZD1L's:
 * docs/mcu/bootloader-hbl.md). The host link reports it (`InfoReply.model`): the page plans updates by it. */
#ifndef MODEL_H
#define MODEL_H
#include <stdint.h>
#include "zd1host_gen.h"

/* the host link's `Model` (protocol/host) */
typedef enum {
    MODEL_UNKNOWN = ZD1H_MODEL_UNKNOWN, /* B&W's bootloader with a HWID we don't know */
    MODEL_ZD1 = ZD1H_MODEL_ZD1,
    MODEL_ZD1L = ZD1H_MODEL_ZD1L,
    MODEL_A7 = ZD1H_MODEL_A7,
} model_t;

void model_detect(void); /* at the start; logs what it found */
model_t model(void);
const char *model_name(model_t m);
uint8_t model_hbl_hwid(void); /* the HWID in B&W's bootloader (0: none, or no HBL) */

#endif
