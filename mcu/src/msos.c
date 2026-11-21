/* Windows binds WinUSB to the host link's interface by itself (Microsoft OS 2.0 descriptors), so the page and zd1
 * reach the speaker there without a driver installed by hand. bcdUSB 2.10 (conf_usb.h: USB_DEVICE_BOS) makes Windows
 * read the BOS descriptor: its platform capability names our vendor request, which returns the descriptor set,
 * interface 3's compatible ID "WINUSB" and the interface GUID WinUSB registers it under. Audio and HID keep their
 * class drivers. Windows keeps what it read per VID:PID:bcdDevice (usbflags): a change here wants bcdDevice changed
 * too (conf_usb.h, USB_DEVICE_MINOR_VERSION). */
#include "conf_usb.h"
#include <udd.h>
#include "msos.h"

#define MSOS_VENDOR_CODE 0x20 /* its bRequest (the host link's are 0x01-0x03: protocol/host, usb) */
#define MSOS_INDEX       7    /* wIndex: MS_OS_20_DESCRIPTOR_INDEX */
#define MSOS_IFACE       3    /* the host link's interface (protocol/host, usb::IFACE) */
#define W(v)             (uint8_t)(v), (uint8_t)((v) >> 8) /* little-endian */
#define D(v)             W(v), W((v) >> 16)
#define U(c)             c, 0 /* a UTF-16LE character */
#define WINDOWS_8_1      0x06030000u

/* the set (lengths: the property's name 42 bytes, its data 80; the property 132, the function's subset 160, the
 * configuration's 168, the set 178) */
static const uint8_t set[] = {
    W(10),  W(0x0000), D(WINDOWS_8_1),
    W(178), /* MS_OS_20_SET_HEADER_DESCRIPTOR */
    W(8),   W(0x0001), 0,
    0,      W(168), /* the configuration's subset: the first (its index) */
    W(8),   W(0x0002), MSOS_IFACE,
    0,      W(160), /* the function's subset: the host link's interface */
    W(20),  W(0x0003), 'W',
    'I',    'N',       'U',
    'S',    'B',       0,
    0,      0,         0,
    0,      0,         0,
    0,      0,         0,    /* compatible ID, no sub-ID */
    W(132), W(0x0004), W(7), /* a registry property, REG_MULTI_SZ */
    W(42),  U('D'),    U('e'),
    U('v'), U('i'),    U('c'),
    U('e'), U('I'),    U('n'),
    U('t'), U('e'),    U('r'),
    U('f'), U('a'),    U('c'),
    U('e'), U('G'),    U('U'),
    U('I'), U('D'),    U('s'),
    U(0),   W(80),     U('{'),
    U('A'), U('3'),    U('6'),
    U('0'), U('C'),    U('B'),
    U('6'), U('5'),    U('-'),
    U('1'), U('2'),    U('F'),
    U('3'), U('-'),    U('4'),
    U('1'), U('A'),    U('0'),
    U('-'), U('A'),    U('C'),
    U('C'), U('E'),    U('-'),
    U('E'), U('3'),    U('A'),
    U('D'), U('9'),    U('0'),
    U('2'), U('8'),    U('5'),
    U('4'), U('0'),    U('3'),
    U('}'), U(0),      U(0),
};

/* the BOS: one platform capability, MS OS 2.0's ({D8DD60DF-4589-4CC7-9CD2-659D9E648A9F}) */
COMPILER_WORD_ALIGNED const uint8_t msos_bos[] = {
    5,
    0x0F,
    W(33),
    1,
    28,
    0x10,
    0x05,
    0,
    0xDF,
    0x60,
    0xDD,
    0xD8,
    0x89,
    0x45,
    0xC7,
    0x4C,
    0x9C,
    0xD2,
    0x65,
    0x9D,
    0x9E,
    0x64,
    0x8A,
    0x9F,
    D(WINDOWS_8_1),
    W(sizeof set),
    MSOS_VENDOR_CODE,
    0,
};

bool msos_request(void) {
    if (udd_g_ctrlreq.req.bmRequestType != (USB_REQ_DIR_IN | USB_REQ_TYPE_VENDOR | USB_REQ_RECIP_DEVICE) ||
        udd_g_ctrlreq.req.bRequest != MSOS_VENDOR_CODE || udd_g_ctrlreq.req.wIndex != MSOS_INDEX)
        return false;
    uint16_t n = udd_g_ctrlreq.req.wLength < sizeof set ? udd_g_ctrlreq.req.wLength : sizeof set;
    udd_set_setup_payload((uint8_t *)set, n);
    return true;
}
