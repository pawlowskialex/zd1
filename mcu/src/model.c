#include <string.h>
#include "log.h"
#include "mcufw.h"
#include "model.h"

static model_t current = MODEL_UNKNOWN;
static uint8_t hbl_hwid;

static int hexval(char c) {
    return c >= '0' && c <= '9'   ? c - '0'
           : c >= 'A' && c <= 'F' ? c - 'A' + 10
           : c >= 'a' && c <= 'f' ? c - 'a' + 10
                                  : -1;
}

/* The HWID the HBL writes into the module's persistent parameters: its string "0xHH000000" (a NUL before and after it
 * in the ZD1L's, 0x800044CC), HH the model's byte. 0 if there is none, or more than one. */
static uint8_t find_hbl_hwid(void) {
    static const char tail[] = "000000";
    int found = 0;
    uint8_t id = 0;
    for (const char *p = (const char *)0x80000001u; p + 11 <= (const char *)MCUFW_APP; p++) {
        if (p[0] != '0' || p[1] != 'x' || p[-1] != 0 || p[10] != 0 || memcmp(p + 4, tail, 6))
            continue;
        int hi = hexval(p[2]), lo = hexval(p[3]);
        if (hi < 0 || lo < 0)
            continue;
        id = (uint8_t)(hi << 4 | lo);
        found++;
    }
    return found == 1 ? id : 0;
}

void model_detect(void) {
    if (!mcufw_hbl_present()) {
        current = MODEL_ZD1; /* B&W's DFU bootloader: only the Zeppelin Air has one */
    } else {
        hbl_hwid = find_hbl_hwid();
        current = hbl_hwid == 0xBE ? MODEL_ZD1L : hbl_hwid == 0xA7 ? MODEL_A7 : MODEL_UNKNOWN;
    }
    log_puts("[model] ");
    log_puts(model_name(current));
    if (mcufw_hbl_present()) {
        log_puts(" (B&W's bootloader, HWID 0x");
        log_hex8(hbl_hwid);
        log_putc(')');
    }
    log_putc('\n');
}

model_t model(void) { return current; }
uint8_t model_hbl_hwid(void) { return hbl_hwid; }

const char *model_name(model_t m) {
    switch (m) {
    case MODEL_ZD1:
        return "Zeppelin Air";
    case MODEL_ZD1L:
        return "Zeppelin Air with Lightning";
    case MODEL_A7:
        return "A7";
    default:
        return "unknown model";
    }
}
