/* Microsoft OS 2.0 descriptors: WinUSB for the host link's interface on Windows (msos.c) */
#ifndef MSOS_H
#define MSOS_H
#include <stdbool.h>
#include <stdint.h>

extern const uint8_t msos_bos[]; /* the BOS descriptor (udc_config.conf_bos: conf_usb.h, USB_DEVICE_BOS) */
bool msos_request(void);         /* the set, on its vendor request (USB_DEVICE_SPECIFIC_REQUEST) */

#endif
