#include <string.h>
#include <pm.h>
#include "log.h"
#include "tick.h"
#include "modboot.h"
#include "selfdfu.h"
#include "mcufw.h"

/* `what` somewhere in flash [from, to) */
static bool in_flash(uint32_t from, uint32_t to, const char *what) {
    size_t n = strlen(what);
    for (const char *p = (const char *)from; p + n <= (const char *)to; p++)
        if (*p == what[0] && !memcmp(p, what, n))
            return true;
    return false;
}

bool mcufw_preloader_present(void) { return in_flash(0x80004000u, MCUFW_RECORD, MCUFW_PRELOADER_MAGIC); }
bool mcufw_hbl_present(void) { return in_flash(0x80000000u, 0x80008000u, "Net HBL"); }

static bool allowed; /* a person allowed the next app from the module (mcufw_allow), until allowed_until */
static uint32_t allowed_until;

void mcufw_allow(void) {
    allowed = true;
    allowed_until = tick_ms() + MCUFW_ALLOW_MS;
    log_puts("[mcufw] allowed: an app from the module, once, within 10 minutes\n");
}

void mcufw_request(void) {
    if (!mcufw_preloader_present() && !mcufw_hbl_present()) {
        log_puts(
            "[mcufw] our OS has an app for us, but no bootloader here takes one from the module (a ZD1 without the "
            "preloader): ignored\n");
        return;
    }
    if (!allowed || tick_after(tick_ms(), allowed_until)) {
        allowed = false;
        log_puts("[mcufw] our OS has an app for us, but nobody allowed it at the speaker (Allow): not taken\n");
        return;
    }
    allowed = false;
    mcufw_enter();
}

/* into what takes the app our OS has staged (reg 0xD7) from the module. The module goes off first
 * (selfdfu: it boots again under the bootloader, which asks it for BL2). */
void mcufw_enter(void) {
    if (mcufw_preloader_present()) {
        uint32_t g1 = pm_read_gplp(&AVR32_PM, 1) & 0xFFFFu, good = modboot_good();
        pm_write_gplp(&AVR32_PM, 1, g1 | (good << 8 | good) << MCUFW_GPLP1_BOOT_SHIFT);
        pm_write_gplp(&AVR32_PM, 0, pm_read_gplp(&AVR32_PM, 0) | MCUFW_GPLP0_UPDATE);
        log_puts("[mcufw] into the preloader for the module's app\n");
        selfdfu_reboot();
    }
    if (mcufw_hbl_present()) {
        log_puts("[mcufw] into B&W's bootloader for the module's app\n");
        selfdfu_enter();
    }
    log_puts("[mcufw] no bootloader here takes an app from the module (a ZD1 without the preloader): ignored\n");
}
