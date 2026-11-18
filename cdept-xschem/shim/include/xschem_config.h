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
/* HAS_LIBJPEG comes from the build (build.rs) when libjpeg is installed. */

/* xschem names its temporary folders (undo, downloads) with rand(), seeded with the
 * time in seconds cut to 16 bits. Helpers started in the same second (CDePT restoring
 * several tabs) then ask for the same names, and the ones that lose run without undo.
 * xs_srand() in xshim.c mixes the process id and a finer clock into the seed. */
#define srand xs_srand

#define PREFIX "/usr/local"
#define XSCHEM_LIBRARY_PATH { NULL }
#define USER_CONF_DIR "~/.xschem"
#define XSCHEM_SHAREDIR PREFIX "/share/xschem"

#endif /* CDEPT_XSCHEM_CONFIG_H */
