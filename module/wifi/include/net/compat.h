/* What this mac80211 (mainline 1ebebea8e8, April 2008) needs from a 2.6.25 kernel that 2.6.24.7 lacks. */
#ifndef _ZD1_COMPAT_H
#define _ZD1_COMPAT_H
#include <linux/wireless.h>
#ifndef IW_MODE_MESH
#define IW_MODE_MESH 7
#endif
#endif
