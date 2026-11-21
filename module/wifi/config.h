/* This build's options, -include'd in every file (nix/module/wifi.nix; the make variables choose the objects): the
 * PID rate control built in and the default, nl80211. No mesh (an element 51 in a beacon is no mesh configuration),
 * no LEDs, no debugfs. Then what 2.6.24.7 lacks. */
#define CONFIG_MAC80211_RC_DEFAULT     "pid"
#define CONFIG_MAC80211_RC_PID         1
#define CONFIG_MAC80211_RC_DEFAULT_PID 1
#define CONFIG_NL80211                 1

#include <net/compat.h>
