/* The rear RGB LED's PWM (PWM 1/2/3 on PB20/PB21/PB22: R, G, B), stock's setup: what the app's looks (led.c) and the
 * preloader's blink draw through. Colours in perceptual units (0..255): the PWM gets the square (gamma 2). */
#ifndef LED_PWM_H
#define LED_PWM_H
#include <stdint.h>

typedef struct {
    uint8_t r, g, b;
} rgb8_t;
#define LED_WHITE {255, 240, 120} /* little blue: a neutral white on this LED needs duties ~334/400/67 */

void led_pwm_init(void);               /* stock's: MCK/1024, CPOL 1, period 400; the pins to the PWM */
uint16_t led_pwm_duty(uint8_t p);      /* a perceptual level as a duty (0..400) */
void led_pwm_set(const uint16_t d[3]); /* R, G, B duties from the next period on; unchanged ones left alone */
void led_pwm_color(rgb8_t c);          /* c at its own levels */

#endif
