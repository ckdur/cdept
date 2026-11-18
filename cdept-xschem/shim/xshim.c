/* xshim.c: the Xlib and Tk calls xschem makes, answered by the helper itself.
 *
 * There is no X server and no Tk. Windows and pixmaps are cairo image surfaces in this
 * process, and the drawing calls below paint on them the way an X server does, so that
 * the picture matches stock xschem pixel for pixel:
 *  - lines of width 1 and more, rectangle outlines, arcs, filled arcs and filled
 *    polygons go through the X server's own code (shim/mi, see xs_mi.h);
 *  - filled rectangles, copies and the few thin (width 0) slanted lines through cairo,
 *    without antialiasing;
 *  - xschem's own cairo code (text, images) draws on the same surfaces: see
 *    cairo_xlib_surface_create() and xs_cairo_show_text().
 *
 * Without a display (cdx_init(..., 0), has_x = 0) none of this is reached.
 *
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <cairo.h>
#include <tk.h>
#include "xshim.h"
#include "cairo-xlib.h"
#include "xs_internal.h"

#ifndef M_PI
#define M_PI 3.14159265358979323846
#endif

/* Report the first call of something the helper has no use for, so that a code path
 * relying on it is noticed in the log. */
#define UNSUPPORTED() \
  do { \
    static int reported = 0; \
    if(!reported) { \
      reported = 1; \
      fprintf(stderr, "cdept-xschem: %s is not supported\n", __func__); \
    } \
  } while(0)

/* ---- drawables --------------------------------------------------------------------------- */

#define XS_MAX_DRAWABLES 1024
#define XS_ID_BASE 0x1000000UL /* ids are XS_ID_BASE + slot + 1 */

static xs_drawable *drawables[XS_MAX_DRAWABLES];

struct xs_display { int unused; };
static Display the_display;
static Visual the_visual = { 0x21, 24 };
static Window root_window = None;
static Tcl_Interp *the_interp = NULL;

Tcl_Interp *xs_interp(void) { return the_interp; }

xs_drawable *xs_lookup(XID id)
{
  unsigned long slot;
  if(id <= XS_ID_BASE) return NULL;
  slot = id - XS_ID_BASE - 1;
  if(slot >= XS_MAX_DRAWABLES) return NULL;
  return drawables[slot];
}

static cairo_surface_t *new_image(int width, int height)
{
  cairo_surface_t *surface;
  cairo_t *cr;
  if(width < 1) width = 1;
  if(height < 1) height = 1;
  surface = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, width, height);
  /* Opaque black: every drawing call keeps the surface opaque from here on. */
  cr = cairo_create(surface);
  cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
  cairo_paint(cr);
  cairo_destroy(cr);
  return surface;
}

static xs_drawable *new_drawable(int kind, int width, int height)
{
  int slot;
  xs_drawable *d;
  for(slot = 0; slot < XS_MAX_DRAWABLES; ++slot) if(!drawables[slot]) break;
  if(slot == XS_MAX_DRAWABLES) {
    fprintf(stderr, "cdept-xschem: too many windows and pixmaps\n");
    return NULL;
  }
  d = calloc(1, sizeof(*d));
  if(!d) return NULL;
  d->id = XS_ID_BASE + (unsigned long)slot + 1;
  d->kind = kind;
  d->width = width < 1 ? 1 : width;
  d->height = height < 1 ? 1 : height;
  if(kind == XS_BITMAP) d->bits = calloc((size_t)d->width * (size_t)d->height, 1);
  else d->surface = new_image(d->width, d->height);
  drawables[slot] = d;
  return d;
}

static void free_drawable(XID id)
{
  xs_drawable *d = xs_lookup(id);
  if(!d) return;
  drawables[id - XS_ID_BASE - 1] = NULL;
  if(d->cr) cairo_destroy(d->cr);
  /* xschem may still hold cairo surfaces made from this one: they keep it alive. */
  if(d->surface) cairo_surface_destroy(d->surface);
  free(d->bits);
  free(d);
}

void xs_window_resize(xs_drawable *win, int width, int height)
{
  if(!win || win->kind != XS_WINDOW) return;
  if(width < 1) width = 1;
  if(height < 1) height = 1;
  if(width == win->width && height == win->height) return;
  if(win->cr) cairo_destroy(win->cr);
  win->cr = NULL;
  cairo_surface_destroy(win->surface);
  win->surface = new_image(width, height);
  win->width = width;
  win->height = height;
  win->ndamage = 0;
  xs_damage(win, 0, 0, width, height);
}

static long rect_area(const struct xs_rect *r)
{
  return (long)(r->x2 - r->x1) * (long)(r->y2 - r->y1);
}

static struct xs_rect rect_union(const struct xs_rect *a, const struct xs_rect *b)
{
  struct xs_rect u;
  u.x1 = a->x1 < b->x1 ? a->x1 : b->x1;
  u.y1 = a->y1 < b->y1 ? a->y1 : b->y1;
  u.x2 = a->x2 > b->x2 ? a->x2 : b->x2;
  u.y2 = a->y2 > b->y2 ? a->y2 : b->y2;
  return u;
}

/* Record that a rectangle of a window changed. Rectangles are merged when that costs
 * little (the merged one is not much larger than the two), or when there are too many. */
void xs_damage(xs_drawable *win, int x, int y, int width, int height)
{
  struct xs_rect r;
  int i, best = -1;
  long best_growth = 0;
  if(!win || win->kind != XS_WINDOW) return;
  r.x1 = x < 0 ? 0 : x;
  r.y1 = y < 0 ? 0 : y;
  r.x2 = x + width > win->width ? win->width : x + width;
  r.y2 = y + height > win->height ? win->height : y + height;
  if(r.x2 <= r.x1 || r.y2 <= r.y1) return;
  for(i = 0; i < win->ndamage; ++i) {
    struct xs_rect u = rect_union(&win->damage[i], &r);
    long growth = rect_area(&u) - rect_area(&win->damage[i]) - rect_area(&r);
    /* 4096 pixels of slack: small things next to each other become one */
    if(growth <= 4096 + (rect_area(&win->damage[i]) + rect_area(&r)) / 4) {
      win->damage[i] = u;
      return;
    }
    if(best < 0 || growth < best_growth) {
      best = i;
      best_growth = growth;
    }
  }
  if(win->ndamage < XS_MAX_DAMAGE) win->damage[win->ndamage++] = r;
  else win->damage[best] = rect_union(&win->damage[best], &r);
}

int xs_take_damage_list(xs_drawable *win, int *rects, int max)
{
  int i, n;
  if(!win || max < 1) return 0;
  /* more than the caller can take: fold the last ones together */
  while(win->ndamage > max) {
    win->damage[max - 1] = rect_union(&win->damage[max - 1], &win->damage[win->ndamage - 1]);
    win->ndamage--;
  }
  n = win->ndamage;
  for(i = 0; i < n; ++i) {
    rects[4 * i] = win->damage[i].x1;
    rects[4 * i + 1] = win->damage[i].y1;
    rects[4 * i + 2] = win->damage[i].x2 - win->damage[i].x1;
    rects[4 * i + 3] = win->damage[i].y2 - win->damage[i].y1;
  }
  win->ndamage = 0;
  return n;
}

int xs_take_damage(xs_drawable *win, int *x, int *y, int *width, int *height)
{
  int rect[4];
  if(!xs_take_damage_list(win, rect, 1)) return 0;
  *x = rect[0];
  *y = rect[1];
  *width = rect[2];
  *height = rect[3];
  return 1;
}

static Window root(void)
{
  if(root_window == None) {
    xs_drawable *d = new_drawable(XS_WINDOW, 1, 1);
    if(d) root_window = d->id;
  }
  return root_window;
}

/* ---- display ---------------------------------------------------------------------------- */

/* Only xschem's main() opens a display itself, and main.c is not compiled. */
Display *XOpenDisplay(const char *name) { (void)name; return &the_display; }
int XCloseDisplay(Display *dpy) { (void)dpy; return 0; }
int XDefaultScreen(Display *dpy) { (void)dpy; return 0; }
Window xs_root_window(Display *dpy) { (void)dpy; return root(); }
/* A screen size for code that wants one (placing dialogs): there is no screen. */
int xs_display_width(Display *dpy) { (void)dpy; return 1920; }
int xs_display_height(Display *dpy) { (void)dpy; return 1080; }
int XFlush(Display *dpy) { (void)dpy; return 0; }
int XSync(Display *dpy, Bool discard) { (void)dpy; (void)discard; return 0; }
/* xschem asks this to drop redraws while more input is queued; ours is never queued
 * in X. */
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

/* Atoms and client messages are how xschem talks to the window manager (full screen,
 * window icon). There is none: accept and do nothing. */
Atom XInternAtom(Display *dpy, const char *name, Bool only_if_exists)
{
  static Atom next = 100;
  (void)dpy; (void)name; (void)only_if_exists;
  return next++;
}

Status XSendEvent(Display *dpy, Window w, Bool propagate, long mask, XEvent *event)
{
  (void)dpy; (void)w; (void)propagate; (void)mask; (void)event;
  return 1;
}

int XGetKeyboardControl(Display *dpy, XKeyboardState *state)
{
  (void)dpy;
  memset(state, 0, sizeof(*state));
  return 1;
}

Status XMatchVisualInfo(Display *dpy, int screen, int depth, int class, XVisualInfo *info)
{
  (void)dpy; (void)screen; (void)class;
  memset(info, 0, sizeof(*info));
  info->visual = &the_visual;
  info->visualid = the_visual.visualid;
  info->depth = depth;
  info->class = TrueColor;
  info->red_mask = 0xff0000;
  info->green_mask = 0x00ff00;
  info->blue_mask = 0x0000ff;
  info->bits_per_rgb = 8;
  return 1;
}

/* ---- windows and pixmaps ---------------------------------------------------------------- */

Window XCreateWindow(Display *dpy, Window parent, int x, int y, unsigned int width,
    unsigned int height, unsigned int border_width, int depth, unsigned int class,
    Visual *visual, unsigned long valuemask, XSetWindowAttributes *attributes)
{
  xs_drawable *d;
  (void)dpy; (void)parent; (void)x; (void)y; (void)border_width; (void)depth; (void)class;
  (void)visual; (void)valuemask; (void)attributes;
  /* Only the screen grab asks for a window of its own (covering the screen). It
   * cannot work here; keep the window tiny. */
  (void)width; (void)height;
  d = new_drawable(XS_WINDOW, 1, 1);
  return d ? d->id : None;
}

int XDestroyWindow(Display *dpy, Window w)
{
  (void)dpy;
  if(w != root_window) free_drawable(w);
  return 0;
}

int XMapRaised(Display *dpy, Window w) { (void)dpy; (void)w; return 0; }

Status XGetWindowAttributes(Display *dpy, Window w, XWindowAttributes *attributes)
{
  static Screen screen;
  xs_drawable *d = xs_lookup(w);
  (void)dpy;
  memset(attributes, 0, sizeof(*attributes));
  if(!d || d->kind != XS_WINDOW) return 0;
  screen.display = &the_display;
  screen.root = root();
  screen.width = xs_display_width(&the_display);
  screen.height = xs_display_height(&the_display);
  attributes->width = d->width;
  attributes->height = d->height;
  attributes->depth = 24;
  attributes->visual = &the_visual;
  attributes->root = root();
  attributes->class = InputOutput;
  attributes->map_state = IsViewable;
  attributes->screen = &screen;
  return 1;
}

Status XQueryTree(Display *dpy, Window w, Window *root_return, Window *parent,
    Window **children, unsigned int *nchildren)
{
  (void)dpy; (void)w;
  *root_return = root();
  *parent = root();
  *children = NULL;
  *nchildren = 0;
  return 1;
}

/* Last pointer position in the drawing window, kept by display.c. */
int xs_pointer_x = 0, xs_pointer_y = 0;

Bool XQueryPointer(Display *dpy, Window w, Window *root_return, Window *child, int *root_x,
    int *root_y, int *win_x, int *win_y, unsigned int *mask)
{
  (void)dpy; (void)w;
  *root_return = root();
  *child = None;
  *root_x = *win_x = xs_pointer_x;
  *root_y = *win_y = xs_pointer_y;
  *mask = 0;
  return True;
}

XWMHints *XAllocWMHints(void) { return calloc(1, sizeof(XWMHints)); }
int XSetWMHints(Display *dpy, Window w, XWMHints *hints) { (void)dpy; (void)w; (void)hints; return 0; }

Pixmap XCreatePixmap(Display *dpy, Drawable d, unsigned int width, unsigned int height,
    unsigned int depth)
{
  xs_drawable *p;
  (void)dpy; (void)d;
  p = new_drawable(depth == 1 ? XS_BITMAP : XS_PIXMAP, (int)width, (int)height);
  return p ? p->id : None;
}

/* XBM layout: rows of (width + 7) / 8 bytes, least significant bit first. */
Pixmap XCreateBitmapFromData(Display *dpy, Drawable d, const char *data, unsigned int width,
    unsigned int height)
{
  unsigned int x, y, row_bytes = (width + 7) / 8;
  xs_drawable *p;
  (void)dpy; (void)d;
  p = new_drawable(XS_BITMAP, (int)width, (int)height);
  if(!p || !p->bits) return None;
  for(y = 0; y < height; ++y) {
    for(x = 0; x < width; ++x) {
      unsigned char byte = (unsigned char)data[y * row_bytes + x / 8];
      p->bits[y * width + x] = (unsigned char)((byte >> (x % 8)) & 1);
    }
  }
  return p->id;
}

int XFreePixmap(Display *dpy, Pixmap pixmap) { (void)dpy; free_drawable(pixmap); return 0; }

/* The window icon. Nobody would see it. */
int XpmCreatePixmapFromData(Display *dpy, Drawable d, char **data, Pixmap *pixmap,
    Pixmap *mask, void *attributes)
{
  (void)dpy; (void)d; (void)data; (void)attributes;
  *pixmap = None;
  *mask = None;
  return -1;
}

int XpmWriteFileFromPixmap(Display *dpy, const char *filename, Pixmap pixmap, Pixmap mask,
    void *attributes)
{
  (void)dpy; (void)filename; (void)pixmap; (void)mask; (void)attributes;
  UNSUPPORTED();
  return -1;
}

/* ---- colours -------------------------------------------------------------------------- */
/* TrueColor: a pixel value is 0xRRGGBB. */

#include "xs_colours.h"

static int compare_colour_name(const void *key, const void *entry)
{
  return strcmp((const char *)key, ((const struct xs_named_colour *)entry)->name);
}

/* "#rgb", "#rrggbb", "#rrrgggbbb", "#rrrrggggbbbb", or an X colour name (any case,
 * with or without spaces: "Light Sky Blue", "grey50"). */
static int parse_colour(const char *name, unsigned long *rgb)
{
  size_t i, len;
  if(!name) return 0;
  while(*name == ' ') ++name;
  len = strlen(name);
  while(len > 0 && name[len - 1] == ' ') --len;
  if(name[0] == '#') {
    size_t digits = len - 1, per = digits / 3;
    unsigned long channel[3];
    int c;
    if(digits == 0 || digits % 3 || per > 4) return 0;
    for(c = 0; c < 3; ++c) {
      unsigned long v = 0;
      for(i = 0; i < per; ++i) {
        char ch = name[1 + (size_t)c * per + i];
        int digit;
        if(ch >= '0' && ch <= '9') digit = ch - '0';
        else if(ch >= 'a' && ch <= 'f') digit = ch - 'a' + 10;
        else if(ch >= 'A' && ch <= 'F') digit = ch - 'A' + 10;
        else return 0;
        v = v * 16 + (unsigned long)digit;
      }
      /* scale to 8 bits: the most significant digits count */
      if(per == 1) v = v * 17;
      else if(per == 3) v >>= 4;
      else if(per == 4) v >>= 8;
      channel[c] = v;
    }
    *rgb = (channel[0] << 16) | (channel[1] << 8) | channel[2];
    return 1;
  } else {
    char lower[32];
    const struct xs_named_colour *found;
    size_t n = 0;
    for(i = 0; i < len; ++i) {
      char ch = name[i];
      if(ch == ' ') continue;
      if(n + 1 >= sizeof(lower)) return 0; /* longer than any colour name */
      lower[n++] = (char)((ch >= 'A' && ch <= 'Z') ? ch - 'A' + 'a' : ch);
    }
    lower[n] = '\0';
    found = bsearch(lower, xs_named_colours, sizeof(xs_named_colours) / sizeof(xs_named_colours[0]),
                    sizeof(xs_named_colours[0]), compare_colour_name);
    if(!found) return 0;
    *rgb = found->rgb;
    return 1;
  }
}

static void fill_xcolor(XColor *color, unsigned long rgb)
{
  color->pixel = rgb;
  color->red = (unsigned short)(((rgb >> 16) & 0xff) * 257);
  color->green = (unsigned short)(((rgb >> 8) & 0xff) * 257);
  color->blue = (unsigned short)((rgb & 0xff) * 257);
  color->flags = DoRed | DoGreen | DoBlue;
}

Status XAllocColor(Display *dpy, Colormap colormap, XColor *color)
{
  (void)dpy; (void)colormap;
  color->pixel = ((unsigned long)(color->red >> 8) << 16) |
                 ((unsigned long)(color->green >> 8) << 8) | (unsigned long)(color->blue >> 8);
  return 1;
}

Status XAllocNamedColor(Display *dpy, Colormap colormap, const char *name, XColor *screen,
    XColor *exact)
{
  unsigned long rgb;
  (void)dpy; (void)colormap;
  if(!parse_colour(name, &rgb)) return 0;
  fill_xcolor(screen, rgb);
  fill_xcolor(exact, rgb);
  return 1;
}

Status XLookupColor(Display *dpy, Colormap colormap, const char *name, XColor *exact,
    XColor *screen)
{
  return XAllocNamedColor(dpy, colormap, name, screen, exact);
}

int XQueryColor(Display *dpy, Colormap colormap, XColor *color)
{
  (void)dpy; (void)colormap;
  fill_xcolor(color, color->pixel & 0xffffff);
  return 1;
}

/* ---- graphics contexts ----------------------------------------------------------------- */

#define XS_MAX_DASHES 16

struct xs_gc {
  int function;
  unsigned long foreground, background;
  int line_width, line_style, cap_style, join_style;
  int fill_style, fill_rule;
  Pixmap tile, stipple;
  double dashes[XS_MAX_DASHES];
  int ndashes, dash_offset;
  int has_clip; /* 0: draw everywhere */
  int nclip;
  XRectangle *clip;
  int clip_x, clip_y;
  /* The stipple painted in the foreground colour, made on first use. */
  cairo_pattern_t *stipple_pattern;
  Pixmap pattern_stipple;
  unsigned long pattern_foreground;
};

static void drop_stipple_pattern(GC gc)
{
  if(gc->stipple_pattern) cairo_pattern_destroy(gc->stipple_pattern);
  gc->stipple_pattern = NULL;
}

int XChangeGC(Display *dpy, GC gc, unsigned long valuemask, XGCValues *values)
{
  (void)dpy;
  if(!gc || !values) return 0;
  if(valuemask & GCFunction) gc->function = values->function;
  if(valuemask & GCForeground) gc->foreground = values->foreground;
  if(valuemask & GCBackground) gc->background = values->background;
  if(valuemask & GCLineWidth) gc->line_width = values->line_width;
  if(valuemask & GCLineStyle) gc->line_style = values->line_style;
  if(valuemask & GCCapStyle) gc->cap_style = values->cap_style;
  if(valuemask & GCJoinStyle) gc->join_style = values->join_style;
  if(valuemask & GCFillStyle) gc->fill_style = values->fill_style;
  if(valuemask & GCFillRule) gc->fill_rule = values->fill_rule;
  if(valuemask & GCTile) gc->tile = values->tile;
  if(valuemask & GCStipple) gc->stipple = values->stipple;
  if(valuemask & GCDashOffset) gc->dash_offset = values->dash_offset;
  if(valuemask & GCDashList) {
    gc->dashes[0] = gc->dashes[1] = (unsigned char)values->dashes;
    gc->ndashes = 2;
  }
  return 1;
}

GC XCreateGC(Display *dpy, Drawable d, unsigned long valuemask, XGCValues *values)
{
  GC gc = calloc(1, sizeof(*gc));
  (void)d;
  if(!gc) return NULL;
  /* X defaults */
  gc->function = GXcopy;
  gc->foreground = 0x000000;
  gc->background = 0xffffff;
  gc->line_style = LineSolid;
  gc->cap_style = CapButt;
  gc->join_style = JoinMiter;
  gc->fill_style = FillSolid;
  gc->fill_rule = EvenOddRule;
  gc->dashes[0] = gc->dashes[1] = 4;
  gc->ndashes = 2;
  XChangeGC(dpy, gc, valuemask, values);
  return gc;
}

int XFreeGC(Display *dpy, GC gc)
{
  (void)dpy;
  if(!gc) return 0;
  drop_stipple_pattern(gc);
  free(gc->clip);
  free(gc);
  return 1;
}

int XSetForeground(Display *dpy, GC gc, unsigned long foreground)
{
  (void)dpy;
  if(gc) gc->foreground = foreground;
  return 1;
}

int XSetBackground(Display *dpy, GC gc, unsigned long background)
{
  (void)dpy;
  if(gc) gc->background = background;
  return 1;
}

int XSetLineAttributes(Display *dpy, GC gc, unsigned int line_width, int line_style,
    int cap_style, int join_style)
{
  (void)dpy;
  if(!gc) return 0;
  gc->line_width = (int)line_width;
  gc->line_style = line_style;
  gc->cap_style = cap_style;
  gc->join_style = join_style;
  return 1;
}

/* A list of one length means "on n, off n". */
int XSetDashes(Display *dpy, GC gc, int dash_offset, const char *dash_list, int n)
{
  int i;
  (void)dpy;
  if(!gc || n < 1) return 0;
  if(n > XS_MAX_DASHES) n = XS_MAX_DASHES;
  for(i = 0; i < n; ++i) gc->dashes[i] = (unsigned char)dash_list[i];
  if(n == 1) {
    gc->dashes[1] = gc->dashes[0];
    n = 2;
  }
  gc->ndashes = n;
  gc->dash_offset = dash_offset;
  return 1;
}

int XSetFillStyle(Display *dpy, GC gc, int fill_style)
{
  (void)dpy;
  if(gc) gc->fill_style = fill_style;
  return 1;
}

int XSetTile(Display *dpy, GC gc, Pixmap tile)
{
  (void)dpy;
  if(gc) gc->tile = tile;
  return 1;
}

int XSetStipple(Display *dpy, GC gc, Pixmap stipple)
{
  (void)dpy;
  if(gc) gc->stipple = stipple;
  return 1;
}

/* xschem only ever passes None: "no clipping". */
int XSetClipMask(Display *dpy, GC gc, Pixmap pixmap)
{
  (void)dpy;
  if(!gc) return 0;
  if(pixmap != None) UNSUPPORTED();
  gc->has_clip = 0;
  gc->nclip = 0;
  return 1;
}

int XSetClipRectangles(Display *dpy, GC gc, int clip_x_origin, int clip_y_origin,
    XRectangle *rectangles, int n, int ordering)
{
  XRectangle *copy = NULL;
  (void)dpy; (void)ordering;
  if(!gc) return 0;
  if(n > 0) {
    copy = malloc((size_t)n * sizeof(*copy));
    if(!copy) return 0;
    memcpy(copy, rectangles, (size_t)n * sizeof(*copy));
  }
  free(gc->clip);
  gc->clip = copy;
  gc->nclip = n > 0 ? n : 0;
  gc->has_clip = 1; /* with n == 0 nothing is drawn, as in X */
  gc->clip_x = clip_x_origin;
  gc->clip_y = clip_y_origin;
  return 1;
}

/* ---- drawing ---------------------------------------------------------------------------- */

static void set_rgb(cairo_t *cr, unsigned long pixel)
{
  cairo_set_source_rgb(cr, ((pixel >> 16) & 0xff) / 255.0, ((pixel >> 8) & 0xff) / 255.0,
                       (pixel & 0xff) / 255.0);
}

static cairo_pattern_t *stipple_pattern(GC gc)
{
  xs_drawable *bitmap = xs_lookup(gc->stipple);
  cairo_surface_t *surface;
  unsigned char *data;
  int stride, x, y;

  if(gc->stipple_pattern && gc->pattern_stipple == gc->stipple &&
     gc->pattern_foreground == gc->foreground) return gc->stipple_pattern;
  drop_stipple_pattern(gc);
  if(!bitmap || bitmap->kind != XS_BITMAP || !bitmap->bits) return NULL;
  surface = cairo_image_surface_create(CAIRO_FORMAT_ARGB32, bitmap->width, bitmap->height);
  cairo_surface_flush(surface);
  data = cairo_image_surface_get_data(surface);
  stride = cairo_image_surface_get_stride(surface);
  for(y = 0; y < bitmap->height; ++y) {
    uint32_t *row = (uint32_t *)(void *)(data + (size_t)y * (size_t)stride);
    for(x = 0; x < bitmap->width; ++x) {
      row[x] = bitmap->bits[y * bitmap->width + x]
                   ? (0xff000000u | (uint32_t)(gc->foreground & 0xffffff))
                   : 0u;
    }
  }
  cairo_surface_mark_dirty(surface);
  gc->stipple_pattern = cairo_pattern_create_for_surface(surface);
  cairo_surface_destroy(surface);
  cairo_pattern_set_extend(gc->stipple_pattern, CAIRO_EXTEND_REPEAT);
  cairo_pattern_set_filter(gc->stipple_pattern, CAIRO_FILTER_NEAREST);
  gc->pattern_stipple = gc->stipple;
  gc->pattern_foreground = gc->foreground;
  return gc->stipple_pattern;
}

/* What the GC paints with: a colour, the stipple, or the tile. Returns 0 when there is
 * nothing to paint with. */
static int set_source(cairo_t *cr, GC gc)
{
  if(gc->fill_style == FillTiled) {
    xs_drawable *tile = xs_lookup(gc->tile);
    if(!tile || !tile->surface) return 0;
    cairo_set_source_surface(cr, tile->surface, 0.0, 0.0);
    cairo_pattern_set_extend(cairo_get_source(cr), CAIRO_EXTEND_REPEAT);
    cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_NEAREST);
  } else if(gc->fill_style == FillStippled || gc->fill_style == FillOpaqueStippled) {
    cairo_pattern_t *pattern = stipple_pattern(gc);
    if(!pattern) return 0;
    cairo_set_source(cr, pattern);
  } else {
    set_rgb(cr, gc->foreground);
  }
  return 1;
}

/* Bounding box of the GC's clip rectangles; returns 0 if the GC draws nowhere. */
static int clip_box(GC gc, int *x1, int *y1, int *x2, int *y2)
{
  int i;
  if(!gc->has_clip) return 1;
  if(gc->nclip == 0) return 0;
  for(i = 0; i < gc->nclip; ++i) {
    int rx1 = gc->clip_x + gc->clip[i].x, ry1 = gc->clip_y + gc->clip[i].y;
    int rx2 = rx1 + gc->clip[i].width, ry2 = ry1 + gc->clip[i].height;
    if(i == 0) {
      *x1 = rx1; *y1 = ry1; *x2 = rx2; *y2 = ry2;
    } else {
      if(rx1 < *x1) *x1 = rx1;
      if(ry1 < *y1) *y1 = ry1;
      if(rx2 > *x2) *x2 = rx2;
      if(ry2 > *y2) *y2 = ry2;
    }
  }
  return 1;
}

static cairo_t *context_of(xs_drawable *d)
{
  if(!d->cr) {
    d->cr = cairo_create(d->surface);
    cairo_set_antialias(d->cr, CAIRO_ANTIALIAS_NONE);
  }
  return d->cr;
}

static void apply_clip(cairo_t *cr, GC gc)
{
  int i;
  if(!gc->has_clip) return;
  for(i = 0; i < gc->nclip; ++i) {
    cairo_rectangle(cr, gc->clip_x + gc->clip[i].x, gc->clip_y + gc->clip[i].y,
                    gc->clip[i].width, gc->clip[i].height);
  }
  cairo_clip(cr);
}

/* Start a drawing call: returns the context to draw with, clipped and with its source
 * set, or NULL when the call draws nothing. Must be paired with end_draw(). */
static cairo_t *begin_draw(xs_drawable *d, GC gc)
{
  cairo_t *cr;
  if(!d || !d->surface || !gc) return NULL;
  if(gc->function == GXnoop) return NULL;
  if(gc->has_clip && gc->nclip == 0) return NULL;
  cr = context_of(d);
  cairo_save(cr);
  apply_clip(cr, gc);
  if(!set_source(cr, gc)) {
    cairo_restore(cr);
    return NULL;
  }
  switch(gc->function) {
    case GXclear:
      cairo_set_source_rgb(cr, 0.0, 0.0, 0.0);
      break;
    case GXset:
      cairo_set_source_rgb(cr, 1.0, 1.0, 1.0);
      break;
    case GXxor:
      /* Used with a white foreground to invert; "difference" does the same. */
      cairo_set_operator(cr, CAIRO_OPERATOR_DIFFERENCE);
      break;
    default:
      break;
  }
  return cr;
}

/* Finish a drawing call that touched the box x1,y1 .. x2,y2 (exclusive). */
static void end_draw(xs_drawable *d, GC gc, cairo_t *cr, int x1, int y1, int x2, int y2)
{
  cairo_restore(cr);
  if(d->kind == XS_WINDOW) {
    int cx1 = 0, cy1 = 0, cx2 = d->width, cy2 = d->height;
    if(gc->has_clip) clip_box(gc, &cx1, &cy1, &cx2, &cy2);
    if(x1 < cx1) x1 = cx1;
    if(y1 < cy1) y1 = cy1;
    if(x2 > cx2) x2 = cx2;
    if(y2 > cy2) y2 = cy2;
    xs_damage(d, x1, y1, x2 - x1, y2 - y1);
  }
}

/* Line attributes of the GC. Returns how far a line reaches beyond its points. */
static int set_stroke(cairo_t *cr, GC gc)
{
  /* X: width 0 is a one pixel line. */
  int width = gc->line_width < 1 ? 1 : gc->line_width;
  int dashed = gc->line_style != LineSolid && gc->ndashes > 0;
  cairo_line_cap_t cap;
  cairo_line_join_t join;

  cairo_set_line_width(cr, width);
  if(width == 1 && !dashed) {
    /* Both end pixels are painted, whatever the cap style. */
    cap = CAIRO_LINE_CAP_SQUARE;
  } else {
    switch(gc->cap_style) {
      case CapRound: cap = CAIRO_LINE_CAP_ROUND; break;
      case CapProjecting: cap = CAIRO_LINE_CAP_SQUARE; break;
      default: cap = CAIRO_LINE_CAP_BUTT; break;
    }
  }
  switch(gc->join_style) {
    case JoinRound: join = CAIRO_LINE_JOIN_ROUND; break;
    case JoinBevel: join = CAIRO_LINE_JOIN_BEVEL; break;
    default: join = CAIRO_LINE_JOIN_MITER; break;
  }
  cairo_set_line_cap(cr, cap);
  cairo_set_line_join(cr, join);
  if(dashed) cairo_set_dash(cr, gc->dashes, gc->ndashes, gc->dash_offset);
  else cairo_set_dash(cr, NULL, 0, 0.0);
  /* mitre joins and square caps reach further than half the width */
  return width + 2;
}

static void stroke(cairo_t *cr, GC gc)
{
  if(gc->line_style == LineDoubleDash && gc->ndashes > 0) {
    /* the gaps are painted in the background colour */
    cairo_save(cr);
    cairo_set_dash(cr, NULL, 0, 0.0);
    set_rgb(cr, gc->background);
    cairo_stroke_preserve(cr);
    cairo_restore(cr);
  }
  cairo_stroke(cr);
}

struct box { int x1, y1, x2, y2; int any; };

/* Lines of width 1 and more, rectangle outlines and filled arcs are turned into pixels
 * by the X server's own code: see xs_mi.h, included at the end of this file. */
static int xs_mi_lines(xs_drawable *dst, GC gc, int mode, int npoints, const XPoint *points);
static int xs_mi_segments(xs_drawable *dst, GC gc, const XSegment *segments, int nsegments);
static int xs_mi_rectangles(xs_drawable *dst, GC gc, const XRectangle *rects, int nrects);
static int xs_mi_fill_arcs(xs_drawable *dst, GC gc, const XArc *arcs, int narcs);
static int xs_mi_arcs(xs_drawable *dst, GC gc, const XArc *arcs, int narcs);
static int xs_mi_fill_polygon(xs_drawable *dst, GC gc, int shape, int mode, int npoints,
    const XPoint *points);
static void xs_thin_axis_segments(xs_drawable *dst, GC gc, const XSegment *segments,
    int nsegments);

static void box_add(struct box *b, int x, int y)
{
  if(!b->any) {
    b->x1 = b->x2 = x;
    b->y1 = b->y2 = y;
    b->any = 1;
  } else {
    if(x < b->x1) b->x1 = x;
    if(x > b->x2) b->x2 = x;
    if(y < b->y1) b->y1 = y;
    if(y > b->y2) b->y2 = y;
  }
}

int XDrawSegments(Display *dpy, Drawable d, GC gc, XSegment *segments, int nsegments)
{
  xs_drawable *dst = xs_lookup(d);
  cairo_t *cr;
  struct box b = {0, 0, 0, 0, 0};
  int i, reach;
  (void)dpy;
  if(gc && gc->line_width >= 1) return xs_mi_segments(dst, gc, segments, nsegments);
  /* Width 0 ("thin"): xschem uses it only for the grid's axes and a grid drawn as
   * lines, all horizontal or vertical. Those are done pixel by pixel; slanted thin
   * lines go through cairo. */
  if(nsegments <= 0 || !dst || !gc) return 0;
  xs_thin_axis_segments(dst, gc, segments, nsegments);
  if(!(cr = begin_draw(dst, gc))) return 0;
  reach = set_stroke(cr, gc);
  for(i = 0; i < nsegments; ++i) {
    if(segments[i].x1 == segments[i].x2 || segments[i].y1 == segments[i].y2) continue;
    /* X coordinates name pixel centres */
    cairo_move_to(cr, segments[i].x1 + 0.5, segments[i].y1 + 0.5);
    cairo_line_to(cr, segments[i].x2 + 0.5, segments[i].y2 + 0.5);
    box_add(&b, segments[i].x1, segments[i].y1);
    box_add(&b, segments[i].x2, segments[i].y2);
    /* One at a time: stroked together, cairo fills the gap between two parallel
     * lines that are two pixels apart. */
    stroke(cr, gc);
  }
  if(b.any) {
    end_draw(dst, gc, cr, b.x1 - reach, b.y1 - reach, b.x2 + reach + 1, b.y2 + reach + 1);
  } else {
    cairo_restore(cr);
  }
  return 1;
}

int XDrawLine(Display *dpy, Drawable d, GC gc, int x1, int y1, int x2, int y2)
{
  XSegment segment;
  segment.x1 = (short)x1;
  segment.y1 = (short)y1;
  segment.x2 = (short)x2;
  segment.y2 = (short)y2;
  return XDrawSegments(dpy, d, gc, &segment, 1);
}

int XDrawLines(Display *dpy, Drawable d, GC gc, XPoint *points, int npoints, int mode)
{
  xs_drawable *dst = xs_lookup(d);
  cairo_t *cr;
  struct box b = {0, 0, 0, 0, 0};
  int i, reach, x = 0, y = 0;
  (void)dpy;
  if(gc && gc->line_width >= 1) return xs_mi_lines(dst, gc, mode, npoints, points);
  if(npoints <= 0 || !(cr = begin_draw(dst, gc))) return 0;
  reach = set_stroke(cr, gc);
  for(i = 0; i < npoints; ++i) {
    if(mode == CoordModePrevious && i > 0) {
      x += points[i].x;
      y += points[i].y;
    } else {
      x = points[i].x;
      y = points[i].y;
    }
    if(i == 0) cairo_move_to(cr, x + 0.5, y + 0.5);
    else cairo_line_to(cr, x + 0.5, y + 0.5);
    box_add(&b, x, y);
  }
  /* a closed outline gets a join instead of two caps */
  if(npoints > 2 && mode != CoordModePrevious && points[0].x == points[npoints - 1].x &&
     points[0].y == points[npoints - 1].y) cairo_close_path(cr);
  stroke(cr, gc);
  end_draw(dst, gc, cr, b.x1 - reach, b.y1 - reach, b.x2 + reach + 1, b.y2 + reach + 1);
  return 1;
}

/* The grid: many single pixels. Written straight into the image. */
int XDrawPoints(Display *dpy, Drawable d, GC gc, XPoint *points, int npoints, int mode)
{
  xs_drawable *dst = xs_lookup(d);
  unsigned char *data;
  int stride, i, x = 0, y = 0;
  int cx1 = 0, cy1 = 0, cx2, cy2;
  struct box b = {0, 0, 0, 0, 0};
  uint32_t pixel;
  (void)dpy;
  if(!dst || !dst->surface || !gc || npoints <= 0) return 0;
  if(gc->function != GXcopy) return 0;
  cx2 = dst->width;
  cy2 = dst->height;
  if(gc->has_clip) {
    int bx1 = 0, by1 = 0, bx2 = 0, by2 = 0;
    if(!clip_box(gc, &bx1, &by1, &bx2, &by2)) return 0;
    /* xschem clips to one rectangle; with several, their bounding box is close enough
     * for grid dots */
    if(bx1 > cx1) cx1 = bx1;
    if(by1 > cy1) cy1 = by1;
    if(bx2 < cx2) cx2 = bx2;
    if(by2 < cy2) cy2 = by2;
  }
  pixel = 0xff000000u | (uint32_t)(gc->foreground & 0xffffff);
  cairo_surface_flush(dst->surface);
  data = cairo_image_surface_get_data(dst->surface);
  stride = cairo_image_surface_get_stride(dst->surface);
  for(i = 0; i < npoints; ++i) {
    if(mode == CoordModePrevious && i > 0) {
      x += points[i].x;
      y += points[i].y;
    } else {
      x = points[i].x;
      y = points[i].y;
    }
    if(x < cx1 || y < cy1 || x >= cx2 || y >= cy2) continue;
    ((uint32_t *)(void *)(data + (size_t)y * (size_t)stride))[x] = pixel;
    box_add(&b, x, y);
  }
  if(b.any) {
    cairo_surface_mark_dirty_rectangle(dst->surface, b.x1, b.y1, b.x2 - b.x1 + 1,
                                       b.y2 - b.y1 + 1);
    xs_damage(dst, b.x1, b.y1, b.x2 - b.x1 + 1, b.y2 - b.y1 + 1);
  }
  return 1;
}

int XDrawRectangles(Display *dpy, Drawable d, GC gc, XRectangle *rectangles, int n)
{
  xs_drawable *dst = xs_lookup(d);
  cairo_t *cr;
  struct box b = {0, 0, 0, 0, 0};
  int i, reach;
  (void)dpy;
  if(gc && gc->line_width >= 1) return xs_mi_rectangles(dst, gc, rectangles, n);
  if(n <= 0 || !(cr = begin_draw(dst, gc))) return 0;
  reach = set_stroke(cr, gc);
  for(i = 0; i < n; ++i) {
    /* the outline passes through the pixels x .. x + width, y .. y + height */
    cairo_rectangle(cr, rectangles[i].x + 0.5, rectangles[i].y + 0.5, rectangles[i].width,
                    rectangles[i].height);
    box_add(&b, rectangles[i].x, rectangles[i].y);
    box_add(&b, rectangles[i].x + rectangles[i].width, rectangles[i].y + rectangles[i].height);
  }
  stroke(cr, gc);
  end_draw(dst, gc, cr, b.x1 - reach, b.y1 - reach, b.x2 + reach + 1, b.y2 + reach + 1);
  return 1;
}

int XDrawRectangle(Display *dpy, Drawable d, GC gc, int x, int y, unsigned int width,
    unsigned int height)
{
  XRectangle rectangle;
  rectangle.x = (short)x;
  rectangle.y = (short)y;
  rectangle.width = (unsigned short)width;
  rectangle.height = (unsigned short)height;
  return XDrawRectangles(dpy, d, gc, &rectangle, 1);
}

int XFillRectangles(Display *dpy, Drawable d, GC gc, XRectangle *rectangles, int n)
{
  xs_drawable *dst = xs_lookup(d);
  cairo_t *cr;
  struct box b = {0, 0, 0, 0, 0};
  int i;
  (void)dpy;
  if(n <= 0 || !(cr = begin_draw(dst, gc))) return 0;
  for(i = 0; i < n; ++i) {
    cairo_rectangle(cr, rectangles[i].x, rectangles[i].y, rectangles[i].width,
                    rectangles[i].height);
    box_add(&b, rectangles[i].x, rectangles[i].y);
    box_add(&b, rectangles[i].x + rectangles[i].width, rectangles[i].y + rectangles[i].height);
  }
  cairo_fill(cr);
  end_draw(dst, gc, cr, b.x1, b.y1, b.x2, b.y2);
  return 1;
}

int XFillRectangle(Display *dpy, Drawable d, GC gc, int x, int y, unsigned int width,
    unsigned int height)
{
  XRectangle rectangle;
  rectangle.x = (short)x;
  rectangle.y = (short)y;
  rectangle.width = (unsigned short)width;
  rectangle.height = (unsigned short)height;
  return XFillRectangles(dpy, d, gc, &rectangle, 1);
}

/* Add an X arc to the path: angles in 1/64 degree, counter-clockwise from 3 o'clock,
 * inside the box x, y, width, height. `offset` is 0.5 for outlines (pixel centres). */
static void arc_path(cairo_t *cr, const XArc *arc, double offset, int pie)
{
  double start = -arc->angle1 * M_PI / (180.0 * 64.0);
  double extent = arc->angle2;
  double end;
  int full;
  if(arc->width == 0 || arc->height == 0) return;
  if(extent > 360 * 64) extent = 360 * 64;
  if(extent < -360 * 64) extent = -360 * 64;
  full = extent >= 360 * 64 || extent <= -360 * 64;
  end = start - extent * M_PI / (180.0 * 64.0);
  cairo_new_sub_path(cr);
  cairo_save(cr);
  cairo_translate(cr, arc->x + arc->width / 2.0 + offset, arc->y + arc->height / 2.0 + offset);
  cairo_scale(cr, arc->width / 2.0, arc->height / 2.0);
  if(pie && !full) cairo_move_to(cr, 0.0, 0.0);
  /* y grows downwards: X's counter-clockwise is cairo's negative direction */
  if(extent >= 0) cairo_arc_negative(cr, 0.0, 0.0, 1.0, start, end);
  else cairo_arc(cr, 0.0, 0.0, 1.0, start, end);
  if(pie || full) cairo_close_path(cr);
  /* back to pixel units before stroking, so the line width is not scaled */
  cairo_restore(cr);
}

int XDrawArcs(Display *dpy, Drawable d, GC gc, XArc *arcs, int narcs)
{
  xs_drawable *dst = xs_lookup(d);
  cairo_t *cr;
  struct box b = {0, 0, 0, 0, 0};
  int i, reach;
  (void)dpy;
  /* The X server code handles the raster operations xschem draws arcs with. */
  if(gc && gc->line_width >= 1 &&
     (gc->function == GXcopy || gc->function == GXclear || gc->function == GXset)) {
    return xs_mi_arcs(dst, gc, arcs, narcs);
  }
  if(narcs <= 0 || !(cr = begin_draw(dst, gc))) return 0;
  reach = set_stroke(cr, gc);
  for(i = 0; i < narcs; ++i) {
    arc_path(cr, &arcs[i], 0.5, 0);
    box_add(&b, arcs[i].x, arcs[i].y);
    box_add(&b, arcs[i].x + arcs[i].width, arcs[i].y + arcs[i].height);
  }
  stroke(cr, gc);
  end_draw(dst, gc, cr, b.x1 - reach, b.y1 - reach, b.x2 + reach + 1, b.y2 + reach + 1);
  return 1;
}

int XDrawArc(Display *dpy, Drawable d, GC gc, int x, int y, unsigned int width,
    unsigned int height, int angle1, int angle2)
{
  XArc arc;
  arc.x = (short)x;
  arc.y = (short)y;
  arc.width = (unsigned short)width;
  arc.height = (unsigned short)height;
  arc.angle1 = (short)angle1;
  arc.angle2 = (short)angle2;
  return XDrawArcs(dpy, d, gc, &arc, 1);
}

int XFillArcs(Display *dpy, Drawable d, GC gc, XArc *arcs, int narcs)
{
  (void)dpy;
  return xs_mi_fill_arcs(xs_lookup(d), gc, arcs, narcs);
}

int XFillArc(Display *dpy, Drawable d, GC gc, int x, int y, unsigned int width,
    unsigned int height, int angle1, int angle2)
{
  XArc arc;
  arc.x = (short)x;
  arc.y = (short)y;
  arc.width = (unsigned short)width;
  arc.height = (unsigned short)height;
  arc.angle1 = (short)angle1;
  arc.angle2 = (short)angle2;
  return XFillArcs(dpy, d, gc, &arc, 1);
}

int XFillPolygon(Display *dpy, Drawable d, GC gc, XPoint *points, int npoints, int shape,
    int mode)
{
  (void)dpy;
  return xs_mi_fill_polygon(xs_lookup(d), gc, shape, mode, npoints, points);
}

/* Copies pixels as they are. xschem uses this to bring its off-screen picture to the
 * window, whole or in parts (to erase what it drew on top). */
int XCopyArea(Display *dpy, Drawable src, Drawable dest, GC gc, int src_x, int src_y,
    unsigned int width, unsigned int height, int dest_x, int dest_y)
{
  xs_drawable *from = xs_lookup(src), *to = xs_lookup(dest);
  cairo_t *cr;
  (void)dpy;
  if(!from || !from->surface || !to || !to->surface || !gc) return 0;
  if(gc->function == GXnoop || (gc->has_clip && gc->nclip == 0)) return 0;
  /* sizes computed from coordinates in unsigned arithmetic can wrap around */
  if(width > 65535 || height > 65535) return 0;
  cr = context_of(to);
  cairo_save(cr);
  apply_clip(cr, gc);
  cairo_set_source_surface(cr, from->surface, dest_x - src_x, dest_y - src_y);
  cairo_pattern_set_filter(cairo_get_source(cr), CAIRO_FILTER_NEAREST);
  cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
  cairo_rectangle(cr, dest_x, dest_y, width, height);
  cairo_fill(cr);
  end_draw(to, gc, cr, dest_x, dest_y, dest_x + (int)width, dest_y + (int)height);
  return 1;
}

/* ---- cairo surfaces on our drawables --------------------------------------------------- */
/* xschem draws text and images with cairo, on surfaces it asks for with
 * cairo_xlib_surface_create(). Ours are the very image the X calls draw on.
 *
 * What xschem draws that way never passes through the functions above, so for a
 * window the surface handed out is an observer of the image: any cairo drawing on it
 * marks the whole window as changed. (xschem normally draws to its pixmap and copies;
 * cairo drawing straight on the window is the exception.) */

static cairo_user_data_key_t image_key;

static void window_touched(cairo_surface_t *observer, cairo_surface_t *target, void *data)
{
  xs_drawable *win = xs_lookup((XID)(uintptr_t)data);
  (void)observer;
  /* the window may have been resized or destroyed since: then this is an old image */
  if(win && win->surface == target) xs_damage(win, 0, 0, win->width, win->height);
}

cairo_surface_t *cairo_xlib_surface_create(Display *dpy, Drawable drawable, Visual *visual,
    int width, int height)
{
  xs_drawable *d = xs_lookup(drawable);
  cairo_surface_t *surface;
  void *id = (void *)(uintptr_t)drawable;
  (void)dpy; (void)visual;
  if(!d || !d->surface) {
    /* never NULL: callers go on to use it */
    return cairo_image_surface_create(CAIRO_FORMAT_ARGB32, width < 1 ? 1 : width,
                                      height < 1 ? 1 : height);
  }
  if(d->kind != XS_WINDOW) return cairo_surface_reference(d->surface);
  surface = cairo_surface_create_observer(d->surface, CAIRO_SURFACE_OBSERVER_NORMAL);
  cairo_surface_observer_add_paint_callback(surface, window_touched, id);
  cairo_surface_observer_add_mask_callback(surface, window_touched, id);
  cairo_surface_observer_add_fill_callback(surface, window_touched, id);
  cairo_surface_observer_add_stroke_callback(surface, window_touched, id);
  cairo_surface_observer_add_glyphs_callback(surface, window_touched, id);
  cairo_surface_set_user_data(surface, &image_key, d->surface, NULL);
  return surface;
}

static cairo_surface_t *image_of(cairo_surface_t *surface)
{
  cairo_surface_t *image = cairo_surface_get_user_data(surface, &image_key);
  return image ? image : surface;
}

int cairo_xlib_surface_get_width(cairo_surface_t *surface)
{
  return cairo_image_surface_get_width(image_of(surface));
}

int cairo_xlib_surface_get_height(cairo_surface_t *surface)
{
  return cairo_image_surface_get_height(image_of(surface));
}

/* Text. When the boxes of the glyphs of a string overlap, cairo draws the string
 * through a mask. On an image surface that mask is as large as the font's metrics say
 * the text is; glyph images can reach a pixel beyond that (antialiased edges), and
 * those pixels are lost. An X server adds the whole glyph images into its mask. So for
 * such strings do the same here: add every glyph, one at a time, into a mask with
 * room to spare, then paint through it. Other strings are left to cairo, which draws
 * their glyphs one by one, as an X server does. */

/* What cairo works out about a string before drawing it
 * (_cairo_scaled_font_glyph_device_extents), rebuilt from the public metrics: whether
 * the glyph boxes overlap, and the box around them all in whole pixels. `each`, if
 * not NULL, gets the box of every glyph (x1, y1, x2, y2). */
static int glyph_boxes(cairo_t *cr, cairo_scaled_font_t *font, const cairo_glyph_t *glyphs,
    int n, int box[4], double *each)
{
  cairo_matrix_t ctm;
  double bx1 = 0.0, by1 = 0.0, bx2 = 0.0, by2 = 0.0;
  int i, k, any = 0, overlap = 0;

  cairo_get_matrix(cr, &ctm);
  for(i = 0; i < n; ++i) {
    cairo_text_extents_t e;
    cairo_glyph_t at_origin = glyphs[i];
    double px = glyphs[i].x, py = glyphs[i].y;
    double x1 = 0.0, y1 = 0.0, x2 = 0.0, y2 = 0.0;
    at_origin.x = at_origin.y = 0.0;
    cairo_scaled_font_glyph_extents(font, &at_origin, 1, &e);
    /* the inked box, turned with the text and rounded as cairo's fixed point does */
    for(k = 0; k < 4; ++k) {
      double cx = e.x_bearing + ((k & 1) ? e.width : 0.0);
      double cy = e.y_bearing + ((k & 2) ? e.height : 0.0);
      cairo_matrix_transform_distance(&ctm, &cx, &cy);
      if(k == 0 || cx < x1) x1 = cx;
      if(k == 0 || cx > x2) x2 = cx;
      if(k == 0 || cy < y1) y1 = cy;
      if(k == 0 || cy > y2) y2 = cy;
    }
    /* glyphs are placed on whole pixels */
    cairo_matrix_transform_point(&ctm, &px, &py);
    px = floor(px + 0.5);
    py = floor(py + 0.5);
    x1 = px + floor(x1 * 256.0 + 0.5) / 256.0;
    x2 = px + floor(x2 * 256.0 + 0.5) / 256.0;
    y1 = py + floor(y1 * 256.0 + 0.5) / 256.0;
    y2 = py + floor(y2 * 256.0 + 0.5) / 256.0;
    if(each) {
      each[4 * i] = x1;
      each[4 * i + 1] = y1;
      each[4 * i + 2] = x2;
      each[4 * i + 3] = y2;
    }
    if(any && x1 != x2 && y1 != y2 && x2 > bx1 && x1 < bx2 && y2 > by1 && y1 < by2) overlap = 1;
    if(!any || x1 < bx1) bx1 = x1;
    if(!any || x2 > bx2) bx2 = x2;
    if(!any || y1 < by1) by1 = y1;
    if(!any || y2 > by2) by2 = y2;
    any = 1;
  }
  box[0] = (int)floor(bx1);
  box[1] = (int)floor(by1);
  box[2] = (int)ceil(bx2);
  box[3] = (int)ceil(by2);
  return overlap;
}

#undef cairo_show_text
void xs_cairo_show_text(cairo_t *cr, const char *utf8)
{
  cairo_surface_type_t type = cairo_surface_get_type(cairo_get_target(cr));
  cairo_scaled_font_t *font = cairo_get_scaled_font(cr);
  cairo_glyph_t *glyphs = NULL;
  cairo_text_extents_t last;
  cairo_matrix_t matrix;
  cairo_surface_t *mask;
  cairo_t *m;
  double x = 0.0, y = 0.0, cx1, cy1, cx2, cy2, *each;
  int n = 0, i, box[4], seen[4], left, top, width, height;

  if(!utf8 || !utf8[0]) return;
  /* Our windows and pixmaps are images (a window through an observer). Leave any
   * surface that keeps text as text alone. */
  if(type == CAIRO_SURFACE_TYPE_PDF || type == CAIRO_SURFACE_TYPE_PS ||
     type == CAIRO_SURFACE_TYPE_SVG || type == CAIRO_SURFACE_TYPE_RECORDING) {
    cairo_show_text(cr, utf8);
    return;
  }
  if(cairo_has_current_point(cr)) cairo_get_current_point(cr, &x, &y);
  if(cairo_scaled_font_status(font) != CAIRO_STATUS_SUCCESS ||
     cairo_scaled_font_text_to_glyphs(font, x, y, utf8, -1, &glyphs, &n, NULL, NULL, NULL) !=
         CAIRO_STATUS_SUCCESS || n < 1) {
    if(glyphs) cairo_glyph_free(glyphs);
    cairo_show_text(cr, utf8);
    return;
  }
  /* where the text ends, for the current point afterwards */
  cairo_glyph_extents(cr, &glyphs[n - 1], 1, &last);
  x = glyphs[n - 1].x + last.x_advance;
  y = glyphs[n - 1].y + last.y_advance;

  each = n > 1 ? malloc((size_t)n * 4 * sizeof(*each)) : NULL;
  if(!each || !glyph_boxes(cr, font, glyphs, n, box, each)) {
    cairo_show_glyphs(cr, glyphs, n);
  } else {
    /* The part of the text's box that is inside the surface and the clip. */
    cairo_get_matrix(cr, &matrix);
    cairo_save(cr);
    cairo_identity_matrix(cr);
    cairo_clip_extents(cr, &cx1, &cy1, &cx2, &cy2);
    seen[0] = box[0] > (int)floor(cx1) ? box[0] : (int)floor(cx1);
    seen[1] = box[1] > (int)floor(cy1) ? box[1] : (int)floor(cy1);
    seen[2] = box[2] < (int)ceil(cx2) ? box[2] : (int)ceil(cx2);
    seen[3] = box[3] < (int)ceil(cy2) ? box[3] : (int)ceil(cy2);
    if(seen[2] > seen[0] && seen[3] > seen[1]) {
      /* Text that is cut by the surface or the clip is confined to its box (cairo
       * sets a clip then); otherwise the glyph images are painted whole. */
      int cut = box[2] - box[0] > seen[2] - seen[0] || box[3] - box[1] > seen[3] - seen[1];
      /* The mask holds, whole, every glyph that shows: a glyph cut by the edge of
       * the mask would lose its edge pixels again. */
      int mx1 = seen[0], my1 = seen[1], mx2 = seen[2], my2 = seen[3];
      for(i = 0; i < n; ++i) {
        const double *g = &each[4 * i];
        if(g[2] <= seen[0] - 2 || g[0] >= seen[2] + 2 || g[3] <= seen[1] - 2 ||
           g[1] >= seen[3] + 2) continue;
        if(g[0] < mx1) mx1 = (int)floor(g[0]);
        if(g[1] < my1) my1 = (int)floor(g[1]);
        if(g[2] > mx2) mx2 = (int)ceil(g[2]);
        if(g[3] > my2) my2 = (int)ceil(g[3]);
      }
      left = mx1 - 2;
      top = my1 - 2;
      width = mx2 - mx1 + 4;
      height = my2 - my1 + 4;
      mask = cairo_image_surface_create(CAIRO_FORMAT_A8, width, height);
      m = cairo_create(mask);
      matrix.x0 -= left;
      matrix.y0 -= top;
      cairo_set_matrix(m, &matrix);
      cairo_set_scaled_font(m, font);
      cairo_set_operator(m, CAIRO_OPERATOR_ADD);
      for(i = 0; i < n; ++i) {
        const double *g = &each[4 * i];
        if(g[2] <= seen[0] - 2 || g[0] >= seen[2] + 2 || g[3] <= seen[1] - 2 ||
           g[1] >= seen[3] + 2) continue;
        cairo_show_glyphs(m, &glyphs[i], 1);
      }
      cairo_destroy(m);
      if(cut) {
        cairo_rectangle(cr, seen[0], seen[1], seen[2] - seen[0], seen[3] - seen[1]);
        cairo_clip(cr);
      }
      cairo_mask_surface(cr, mask, left, top);
      cairo_surface_destroy(mask);
    }
    cairo_restore(cr);
  }
  free(each);
  cairo_glyph_free(glyphs);
  cairo_move_to(cr, x, y);
}

/* xschem makes new surfaces after a resize instead of calling this. */
void cairo_xlib_surface_set_size(cairo_surface_t *surface, int width, int height)
{
  (void)surface; (void)width; (void)height;
  UNSUPPORTED();
}

/* ---- Tk ----------------------------------------------------------------------------------- */
/* The Tk commands are a stand-in written in Tcl (shim/tcl). Here are the few C entry
 * points xschem uses: a Tk window is its path plus the X window behind it. */

struct xs_tkwin {
  char *path;
  Window window;
  struct xs_tkwin *next;
};

static struct xs_tkwin *tkwins = NULL;

/* Evaluate a script whose result is a number, leaving the interpreter's result as it
 * was. Returns `fallback` on error. */
static int tcl_int(const char *script, int fallback)
{
  Tcl_InterpState saved;
  int value = fallback;
  if(!the_interp) return fallback;
  saved = Tcl_SaveInterpState(the_interp, TCL_OK);
  if(Tcl_Eval(the_interp, script) == TCL_OK) {
    if(Tcl_GetIntFromObj(NULL, Tcl_GetObjResult(the_interp), &value) != TCL_OK) value = fallback;
  }
  Tcl_RestoreInterpState(the_interp, saved);
  return value;
}

static int tcl_window_query(const char *what, const char *path, int fallback)
{
  Tcl_DString script;
  int value;
  Tcl_DStringInit(&script);
  Tcl_DStringAppendElement(&script, "winfo");
  Tcl_DStringAppendElement(&script, what);
  Tcl_DStringAppendElement(&script, path);
  value = tcl_int(Tcl_DStringValue(&script), fallback);
  Tcl_DStringFree(&script);
  return value;
}

static struct xs_tkwin *find_tkwin(const char *path)
{
  struct xs_tkwin *w;
  for(w = tkwins; w; w = w->next) if(!strcmp(w->path, path)) return w;
  return NULL;
}

static void forget_tkwin(struct xs_tkwin *gone)
{
  struct xs_tkwin **link;
  for(link = &tkwins; *link; link = &(*link)->next) {
    if(*link == gone) {
      *link = gone->next;
      if(gone->window != None) free_drawable(gone->window);
      free(gone->path);
      free(gone);
      return;
    }
  }
}

static int source_file(Tcl_Interp *interp, const char *dir, const char *name)
{
  char path[4096];
  snprintf(path, sizeof(path), "%s/cdept/%s", dir, name);
  if(Tcl_EvalFile(interp, path) != TCL_OK) {
    const char *info = Tcl_GetVar(interp, "errorInfo", TCL_GLOBAL_ONLY);
    fprintf(stderr, "cdept-xschem: %s: %s\n", path, info ? info : Tcl_GetStringResult(interp));
    return TCL_ERROR;
  }
  return TCL_OK;
}

/* Called by xschem's start-up in place of Tk's own: loads the Tcl stand-in for the Tk
 * commands. See shim/tcl/tkmin.tcl for what that file has to provide. */
int Tk_Init(Tcl_Interp *interp)
{
  const char *share = getenv("XSCHEM_SHAREDIR");
  const char *choice = getenv("CDEPT_XSCHEM_TK");
  char path[4096];
  int use_faketk = 0;

  the_interp = interp;
  /* xschem.tcl builds its widgets only if DISPLAY names something. The value is not
   * used by us; a real one is left alone for the programs xschem starts. */
  Tcl_Eval(interp, "if {![info exists ::env(DISPLAY)] || $::env(DISPLAY) eq {}} "
                   "{set ::env(DISPLAY) :0}");
  /* What the Tcl side may ask about the drawing window (kept up to date by
   * display.c). */
  Tcl_Eval(interp, "namespace eval ::cdept { variable drw; "
                   "array set drw {width 1 height 1 mapped 0 pointerx 0 pointery 0} }");
  xs_register_display_commands(interp);
  if(!share) {
    fprintf(stderr, "cdept-xschem: XSCHEM_SHAREDIR is not set, cannot load the Tk stand-in\n");
    return TCL_ERROR;
  }
  if(!choice || !choice[0] || !strcmp(choice, "faketk")) {
    FILE *f;
    snprintf(path, sizeof(path), "%s/cdept/faketk.tcl", share);
    f = fopen(path, "r");
    if(f) {
      fclose(f);
      use_faketk = 1;
    }
  }
  return source_file(interp, share, use_faketk ? "faketk.tcl" : "tkmin.tcl");
}

void Tk_MainLoop(void) { UNSUPPORTED(); }

void Tk_Main(int argc, char **argv, Tcl_AppInitProc *init)
{
  (void)argc; (void)argv; (void)init;
  UNSUPPORTED();
}

Tk_Window Tk_NameToWindow(Tcl_Interp *interp, const char *path, Tk_Window main)
{
  struct xs_tkwin *w = find_tkwin(path);
  (void)interp; (void)main;
  /* "." always exists; anything else exists if the Tcl side says so (as with Tk, a
   * window that was destroyed yields NULL). */
  if(strcmp(path, ".") && !tcl_window_query("exists", path, 0)) {
    if(w) forget_tkwin(w);
    return NULL;
  }
  if(!w) {
    w = calloc(1, sizeof(*w));
    if(!w) return NULL;
    w->path = malloc(strlen(path) + 1);
    if(!w->path) {
      free(w);
      return NULL;
    }
    strcpy(w->path, path);
    w->next = tkwins;
    tkwins = w;
  }
  return w;
}

Tk_Window Tk_MainWindow(Tcl_Interp *interp)
{
  if(!the_interp) return NULL;
  return Tk_NameToWindow(interp, ".", NULL);
}

void Tk_MakeWindowExist(Tk_Window win)
{
  xs_drawable *d;
  int width = 1, height = 1;
  if(!win || win->window != None) return;
  /* The drawing window gets its size from cdx_display_resize(); any other window
   * (a preview) has the size the Tcl side gives it. */
  if(strcmp(win->path, ".drw") && strcmp(win->path, ".")) {
    width = tcl_window_query("width", win->path, 1);
    height = tcl_window_query("height", win->path, 1);
  }
  d = new_drawable(XS_WINDOW, width, height);
  if(d) win->window = d->id;
}

void Tk_DestroyWindow(Tk_Window win)
{
  if(win) forget_tkwin(win);
}

Window Tk_WindowId(Tk_Window win)
{
  if(!win) return None;
  Tk_MakeWindowExist(win);
  return win->window;
}

xs_drawable *xs_window_of_path(const char *path)
{
  struct xs_tkwin *w = find_tkwin(path);
  if(!w || w->window == None) return NULL;
  return xs_lookup(w->window);
}

Display *Tk_Display(Tk_Window win) { (void)win; return &the_display; }
Visual *Tk_Visual(Tk_Window win) { (void)win; return &the_visual; }
Colormap Tk_Colormap(Tk_Window win) { (void)win; return 1; }
int Tk_Depth(Tk_Window win) { (void)win; return 24; }
int Tk_ScreenNumber(Tk_Window win) { (void)win; return 0; }

/* xschem asks for a backing store; our windows are nothing but a backing store. */
void Tk_ChangeWindowAttributes(Tk_Window win, unsigned long valuemask,
    XSetWindowAttributes *attributes)
{
  (void)win; (void)valuemask; (void)attributes;
}

XColor *Tk_GetColor(Tcl_Interp *interp, Tk_Window win, const char *name)
{
  static XColor color;
  unsigned long rgb = 0;
  (void)interp; (void)win;
  if(!parse_colour(name, &rgb)) return NULL;
  fill_xcolor(&color, rgb);
  return &color;
}

Pixmap Tk_GetPixmap(Display *dpy, Drawable d, int width, int height, int depth)
{
  return XCreatePixmap(dpy, d, (unsigned int)width, (unsigned int)height, (unsigned int)depth);
}

void Tk_FreePixmap(Display *dpy, Pixmap pixmap) { XFreePixmap(dpy, pixmap); }

/* ---- the X server's line and arc code ------------------------------------------------- */

#include "xs_mi.h"
