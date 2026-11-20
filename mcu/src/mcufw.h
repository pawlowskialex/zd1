/* The MCU's own firmware from the module (docs/mcu/bootloader-hbl.md): the flash layout both models share and the
 * hand-over between the app and what takes a new one. On the ZD1L that is B&W's "Net HBL" bootloader
 * (0x80000000-0x80007FFF), on the ZD1 our preloader (preloader/, 0x80004000-0x80007DFF, behind B&W's DFU bootloader);
 * both take the app over SPI from the module, which plays BL2 (zd1-linkd). Shared by the app and the preloader. */
#ifndef MCUFW_H
#define MCUFW_H
#include <stdint.h>

#define MCUFW_APP         0x80008000u /* the app, on both models (a B&W .dmb's place) */
#define MCUFW_APP_END     0x8003FE00u /* page 511 is the boot log (zd1_board.c) */
#define MCUFW_RECORD      0x80007E00u /* the preloader's record of the app it installed: its .dmb header */
#define MCUFW_RECORD_PAGE 63u         /* (MCUFW_RECORD - 0x80000000) / 512 */
#define MCUFW_DMB_HEADER  128u
/* GPLP0 (kept over a watchdog reset; B&W's bootloaders never touch it): the app asks our preloader for a new app */
#define MCUFW_GPLP0_UPDATE 0x02000000u
/* GPLP1 bits 31..16: what the preloader answers our boot stub on reg 0xC4 (b0 the slot, b1 the good one), so the
 * module boots the system it runs now instead of the rescue */
#define MCUFW_GPLP1_BOOT_SHIFT 16
/* in the preloader's image: how the app knows the preloader is there (else B&W's HBL, the ZD1L) */
#define MCUFW_PRELOADER_MAGIC "ZD1-PRELOADER-01"
/* status words of BL2 (and of our OS playing it) the bootloader side waits for */
#define MCUFW_STATUS_FINISHED 0x2500u
#define MCUFW_HCR_BL2         0x2120u
#define MCUFW_DCR_HOST_FW     0x2423u

/* a .dmb header (big-endian): 00 01 00 80, +8 the app's length, +16 its CRC-16 */
static inline uint32_t mcufw_be32(const volatile uint8_t *p) {
    return (uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 | (uint32_t)p[2] << 8 | p[3];
}
static inline int mcufw_header_ok(const volatile uint8_t *h) {
    uint32_t len = mcufw_be32(h + 8);
    return h[0] == 0 && h[1] == 1 && h[2] == 0 && h[3] == 0x80 && len > 0 && len <= MCUFW_APP_END - MCUFW_APP;
}
#ifndef MCUFW_PRELOADER_BUILD
#include <stdbool.h>
bool mcufw_preloader_present(void); /* the ZD1 with our preloader at 0x80004000 */
bool mcufw_hbl_present(void);       /* the ZD1L: B&W's "Net HBL" at 0x80000000 */
/* A module can be taken over and the MCU is what keeps the amp safe, so an app from the module is taken only after a
 * person at the speaker allowed it (the host link's `Allow`, which our OS asks at a network update's start: ON/OFF held
 * 5 s): once, within MCUFW_ALLOW_MS. Our OS has an app staged (reg 0xD7): taken if allowed, else refused (logged). */
#define MCUFW_ALLOW_MS (10u * 60 * 1000)
void mcufw_allow(void);
void mcufw_request(void);
void mcufw_enter(void); /* into what takes the module's app (no return), else log */
#endif
#endif
