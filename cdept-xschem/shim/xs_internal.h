/* xs_internal.h: shared between xshim.c (stand-in Xlib and Tk) and display.c (the
 * drawing window as the Rust side sees it).
 *
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#ifndef CDEPT_XS_INTERNAL_H
#define CDEPT_XS_INTERNAL_H

#include <cairo.h>
#include <tk.h>
#include "xshim.h"

enum { XS_WINDOW = 1, XS_PIXMAP = 2, XS_BITMAP = 3 };

/* How many separate rectangles of damage a window keeps. A cross-hair cursor changes
 * two thin strips; as one rectangle that would be the whole window. */
#define XS_MAX_DAMAGE 8

struct xs_rect { int x1, y1, x2, y2; };

/* A window, a pixmap or a one-bit bitmap. */
typedef struct xs_drawable {
  XID id;
  int kind;
  int width, height;
  cairo_surface_t *surface; /* windows and pixmaps: ARGB32 image, every pixel opaque */
  cairo_t *cr;              /* context the X drawing calls use, created on first use */
  unsigned char *bits;      /* bitmaps: width * height bytes, 0 or 1 */
  /* windows: what changed since it was last asked for, as a few rectangles
   * (x1, y1 inclusive; x2, y2 exclusive) */
  struct xs_rect damage[XS_MAX_DAMAGE];
  int ndamage;
} xs_drawable;

xs_drawable *xs_lookup(XID id);

/* Give a window a new size. Its pixels start black; xschem redraws. */
void xs_window_resize(xs_drawable *win, int width, int height);

void xs_damage(xs_drawable *win, int x, int y, int width, int height);
/* The changed area as one rectangle, or as up to `max` rectangles (x, y, width, height
 * each; returns how many). Both forget the damage. */
int xs_take_damage(xs_drawable *win, int *x, int *y, int *width, int *height);
int xs_take_damage_list(xs_drawable *win, int *rects, int max);

/* The X window behind a Tk window path, or NULL if the Tcl side has no such window.
 * Created on first use. */
xs_drawable *xs_window_of_path(const char *path);

/* The windows xschem draws into besides the drawing window: the picture panes of its
 * dialogs (xschem's preview_window(): a Tk frame whose X window the C code draws in).
 * Fills up to `max` Tk paths and windows and returns how many. Windows whose widget is
 * gone are left out. The paths stay valid until the next call into Tcl. */
int xs_preview_windows(const char **paths, xs_drawable **wins, int max);

/* Call once xschem has started: see xshim.c. */
void xs_after_start(void);

/* The interpreter the stand-in Tk was initialised with (NULL without a display). */
Tcl_Interp *xs_interp(void);

/* display.c: the Tcl commands cdept::png, cdept::event, cdept::resize, cdept::damage,
 * cdept::colour, cdept::colours, cdept::window_resize, cdept::previews. */
void xs_register_display_commands(Tcl_Interp *interp);

#endif /* CDEPT_XS_INTERNAL_H */
