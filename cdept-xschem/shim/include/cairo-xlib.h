/* Stand-in for <cairo-xlib.h>: cairo surfaces on the helper's own drawables.
 * The names are redirected to xs_ functions in xshim.h.
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#ifndef CDEPT_CAIRO_XLIB_SHIM_H
#define CDEPT_CAIRO_XLIB_SHIM_H

#include <cairo.h>
#include "xshim.h"

cairo_surface_t *cairo_xlib_surface_create(Display *dpy, Drawable drawable, Visual *visual,
    int width, int height);
int cairo_xlib_surface_get_width(cairo_surface_t *surface);
int cairo_xlib_surface_get_height(cairo_surface_t *surface);
void cairo_xlib_surface_set_size(cairo_surface_t *surface, int width, int height);

/* xschem shows text with cairo_show_text(). On an image surface cairo cuts the edge
 * pixels of some glyphs, which an X server does not: see xs_cairo_show_text() in
 * xshim.c. */
void xs_cairo_show_text(cairo_t *cr, const char *utf8);
#define cairo_show_text xs_cairo_show_text

#endif /* CDEPT_CAIRO_XLIB_SHIM_H */
