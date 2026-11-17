/* xschem_config.h: replaces the config.h that xschem's ./configure would generate.
 *
 * xschem.h skips "../config.h" when NO_SCCONFIG is defined; the build passes that and
 * force-includes this file. Paths here are fallbacks only: the helper sets
 * XSCHEM_SHAREDIR in the environment and the library path in the generated rc file.
 *
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#ifndef CDEPT_XSCHEM_CONFIG_H
#define CDEPT_XSCHEM_CONFIG_H

#define _XOPEN_SOURCE

#define HAS_CAIRO 1
#define HAS_DUP2 1
#define HAS_POPEN 1
/* HAS_LIBJPEG is left off: JPEG images embedded in schematics are not drawn yet. */

#define PREFIX "/usr/local"
#define XSCHEM_LIBRARY_PATH { NULL }
#define USER_CONF_DIR "~/.xschem"
#define XSCHEM_SHAREDIR PREFIX "/share/xschem"

#endif /* CDEPT_XSCHEM_CONFIG_H */
