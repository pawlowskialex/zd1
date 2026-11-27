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

static bool allowed;       /* a person allowed the next app from the module (mcufw_allow), until allowed_until */
static bool allowed_ready; /* ... and the module has been ready since: allowed_until is that ready's */
static uint32_t allowed_until;

void mcufw_report(void) {
    static const char *const end[] = {[MCUFW_PRE_TOOK] = "took the app",
                                      [MCUFW_PRE_NOT_READY] = "the module never got ready",
                                      [MCUFW_PRE_NO_BL2] = "no BL2",
                                      [MCUFW_PRE_NO_APP] = "BL2 had no app for it",
                                      [MCUFW_PRE_TRANSFER] = "the transfer failed"};
    uint32_t g = pm_read_gplp(&AVR32_PM, 0);
    if (!(g & MCUFW_GPLP0_PRE))
        return;
    pm_write_gplp(&AVR32_PM, 0, g & ~MCUFW_GPLP0_PRE_MASK);
    unsigned tries = g >> 12 & 0xFF, shown = tries < 3 ? tries : 3;
    log_puts("[pre] the preloader's last run: ");
    log_dec(tries);
    log_puts(tries == 1 ? " try" : " tries");
    for (unsigned k = shown; k-- > 0;) { /* the oldest of the last three first */
        unsigned e = g >> (4 * k) & 0xF;
        log_puts(k == shown - 1 ? ": " : "; ");
        log_puts(e && e <= MCUFW_PRE_TRANSFER ? end[e] : "?");
    }
    uint32_t g1 = pm_read_gplp(&AVR32_PM, 1);
    if (g1 & MCUFW_GPLP1_SEEN) {
        pm_write_gplp(&AVR32_PM, 1, g1 & ~MCUFW_GPLP1_SEEN_MASK);
        log_puts("; the last try that failed: the module's status 0x");
        log_hex8((uint8_t)(g1 >> MCUFW_GPLP1_SEEN_SHIFT));
        log_puts(g1 & MCUFW_GPLP1_SEEN_KNOCKED ? "xx, knocked" : "xx, never knocked (its request line high)");
        log_puts(g1 & MCUFW_GPLP1_SEEN_ANSWER ? ", the stub's slot question answered" : ", no slot question answered");
    }
    log_putc('\n');
}

void mcufw_allow(void) {
    allowed = true;
    allowed_ready = false;
    allowed_until = tick_ms() + MCUFW_ALLOW_MS;
    log_puts("[mcufw] allowed: an app from the module, once, until 60 s after the module's next ready\n");
}

void mcufw_module_ready(void) {
    if (!allowed || allowed_ready)
        return;
    allowed_ready = true;
    uint32_t end = tick_ms() + MCUFW_ALLOW_AFTER_READY_MS;
    if (tick_after(allowed_until, end))
        allowed_until = end;
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

/* into B&W's bootloader with nothing staged for it (the host link's Bl2Updater, confirmed): it powers the module and
 * asks it for BL2, our OS (which asked for no app) reboots into BL2, and BL2's web updater takes a .fw for both chips
 * (docs/mcu/bootloader-hbl.md) */
void mcufw_bl2_updater(void) {
    if (!mcufw_hbl_present()) {
        log_puts("[mcufw] no B&W bootloader here for BL2's web updater: ignored\n");
        return;
    }
    log_puts("[mcufw] into B&W's bootloader: the module into BL2's web updater\n");
    selfdfu_enter();
}
