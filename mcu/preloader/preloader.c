/* The ZD1's preloader (docs/mcu/bootloader-hbl.md): our code at 0x80004000, where B&W's DFU bootloader starts the
 * app. It starts the app at 0x80008000, or takes a new one from the module the way the ZD1L's "Net HBL" bootloader
 * does, so that one module side (zd1-linkd, playing BL2) and one app build serve both models:
 *   - the app asked for it (GPLP0 MCUFW_GPLP0_UPDATE, then a watchdog reset), or there is no good app (no record):
 *     power the module, wait for ready (answering our boot stub's slot question with what the app left in GPLP1),
 *     ask for BL2 (a configuration-channel read of NetIF/mode first, as the HBL does: our OS's sign that this is
 *     the bootloader), wait for BL2 (status 0x2300) and the DCR 0x2423, then XModem the .dmb in from 0x2C:
 *     its header first, the app into 0x80008000.., checked (CRC-16) before the record at 0x80007E00 says it is good
 *   - else: straight into the app, nothing touched (the RC oscillator, the pins as the reset left them).
 * The record is erased before the first page is written: a transfer cut short leaves no good app, and the next start
 * comes here again. B&W's DFU bootloader (ON/OFF at power-up) stays the way back from anything. */
#include <avr32/io.h>
#include <stdbool.h>
#include <string.h>
#include <sysclk.h>
#include <gpio.h>
#include <spi.h>
#include <flashc.h>
#include <wdt.h>
#include <pm.h>
#include <cycle_counter.h>
#include "zd1link.h"
#define MCUFW_PRELOADER_BUILD
#include "mcufw.h"

#define PIN_MOD_PWR AVR32_PIN_PA04 /* module power, high = on */
#define PIN_MOD_RST AVR32_PIN_PA17 /* module reset, low = reset */
#define PIN_MOD_REQ AVR32_PIN_PA20 /* module request line, high = data waiting */

/* found by the app (mcufw_preloader_present); read once in main, or --gc-sections drops it */
static const char magic[] = MCUFW_PRELOADER_MAGIC;

/* ---- time (the cycle counter, at the PLL's 48 MHz once sysclk_init ran) ---- */
static uint32_t ms_now, cy_last, cy_rem;
static uint32_t tick_ms(void) {
    uint32_t c = Get_sys_count(), per = sysclk_get_cpu_hz() / 1000u;
    cy_rem += c - cy_last;
    cy_last = c;
    ms_now += cy_rem / per;
    cy_rem %= per;
    return ms_now;
}
static bool after(uint32_t t) { return (int32_t)(tick_ms() - t) >= 0; }
static void wait_ms(uint32_t n) {
    uint32_t end = tick_ms() + n;
    while (!after(end))
        wdt_clear();
}

/* ---- its log: USART0 (PA00/PA01, 115200 8N1), the debug UART B&W's bootloaders and the stock app print on ---- */
static void log_init(void) {
    static const gpio_map_t pins = {{AVR32_USART0_RXD_0_0_PIN, AVR32_USART0_RXD_0_0_FUNCTION},
                                    {AVR32_USART0_TXD_0_0_PIN, AVR32_USART0_TXD_0_0_FUNCTION}};
    gpio_enable_module(pins, 2);
    volatile avr32_usart_t *u = &AVR32_USART0;
    u->cr = AVR32_USART_CR_RSTRX_MASK | AVR32_USART_CR_RSTTX_MASK;
    u->mr = AVR32_USART_MR_CHRL_8 << AVR32_USART_MR_CHRL_OFFSET | AVR32_USART_MR_PAR_NONE << AVR32_USART_MR_PAR_OFFSET;
    u->brgr = (sysclk_get_pba_hz() + 8 * 115200) / (16 * 115200);
    u->cr = AVR32_USART_CR_TXEN_MASK;
}
static void say(const char *s) {
    for (; *s; s++) {
        if (*s == '\n') {
            while (!(AVR32_USART0.csr & AVR32_USART_CSR_TXRDY_MASK))
                ;
            AVR32_USART0.thr = '\r';
        }
        while (!(AVR32_USART0.csr & AVR32_USART_CSR_TXRDY_MASK))
            ;
        AVR32_USART0.thr = (uint8_t)*s;
    }
}
static void say_hex(uint32_t v) {
    char b[11] = "0x";
    for (int i = 0; i < 8; i++)
        b[2 + i] = "0123456789abcdef"[v >> (28 - 4 * i) & 15];
    b[10] = 0;
    say(b);
}

/* ---- the module link: the master half of the register protocol (as dm870.c, without the app's state) ---- */
static void spi0_init(void) {
    static const gpio_map_t pins = {
        {AVR32_SPI0_NPCS_2_0_PIN, AVR32_SPI0_NPCS_2_0_FUNCTION}, /* PA09 */
        {AVR32_SPI0_MISO_0_0_PIN, AVR32_SPI0_MISO_0_0_FUNCTION}, /* PA11 */
        {AVR32_SPI0_MOSI_0_0_PIN, AVR32_SPI0_MOSI_0_0_FUNCTION}, /* PA12 */
        {AVR32_SPI0_SCK_0_0_PIN, AVR32_SPI0_SCK_0_0_FUNCTION},   /* PA13 */
    };
    const spi_options_t opt = {.reg = 2,
                               .baudrate = 3000000,
                               .bits = 8,
                               .spck_delay = 0,
                               .trans_delay = 0,
                               .stay_act = 1,
                               .spi_mode = 3,
                               .modfdis = 1};
    gpio_enable_module(pins, sizeof pins / sizeof pins[0]);
    spi_initMaster(&AVR32_SPI0, &opt);
    spi_selectionMode(&AVR32_SPI0, 0, 0, 0);
    spi_setupChipReg(&AVR32_SPI0, &opt, sysclk_get_pba_hz());
    spi_enable(&AVR32_SPI0);
    spi_unselectChip(&AVR32_SPI0, 2);
}
static uint8_t xfer(uint8_t b) {
    uint16_t d = 0xFF;
    if (spi_write(&AVR32_SPI0, b) == SPI_OK)
        spi_read(&AVR32_SPI0, &d);
    return (uint8_t)d;
}
static void cs(bool on) {
    if (on)
        spi_selectChip(&AVR32_SPI0, 2);
    else
        spi_unselectChip(&AVR32_SPI0, 2);
}
static void put(uint8_t b) {
    uint8_t o[2], n = zd1_stuff(b, o);
    for (uint8_t i = 0; i < n; i++)
        xfer(o[i]);
}
static int get(uint32_t end) {
    zd1_unstuff_t u = {0};
    while (!after(end)) {
        uint8_t b;
        if (zd1_unstuff(&u, xfer(ZD1_WIRE_IDLE), &b) == ZD1_RX_BYTE)
            return b;
    }
    return -1;
}
static bool skip_to(uint8_t want, uint32_t end) {
    int b;
    while ((b = get(end)) >= 0)
        if (b == want)
            return true;
    return false;
}
static void write_f(uint8_t reg, uint16_t v) {
    cs(true);
    xfer(ZD1_WIRE_WRITE_F);
    put(reg);
    put((uint8_t)(v >> 8));
    put((uint8_t)v);
    cs(false);
}
static void write_v(uint8_t reg, const uint8_t *d, uint16_t n) { /* short ones only: no pacing */
    cs(true);
    xfer(ZD1_WIRE_WRITE_V);
    put(reg);
    put((uint8_t)n);
    put((uint8_t)(n >> 8));
    for (uint16_t i = 0; i < n; i++)
        put(d[i]);
    cs(false);
}
static void request(uint8_t type, uint8_t reg) {
    cs(true);
    xfer(type);
    put(reg);
    xfer(0);
    xfer(0);
    cs(false);
    wait_ms(5);
    cs(true);
}
static bool read_f(uint8_t reg, uint16_t *v) {
    bool ok = false;
    request(ZD1_WIRE_READ_F, reg);
    uint32_t end = tick_ms() + 300;
    if (skip_to(ZD1_WIRE_READ_F, end) && skip_to(reg, end)) {
        int b0 = get(end), b1 = b0 < 0 ? -1 : get(end);
        if (b1 >= 0) {
            *v = (uint16_t)(b0 << 8 | b1);
            ok = true;
        }
    }
    cs(false);
    return ok;
}
static bool read_v(uint8_t reg, uint8_t *buf, uint16_t max, uint16_t *len) {
    bool ok = false;
    int lo, hi;
    request(ZD1_WIRE_READ_V, reg);
    uint32_t end = tick_ms() + 300;
    if (skip_to(ZD1_WIRE_READ_V, end) && skip_to(reg, end) && (lo = get(end)) >= 0 && (hi = get(end)) >= 0) {
        uint16_t n = (uint16_t)(lo | hi << 8);
        ok = true;
        for (uint16_t i = 0; i < n; i++) {
            int b = get(end);
            if (b < 0) {
                ok = false;
                break;
            }
            if (i < max)
                buf[i] = (uint8_t)b;
        }
        *len = n;
    }
    cs(false);
    return ok;
}

/* ---- what the module said ---- */
static uint16_t status;       /* the last 0x11 */
static bool bl2, dcr, hcr_ok; /* BL2 (or our OS playing it) runs; the host firmware is ready; HCR status OK */
static uint8_t vbuf[ZD1_WIRE_V_MAX + 1];

static void took(uint8_t reg, uint16_t v) {
    if (reg == ZD1_REG_STATUS) {
        status = v;
        if (v == ZD1_STATUS_BL2)
            bl2 = true;
    } else if (reg == ZD1_REG_HCR_STATUS) {
        hcr_ok = v == 0x2020 || v == 0x2120;
    } else if (reg == ZD1_REG_DCR) {
        if (v == MCUFW_DCR_HOST_FW)
            dcr = true;
    }
}
/* the queued notifications, for `ms`: each one read the way its register is (F or V) */
static void drain(uint32_t ms) {
    uint32_t end = tick_ms() + ms;
    while (!after(end)) {
        wdt_clear();
        uint16_t p = 0, v = 0, n = 0;
        if (!gpio_get_pin_value(PIN_MOD_REQ) || !read_f(ZD1_REG_PENDING, &p) || !(p >> 8)) {
            wait_ms(5);
            continue;
        }
        uint8_t reg = (uint8_t)(p >> 8);
        if (zd1_reg_kind(reg) == 'F') {
            if (read_f(reg, &v))
                took(reg, v);
        } else
            read_v(reg, vbuf, sizeof vbuf, &n);
    }
}

/* our boot stub asks which system to start (status 0x2000, "zd1-boot" on 0x25, 0xC4 still 0): what the app left */
static void answer_stub(void) {
    uint16_t n = 0, v = 0xFFFF;
    if (!read_v(ZD1_REG_VERSION_IMAGE, vbuf, sizeof vbuf, &n) || n < sizeof ZD1_BOOT_IDENT - 1 ||
        memcmp(vbuf, ZD1_BOOT_IDENT, sizeof ZD1_BOOT_IDENT - 1) || !read_f(ZD1_REG_BOOT, &v) || v)
        return;
    uint16_t a = (uint16_t)(pm_read_gplp(&AVR32_PM, 1) >> MCUFW_GPLP1_BOOT_SHIFT);
    if (!(a >> 8))
        a = ZD1_BOOT_RESCUE << 8;
    write_f(ZD1_REG_BOOT, a);
}

static void module_off(void) {
    gpio_set_pin_low(PIN_MOD_RST);
    wait_ms(20);
    gpio_set_pin_low(PIN_MOD_PWR);
}
static void module_on(void) {
    module_off();
    wait_ms(500);
    gpio_set_pin_high(PIN_MOD_PWR);
    wait_ms(100);
    gpio_set_pin_high(PIN_MOD_RST);
}

/* ---- the app: XModem blocks into flash ---- */
static uint8_t header[MCUFW_DMB_HEADER], page[512];
static uint32_t got; /* app bytes taken */
static bool have_header;

static bool take(const uint8_t *d) {
    if (!have_header) {
        memcpy(header, d, MCUFW_DMB_HEADER);
        have_header = true;
        return mcufw_header_ok(header);
    }
    if (MCUFW_APP + got + 128 > MCUFW_APP_END)
        return false;
    memcpy(page + got % 512, d, 128);
    got += 128;
    if (!(got % 512))
        flashc_memcpy((void *)(MCUFW_APP + got - 512), page, 512, true);
    return true;
}
static void ctl(uint8_t c) { write_v(ZD1_REG_XM_HOST, &c, 1); }

/* one XModem receive from 0x2C; true when the whole image is in flash and checks */
static bool receive(void) {
    uint8_t seq = 1, fails = 0;
    uint16_t n = 0;
    got = 0;
    have_header = false;
    ctl(ZD1_XMODEM_START);
    for (;;) {
        bool fresh = false;
        uint32_t end = tick_ms() + 1000; /* the module queues the next block once it has our ACK */
        while (!after(end)) {
            wait_ms(5);
            n = 0;
            if (read_v(ZD1_REG_XM_MODULE, vbuf, sizeof vbuf, &n) &&
                (n == ZD1_XMODEM_BLOCK + 5 || (n == 1 && vbuf[0] == ZD1_XMODEM_EOT))) {
                fresh = true;
                break;
            }
        }
        if (fresh && n == 1) { /* EOT */
            ctl(ZD1_XMODEM_ACK);
            break;
        }
        bool good = fresh && vbuf[0] == ZD1_XMODEM_SOH && zd1_xmodem_check(vbuf, n) >= 0;
        if (good && vbuf[1] == (uint8_t)(seq - 1)) { /* our ACK was lost: the same block again */
            ctl(ZD1_XMODEM_ACK);
            continue;
        }
        if (!good || vbuf[1] != seq) {
            if (++fails > 10) {
                ctl(ZD1_XMODEM_CAN);
                ctl(ZD1_XMODEM_CAN);
                return false;
            }
            ctl(seq == 1 && !have_header ? ZD1_XMODEM_START : ZD1_XMODEM_NAK);
            continue;
        }
        fails = 0;
        if (!take(vbuf + 3)) {
            ctl(ZD1_XMODEM_CAN);
            ctl(ZD1_XMODEM_CAN);
            return false;
        }
        seq++;
        ctl(ZD1_XMODEM_ACK);
    }
    if (!have_header)
        return false;
    if (got % 512) {
        memset(page + got % 512, 0xFF, 512 - got % 512);
        flashc_memcpy((void *)(MCUFW_APP + got - got % 512), page, 512, true);
    }
    uint32_t len = mcufw_be32(header + 8);
    return len <= got &&
           zd1_xmodem_crc16(0, (const volatile uint8_t *)MCUFW_APP, len) == (uint16_t)(header[16] << 8 | header[17]);
}

/* one session with the module: ready, BL2, the transfer */
static bool session(void) {
    static const uint8_t ask[] = "R\x16"
                                 "Application/NetIF/mode";
    uint32_t end = tick_ms() + 180000; /* a cold boot of our OS, the rescue's at worst */
    bl2 = dcr = hcr_ok = false;
    for (;;) {
        uint16_t st = 0;
        if (after(end)) {
            say("[pre] the module never got ready\n");
            return false;
        }
        if (!gpio_get_pin_value(PIN_MOD_REQ))
            write_f(ZD1_REG_BOOT, ZD1_BOOT_KNOCK); /* our boot stub stays silent until it hears it */
        if (read_f(ZD1_REG_STATUS, &st)) {
            took(ZD1_REG_STATUS, st);
            if (st == ZD1_STATUS_READY || st == ZD1_STATUS_BL2)
                break;
            if (st == ZD1_STATUS_BOOTING)
                answer_stub();
        }
        wait_ms(250);
    }
    say("[pre] the module is ready: asking for BL2\n");
    drain(1000);
    write_v(ZD1_REG_CONFIG, ask, sizeof ask - 1);
    drain(500);
    write_f(ZD1_REG_HCR, MCUFW_HCR_BL2);
    end = tick_ms() + 60000;
    while (!(bl2 && dcr)) {
        if (after(end)) {
            say(bl2 ? "[pre] BL2 has no app for us\n" : "[pre] no BL2\n");
            return false;
        }
        drain(100);
    }
    say("[pre] BL2 has an app: taking it\n");
    flashc_erase_page(MCUFW_RECORD_PAGE, true); /* from here on the old app is gone until this one is in */
    if (!receive()) {
        say("[pre] the transfer failed\n");
        return false;
    }
    say("[pre] app in flash and checked, ");
    say_hex(mcufw_be32(header + 8));
    say(" bytes\n");
    hcr_ok = false;
    end = tick_ms() + 20000;
    while (!after(end) && status != MCUFW_STATUS_FINISHED)
        drain(100);
    flashc_memcpy((void *)MCUFW_RECORD, header, MCUFW_DMB_HEADER, true);
    return true;
}

static void start_app(void) {
    __asm__ volatile("mov pc, %0" ::"r"(MCUFW_APP));
    for (;;) {
    }
}

static void watchdog_reset(void) {
    wdt_opt_t opt = {.us_timeout_period = 1000000}; /* long enough for B&W's bootloader to see it through */
    wdt_enable(&opt);
    for (;;) {
    }
}

int main(void) {
    (void)*(const volatile char *)magic;
    uint32_t g = pm_read_gplp(&AVR32_PM, 0);
    bool asked = g & MCUFW_GPLP0_UPDATE;
    if (!asked && mcufw_header_ok((const volatile uint8_t *)MCUFW_RECORD))
        start_app();
    pm_write_gplp(&AVR32_PM, 0, g & ~MCUFW_GPLP0_UPDATE);

    wdt_disable(); /* one still running from the app's reset */
    sysclk_init();
    cy_last = Get_sys_count();
    log_init();
    say(asked ? "\n[pre] zd1 preloader: the app asks for a new one from the module\n"
              : "\n[pre] zd1 preloader: no good app: waiting for one from the module\n");
    wdt_opt_t opt = {.us_timeout_period = 4000000};
    wdt_enable(&opt);
    gpio_configure_pin(PIN_MOD_PWR, GPIO_DIR_OUTPUT | GPIO_INIT_LOW);
    gpio_configure_pin(PIN_MOD_RST, GPIO_DIR_OUTPUT | GPIO_INIT_LOW);
    gpio_configure_pin(PIN_MOD_REQ, GPIO_DIR_INPUT);
    spi0_init();

    for (unsigned tries = 1;; tries++) {
        module_on();
        bool ok = session();
        module_off();
        if (ok)
            break;
        /* the app still there (nothing was written): back to it after a few tries; else keep asking */
        if (tries >= 3 && mcufw_header_ok((const volatile uint8_t *)MCUFW_RECORD)) {
            say("[pre] giving up: back to the app we have\n");
            break;
        }
        wait_ms(2000);
    }
    say("[pre] restarting\n");
    watchdog_reset();
    return 0;
}
