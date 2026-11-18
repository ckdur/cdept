/* Stand-in for <X11/Xfuncproto.h>: the few macros the X server code in shim/mi uses.
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#ifndef CDEPT_XFUNCPROTO_SHIM_H
#define CDEPT_XFUNCPROTO_SHIM_H
#define _X_INLINE inline
#define _X_EXPORT
#define _X_UNUSED __attribute__((unused))
#endif
