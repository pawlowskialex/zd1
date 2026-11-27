/* Debounced board inputs and key events. Polarity: AUX detect active low (an empty jack reads 1 [HW 2026-10-17]);
 * USB VBUS active high; keys assumed active low like the ON/OFF key [L for PB05/PB06]. */
#ifndef INPUTS_H
#define INPUTS_H
#include <stdint.h>
#include <stdbool.h>

/* ON/OFF gives up to two events per press, like stock (re/mcu: key table 0x01a4, hold timer 0x22 = 2000 ms):
 * KEY_ONOFF on press, then KEY_ONOFF_SHORT on a release before 2 s or KEY_ONOFF_HOLD once it has been held 2 s. */
#define ONOFF_HOLD_MS 2000
typedef enum {
    KEY_NONE = 0,
    KEY_ONOFF,
    KEY_VOLUP,
    KEY_VOLDN,
    KEY_REAR,
    KEY_PLAYPAUSE,
    KEY_NEXT,
    KEY_PREV,
    KEY_ONOFF_SHORT,
    KEY_ONOFF_HOLD,
    KEY_STANDBY
} key_event_t; /* IR standby: toggles run/standby */

typedef struct {
    bool aux_inserted, usb_vbus; /* debounced levels */
} inputs_t;

void inputs_init(void);
void inputs_poll(uint32_t now); /* sample every 5 ms; 4 equal samples = stable */
const inputs_t *inputs(void);
key_event_t inputs_key(void);          /* next key event (press, or auto-repeat for volume), KEY_NONE if none */
void inputs_inject_key(key_event_t k); /* from IR / the host link (Key) */
/* The confirmation (a person at the speaker approving what the network or the module asked: a pairing, an MCU update):
 * ON/OFF held CONFIRM_HOLD_MS. Its 2 s hold still means standby (not held back while something waits: someone holding
 * ON/OFF to switch off lets go when the speaker does, and holding on past that is deliberate). Not a key event:
 * nothing injected (IR, the host link's Key) can make one, only the key's pin. True once per hold. */
#define CONFIRM_HOLD_MS 5000
bool inputs_confirm_take(void);
/* ON/OFF is down now (debounced): B&W's bootloader takes it held 3 s as "back to the app", so nothing goes there while
 * a confirmation's hold lasts */
bool inputs_onoff_down(void);

#endif
