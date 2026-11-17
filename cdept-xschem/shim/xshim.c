/* xshim.c: the Xlib and Tk calls xschem makes, answered by the helper itself.
 *
 * Step 0 state: xschem runs with has_x = 0, so none of these are reached. They exist so
 * the program links without libX11 and libtk. Each one reports its first call, so a
 * code path that does reach them is noticed. Step 2 replaces the drawing calls with an
 * implementation on cairo image surfaces.
 *
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <cairo.h>
#include <tk.h>
#include "xshim.h"
#include "cairo-xlib.h"

/* Report the first call of a function that has no implementation yet. */
#define NOT_YET() \
  do { \
    static int reported = 0; \
    if(!reported) { \
      reported = 1; \
      fprintf(stderr, "cdept-xschem: %s is not implemented yet\n", __func__); \
    } \
  } while(0)

/* ---- display ---------------------------------------------------------------------------- */
Display *XOpenDisplay(const char *name) { (void)name; NOT_YET(); return NULL; }
int XCloseDisplay(Display *dpy) { (void)dpy; return 0; }
int XDefaultScreen(Display *dpy) { (void)dpy; return 0; }
Window xs_root_window(Display *dpy) { (void)dpy; return None; }
int xs_display_width(Display *dpy) { (void)dpy; return 0; }
int xs_display_height(Display *dpy) { (void)dpy; return 0; }
int XFlush(Display *dpy) { (void)dpy; return 0; }
int XSync(Display *dpy, Bool discard) { (void)dpy; (void)discard; return 0; }
int XPending(Display *dpy) { (void)dpy; return 0; }
int XFree(void *data) { free(data); return 1; }
long XMaxRequestSize(Display *dpy) { (void)dpy; return 65535; }
long XExtendedMaxRequestSize(Display *dpy) { (void)dpy; return 4194303; }
XErrorHandler XSetErrorHandler(XErrorHandler handler) { (void)handler; return NULL; }

int XGetErrorText(Display *dpy, int code, char *buffer, int length)
{
  (void)dpy;
  if(length > 0) snprintf(buffer, (size_t)length, "X error %d", code);
  return 0;
}

Atom XInternAtom(Display *dpy, const char *name, Bool only_if_exists)
{
  (void)dpy; (void)name; (void)only_if_exists;
  NOT_YET();
  return None;
}

Status XSendEvent(Display *dpy, Window w, Bool propagate, long mask, XEvent *event)
{
  (void)dpy; (void)w; (void)propagate; (void)mask; (void)event;
  NOT_YET();
  return 0;
}

int XGetKeyboardControl(Display *dpy, XKeyboardState *state)
{
  (void)dpy;
  memset(state, 0, sizeof(*state));
  return 1;
}

Status XMatchVisualInfo(Display *dpy, int screen, int depth, int class, XVisualInfo *info)
{
  (void)dpy; (void)screen; (void)depth; (void)class; (void)info;
  NOT_YET();
  return 0;
}

/* ---- windows and pixmaps ---------------------------------------------------------------- */
Window XCreateWindow(Display *dpy, Window parent, int x, int y, unsigned int width,
    unsigned int height, unsigned int border_width, int depth, unsigned int class,
    Visual *visual, unsigned long valuemask, XSetWindowAttributes *attributes)
{
  (void)dpy; (void)parent; (void)x; (void)y; (void)width; (void)height;
  (void)border_width; (void)depth; (void)class; (void)visual; (void)valuemask;
  (void)attributes;
  NOT_YET();
  return None;
}

int XDestroyWindow(Display *dpy, Window w) { (void)dpy; (void)w; NOT_YET(); return 0; }
int XMapRaised(Display *dpy, Window w) { (void)dpy; (void)w; NOT_YET(); return 0; }

Status XGetWindowAttributes(Display *dpy, Window w, XWindowAttributes *attributes)
{
  (void)dpy; (void)w;
  NOT_YET();
  memset(attributes, 0, sizeof(*attributes));
  return 0;
}

Status XQueryTree(Display *dpy, Window w, Window *root, Window *parent, Window **children,
    unsigned int *nchildren)
{
  (void)dpy; (void)w;
  NOT_YET();
  *root = None;
  *parent = None;
  *children = NULL;
  *nchildren = 0;
  return 0;
}

Bool XQueryPointer(Display *dpy, Window w, Window *root, Window *child, int *root_x,
    int *root_y, int *win_x, int *win_y, unsigned int *mask)
{
  (void)dpy; (void)w;
  NOT_YET();
  *root = None;
  *child = None;
  *root_x = *root_y = *win_x = *win_y = 0;
  *mask = 0;
  return False;
}

XWMHints *XAllocWMHints(void) { return calloc(1, sizeof(XWMHints)); }
int XSetWMHints(Display *dpy, Window w, XWMHints *hints) { (void)dpy; (void)w; (void)hints; return 0; }

Pixmap XCreatePixmap(Display *dpy, Drawable d, unsigned int width, unsigned int height,
    unsigned int depth)
{
  (void)dpy; (void)d; (void)width; (void)height; (void)depth;
  NOT_YET();
  return None;
}

Pixmap XCreateBitmapFromData(Display *dpy, Drawable d, const char *data, unsigned int width,
    unsigned int height)
{
  (void)dpy; (void)d; (void)data; (void)width; (void)height;
  NOT_YET();
  return None;
}

int XFreePixmap(Display *dpy, Pixmap pixmap) { (void)dpy; (void)pixmap; NOT_YET(); return 0; }

int XpmCreatePixmapFromData(Display *dpy, Drawable d, char **data, Pixmap *pixmap,
    Pixmap *mask, void *attributes)
{
  (void)dpy; (void)d; (void)data; (void)attributes;
  NOT_YET();
  *pixmap = None;
  *mask = None;
  return -1;
}

int XpmWriteFileFromPixmap(Display *dpy, const char *filename, Pixmap pixmap, Pixmap mask,
    void *attributes)
{
  (void)dpy; (void)filename; (void)pixmap; (void)mask; (void)attributes;
  NOT_YET();
  return -1;
}

/* ---- colours -------------------------------------------------------------------------- */
Status XAllocColor(Display *dpy, Colormap colormap, XColor *color)
{
  (void)dpy; (void)colormap; (void)color;
  NOT_YET();
  return 0;
}

Status XAllocNamedColor(Display *dpy, Colormap colormap, const char *name, XColor *screen,
    XColor *exact)
{
  (void)dpy; (void)colormap; (void)name; (void)screen; (void)exact;
  NOT_YET();
  return 0;
}

Status XLookupColor(Display *dpy, Colormap colormap, const char *name, XColor *exact,
    XColor *screen)
{
  (void)dpy; (void)colormap; (void)name; (void)exact; (void)screen;
  NOT_YET();
  return 0;
}

int XQueryColor(Display *dpy, Colormap colormap, XColor *color)
{
  (void)dpy; (void)colormap; (void)color;
  NOT_YET();
  return 0;
}

/* ---- graphics contexts ----------------------------------------------------------------- */
GC XCreateGC(Display *dpy, Drawable d, unsigned long valuemask, XGCValues *values)
{
  (void)dpy; (void)d; (void)valuemask; (void)values;
  NOT_YET();
  return NULL;
}

int XChangeGC(Display *dpy, GC gc, unsigned long valuemask, XGCValues *values)
{
  (void)dpy; (void)gc; (void)valuemask; (void)values;
  NOT_YET();
  return 0;
}

int XFreeGC(Display *dpy, GC gc) { (void)dpy; (void)gc; NOT_YET(); return 0; }

int XSetForeground(Display *dpy, GC gc, unsigned long foreground)
{
  (void)dpy; (void)gc; (void)foreground;
  NOT_YET();
  return 0;
}

int XSetBackground(Display *dpy, GC gc, unsigned long background)
{
  (void)dpy; (void)gc; (void)background;
  NOT_YET();
  return 0;
}

int XSetLineAttributes(Display *dpy, GC gc, unsigned int line_width, int line_style,
    int cap_style, int join_style)
{
  (void)dpy; (void)gc; (void)line_width; (void)line_style; (void)cap_style;
  (void)join_style;
  NOT_YET();
  return 0;
}

int XSetDashes(Display *dpy, GC gc, int dash_offset, const char *dash_list, int n)
{
  (void)dpy; (void)gc; (void)dash_offset; (void)dash_list; (void)n;
  NOT_YET();
  return 0;
}

int XSetFillStyle(Display *dpy, GC gc, int fill_style)
{
  (void)dpy; (void)gc; (void)fill_style;
  NOT_YET();
  return 0;
}

int XSetTile(Display *dpy, GC gc, Pixmap tile) { (void)dpy; (void)gc; (void)tile; NOT_YET(); return 0; }

int XSetStipple(Display *dpy, GC gc, Pixmap stipple)
{
  (void)dpy; (void)gc; (void)stipple;
  NOT_YET();
  return 0;
}

int XSetClipMask(Display *dpy, GC gc, Pixmap pixmap)
{
  (void)dpy; (void)gc; (void)pixmap;
  NOT_YET();
  return 0;
}

int XSetClipRectangles(Display *dpy, GC gc, int clip_x_origin, int clip_y_origin,
    XRectangle *rectangles, int n, int ordering)
{
  (void)dpy; (void)gc; (void)clip_x_origin; (void)clip_y_origin; (void)rectangles;
  (void)n; (void)ordering;
  NOT_YET();
  return 0;
}

/* ---- drawing ---------------------------------------------------------------------------- */
int XDrawLine(Display *dpy, Drawable d, GC gc, int x1, int y1, int x2, int y2)
{
  (void)dpy; (void)d; (void)gc; (void)x1; (void)y1; (void)x2; (void)y2;
  NOT_YET();
  return 0;
}

int XDrawLines(Display *dpy, Drawable d, GC gc, XPoint *points, int npoints, int mode)
{
  (void)dpy; (void)d; (void)gc; (void)points; (void)npoints; (void)mode;
  NOT_YET();
  return 0;
}

int XDrawSegments(Display *dpy, Drawable d, GC gc, XSegment *segments, int nsegments)
{
  (void)dpy; (void)d; (void)gc; (void)segments; (void)nsegments;
  NOT_YET();
  return 0;
}

int XDrawPoints(Display *dpy, Drawable d, GC gc, XPoint *points, int npoints, int mode)
{
  (void)dpy; (void)d; (void)gc; (void)points; (void)npoints; (void)mode;
  NOT_YET();
  return 0;
}

int XDrawRectangle(Display *dpy, Drawable d, GC gc, int x, int y, unsigned int width,
    unsigned int height)
{
  (void)dpy; (void)d; (void)gc; (void)x; (void)y; (void)width; (void)height;
  NOT_YET();
  return 0;
}

int XDrawRectangles(Display *dpy, Drawable d, GC gc, XRectangle *rectangles, int n)
{
  (void)dpy; (void)d; (void)gc; (void)rectangles; (void)n;
  NOT_YET();
  return 0;
}

int XDrawArc(Display *dpy, Drawable d, GC gc, int x, int y, unsigned int width,
    unsigned int height, int angle1, int angle2)
{
  (void)dpy; (void)d; (void)gc; (void)x; (void)y; (void)width; (void)height;
  (void)angle1; (void)angle2;
  NOT_YET();
  return 0;
}

int XDrawArcs(Display *dpy, Drawable d, GC gc, XArc *arcs, int narcs)
{
  (void)dpy; (void)d; (void)gc; (void)arcs; (void)narcs;
  NOT_YET();
  return 0;
}

int XFillRectangle(Display *dpy, Drawable d, GC gc, int x, int y, unsigned int width,
    unsigned int height)
{
  (void)dpy; (void)d; (void)gc; (void)x; (void)y; (void)width; (void)height;
  NOT_YET();
  return 0;
}

int XFillRectangles(Display *dpy, Drawable d, GC gc, XRectangle *rectangles, int n)
{
  (void)dpy; (void)d; (void)gc; (void)rectangles; (void)n;
  NOT_YET();
  return 0;
}

int XFillArc(Display *dpy, Drawable d, GC gc, int x, int y, unsigned int width,
    unsigned int height, int angle1, int angle2)
{
  (void)dpy; (void)d; (void)gc; (void)x; (void)y; (void)width; (void)height;
  (void)angle1; (void)angle2;
  NOT_YET();
  return 0;
}

int XFillArcs(Display *dpy, Drawable d, GC gc, XArc *arcs, int narcs)
{
  (void)dpy; (void)d; (void)gc; (void)arcs; (void)narcs;
  NOT_YET();
  return 0;
}

int XFillPolygon(Display *dpy, Drawable d, GC gc, XPoint *points, int npoints, int shape,
    int mode)
{
  (void)dpy; (void)d; (void)gc; (void)points; (void)npoints; (void)shape; (void)mode;
  NOT_YET();
  return 0;
}

int XCopyArea(Display *dpy, Drawable src, Drawable dest, GC gc, int src_x, int src_y,
    unsigned int width, unsigned int height, int dest_x, int dest_y)
{
  (void)dpy; (void)src; (void)dest; (void)gc; (void)src_x; (void)src_y; (void)width;
  (void)height; (void)dest_x; (void)dest_y;
  NOT_YET();
  return 0;
}

/* ---- cairo surfaces on our drawables --------------------------------------------------- */
cairo_surface_t *cairo_xlib_surface_create(Display *dpy, Drawable drawable, Visual *visual,
    int width, int height)
{
  (void)dpy; (void)drawable; (void)visual;
  NOT_YET();
  return cairo_image_surface_create(CAIRO_FORMAT_RGB24, width, height);
}

int cairo_xlib_surface_get_width(cairo_surface_t *surface)
{
  return cairo_image_surface_get_width(surface);
}

int cairo_xlib_surface_get_height(cairo_surface_t *surface)
{
  return cairo_image_surface_get_height(surface);
}

void cairo_xlib_surface_set_size(cairo_surface_t *surface, int width, int height)
{
  (void)surface; (void)width; (void)height;
  NOT_YET();
}

/* ---- Tk ----------------------------------------------------------------------------------- */
int Tk_Init(Tcl_Interp *interp) { (void)interp; NOT_YET(); return TCL_OK; }
void Tk_MainLoop(void) { NOT_YET(); }
void Tk_Main(int argc, char **argv, Tcl_AppInitProc *init) { (void)argc; (void)argv; (void)init; NOT_YET(); }

Tk_Window Tk_MainWindow(Tcl_Interp *interp) { (void)interp; NOT_YET(); return NULL; }

Tk_Window Tk_NameToWindow(Tcl_Interp *interp, const char *path, Tk_Window main)
{
  (void)interp; (void)path; (void)main;
  NOT_YET();
  return NULL;
}

void Tk_MakeWindowExist(Tk_Window win) { (void)win; NOT_YET(); }
void Tk_DestroyWindow(Tk_Window win) { (void)win; NOT_YET(); }
Window Tk_WindowId(Tk_Window win) { (void)win; NOT_YET(); return None; }
Display *Tk_Display(Tk_Window win) { (void)win; NOT_YET(); return NULL; }
Visual *Tk_Visual(Tk_Window win) { (void)win; NOT_YET(); return NULL; }
Colormap Tk_Colormap(Tk_Window win) { (void)win; NOT_YET(); return None; }
int Tk_Depth(Tk_Window win) { (void)win; NOT_YET(); return 24; }
int Tk_ScreenNumber(Tk_Window win) { (void)win; NOT_YET(); return 0; }

void Tk_ChangeWindowAttributes(Tk_Window win, unsigned long valuemask,
    XSetWindowAttributes *attributes)
{
  (void)win; (void)valuemask; (void)attributes;
  NOT_YET();
}

XColor *Tk_GetColor(Tcl_Interp *interp, Tk_Window win, const char *name)
{
  (void)interp; (void)win; (void)name;
  NOT_YET();
  return NULL;
}

Pixmap Tk_GetPixmap(Display *dpy, Drawable d, int width, int height, int depth)
{
  (void)dpy; (void)d; (void)width; (void)height; (void)depth;
  NOT_YET();
  return None;
}

void Tk_FreePixmap(Display *dpy, Pixmap pixmap) { (void)dpy; (void)pixmap; NOT_YET(); }
