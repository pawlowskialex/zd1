#include <gpio.h>
#include <pwm.h>
#include "led_pwm.h"

#define PERIOD 400

/* Stock PWM setup (zd1 emulator trace of 2.04.05): MR = 0, CMR = 0x20A (MCK/1024, CPOL = 1), CPRD = 400,
 * channels 1..3 on PB20/PB21/PB22 (function A), ENA = 0x0E. */
void led_pwm_init(void) {
    static const gpio_map_t pins = {{AVR32_PWM_1_PIN, AVR32_PWM_1_FUNCTION},
                                    {AVR32_PWM_2_PIN, AVR32_PWM_2_FUNCTION},
                                    {AVR32_PWM_3_PIN, AVR32_PWM_3_FUNCTION}};
    const pwm_opt_t opt = {.diva = 0, .divb = 0, .prea = 0, .preb = 0};
    avr32_pwm_channel_t c;
    c.cmr = 0x20A;
    c.cdty = 0;
    c.cprd = PERIOD;
    pwm_init(&opt);
    for (unsigned ch = 1; ch <= 3; ch++)
        pwm_channel_init(ch, &c);
    gpio_enable_module(pins, 3);
    pwm_start_channels(0x0E);
}

uint16_t led_pwm_duty(uint8_t p) { return (uint16_t)((uint32_t)p * p * PERIOD / 65025); }

/* CUPD with CMR.CPD = 0 -> new duty at the next period boundary. Async: ASF's sync variant busy-waits for the
 * period end (~8.5 ms per channel), which would stall the cooperative main loop. */
void led_pwm_set(const uint16_t d[3]) {
    static uint16_t last[3] = {0xFFFF, 0xFFFF, 0xFFFF};
    for (unsigned i = 0; i < 3; i++)
        if (d[i] != last[i]) {
            avr32_pwm_channel_t c;
            c.cmr = 0x20A; /* anonymous union: GCC 4.4 can't designate it in an initializer */
            c.cupd = d[i] > PERIOD ? PERIOD : d[i];
            pwm_async_update_channel(i + 1, &c);
            last[i] = d[i];
        }
}

void led_pwm_color(rgb8_t c) {
    const uint16_t d[3] = {led_pwm_duty(c.r), led_pwm_duty(c.g), led_pwm_duty(c.b)};
    led_pwm_set(d);
}
