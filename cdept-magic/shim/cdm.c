/* cdm.c: Tk and X stand-ins for Magic's tclmagic.so, running inside the helper.
 *
 * tclmagic.so calls Tk only through the Tk stub table (tkStubsPtr), which it gets
 * from `package require Tk`: we provide that package with a table of our own
 * functions. It calls Xlib directly; the helper program exports the Xlib functions
 * Magic uses (it is linked with --export-dynamic), and symbols of the program come
 * before those of libX11 when the dynamic linker resolves tclmagic.so. The same goes
 * for cairo_xlib_surface_create(): a "window" or "pixmap" here is a cairo image
 * surface, and Magic's Cairo display driver (-d XR) draws into it unchanged.
 *
 * Only what Magic's Cairo driver and its Tcl start-up need is implemented; anything
 * else in the stub table aborts with a message naming the slot.
 *
 * Part of cdept-magic, GPL-2.0-or-later.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <elf.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/keysym.h>
#include <tcl.h>
#include <tk.h>
#include <cairo.h>

#include "cdm.h"

static Tcl_Interp *interp = NULL;
static int screen_w = 3840, screen_h = 2160;

/* ------------------------------------------------------------------------------ */
/* Fake display                                                                    */

typedef __typeof__(*((_XPrivDisplay)0)) FakeDisplay;
static FakeDisplay fake_display;
static Screen fake_screen;
static Visual fake_visual;
static Depth fake_depth;

#define DRAWABLE_BASE 0x100
#define MAX_DRAWABLES 512

typedef struct {
  int used;
  int is_window;
  int width, height;
  cairo_surface_t *surface;
} Drawable_;

static Drawable_ drawables[MAX_DRAWABLES];

static Drawable_ *drawable_of(Drawable id)
{
  long i = (long)id - DRAWABLE_BASE;
  if(i < 0 || i >= MAX_DRAWABLES || !drawables[i].used) return NULL;
  return &drawables[i];
}

static Drawable new_drawable(int width, int height, int is_window)
{
  int i;
  if(width < 1) width = 1;
  if(height < 1) height = 1;
  for(i = 0; i < MAX_DRAWABLES; ++i) {
    if(!drawables[i].used) {
      drawables[i].used = 1;
      drawables[i].is_window = is_window;
      drawables[i].width = width;
      drawables[i].height = height;
      drawables[i].surface = cairo_image_surface_create(CAIRO_FORMAT_RGB24, width, height);
      return (Drawable)(DRAWABLE_BASE + i);
    }
  }
  fprintf(stderr, "cdept-magic: too many pixmaps\n");
  return 0;
}

static void free_drawable(Drawable id)
{
  Drawable_ *d = drawable_of(id);
  if(!d) return;
  cairo_surface_destroy(d->surface);
  memset(d, 0, sizeof(*d));
}

static void init_display(void)
{
  fake_visual.visualid = 0x21;
  fake_visual.class = TrueColor;
  fake_visual.red_mask = 0xff0000;
  fake_visual.green_mask = 0x00ff00;
  fake_visual.blue_mask = 0x0000ff;
  fake_visual.bits_per_rgb = 8;
  fake_visual.map_entries = 256;
  fake_depth.depth = 24;
  fake_depth.nvisuals = 1;
  fake_depth.visuals = &fake_visual;
  fake_screen.display = (Display *)&fake_display;
  fake_screen.root = 1;
  fake_screen.width = screen_w;
  fake_screen.height = screen_h;
  fake_screen.mwidth = screen_w * 254 / 960; /* 96 dpi */
  fake_screen.mheight = screen_h * 254 / 960;
  fake_screen.ndepths = 1;
  fake_screen.depths = &fake_depth;
  fake_screen.root_depth = 24;
  fake_screen.root_visual = &fake_visual;
  fake_screen.cmap = 1;
  fake_screen.white_pixel = 0xffffff;
  fake_screen.black_pixel = 0;
  fake_display.nscreens = 1;
  fake_display.screens = &fake_screen;
  fake_display.default_screen = 0;
  fake_display.fd = -1;
  fake_display.display_name = "cdept-magic";
}

/* ------------------------------------------------------------------------------ */
/* Xlib: what tclmagic.so calls. Drawing calls of Magic's plain X11 driver (-d X11)  */
/* are accepted and do nothing: only the Cairo driver is supported.                */

static unsigned long pending_keysym = 0;
static int pointer_x = 0, pointer_y = 0;
static char title[512] = "";

cairo_surface_t *cairo_xlib_surface_create(Display *dpy, Drawable d, Visual *v, int w, int h)
{
  Drawable_ *dr = drawable_of(d);
  (void)dpy; (void)v; (void)w; (void)h;
  if(!dr) {
    /* Magic asks for a surface of the main window "." (id 0) to draw Tk images
     * off-screen. Give it a scratch surface. */
    return cairo_image_surface_create(CAIRO_FORMAT_RGB24, w > 0 ? w : 1, h > 0 ? h : 1);
  }
  return cairo_surface_reference(dr->surface);
}

/* Windows are as large as the screen from the start: Magic keeps its cairo context, so
 * a window cannot get a new surface when it is resized. */
void cairo_xlib_surface_set_size(cairo_surface_t *s, int w, int h)
{
  (void)s; (void)w; (void)h;
}

Pixmap XCreatePixmap(Display *dpy, Drawable d, unsigned int w, unsigned int h, unsigned int depth)
{
  (void)dpy; (void)d; (void)depth;
  return new_drawable((int)w, (int)h, 0);
}

int XFreePixmap(Display *dpy, Pixmap p)
{
  (void)dpy;
  free_drawable(p);
  return 1;
}

int XCopyArea(Display *dpy, Drawable src, Drawable dst, GC gc, int sx, int sy,
              unsigned int w, unsigned int h, int dx, int dy)
{
  Drawable_ *s = drawable_of(src), *t = drawable_of(dst);
  cairo_t *cr;
  (void)dpy; (void)gc;
  if(!s || !t) return 0;
  cr = cairo_create(t->surface);
  cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
  cairo_set_source_surface(cr, s->surface, dx - sx, dy - sy);
  cairo_rectangle(cr, dx, dy, w, h);
  cairo_fill(cr);
  cairo_destroy(cr);
  return 1;
}

GC XCreateGC(Display *dpy, Drawable d, unsigned long mask, XGCValues *values)
{
  (void)dpy; (void)d; (void)mask; (void)values;
  return (GC)calloc(1, 256);
}

int XFreeGC(Display *dpy, GC gc)
{
  (void)dpy;
  free(gc);
  return 1;
}

XVisualInfo *XGetVisualInfo(Display *dpy, long mask, XVisualInfo *template, int *count)
{
  XVisualInfo *info = calloc(1, sizeof(XVisualInfo));
  (void)dpy; (void)mask; (void)template;
  info->visual = &fake_visual;
  info->visualid = fake_visual.visualid;
  info->screen = 0;
  info->depth = 24;
  info->class = TrueColor;
  info->red_mask = fake_visual.red_mask;
  info->green_mask = fake_visual.green_mask;
  info->blue_mask = fake_visual.blue_mask;
  info->colormap_size = 256;
  info->bits_per_rgb = 8;
  *count = 1;
  return info;
}

Colormap XCreateColormap(Display *dpy, Window w, Visual *v, int alloc)
{
  (void)dpy; (void)w; (void)v; (void)alloc;
  return 1;
}

Colormap XDefaultColormap(Display *dpy, int screen) { (void)dpy; (void)screen; return 1; }
Visual *XDefaultVisual(Display *dpy, int screen) { (void)dpy; (void)screen; return &fake_visual; }

int XQueryColor(Display *dpy, Colormap cmap, XColor *c)
{
  (void)dpy; (void)cmap;
  c->red = ((c->pixel >> 16) & 0xff) * 257;
  c->green = ((c->pixel >> 8) & 0xff) * 257;
  c->blue = (c->pixel & 0xff) * 257;
  c->flags = DoRed | DoGreen | DoBlue;
  return 1;
}

int XQueryColors(Display *dpy, Colormap cmap, XColor *c, int n)
{
  int i;
  for(i = 0; i < n; ++i) XQueryColor(dpy, cmap, &c[i]);
  return 1;
}

Status XAllocColorCells(Display *dpy, Colormap cmap, Bool contig, unsigned long *planes,
                        unsigned int nplanes, unsigned long *pixels, unsigned int npixels)
{
  (void)dpy; (void)cmap; (void)contig; (void)planes; (void)nplanes; (void)pixels; (void)npixels;
  return 0;
}

int XStoreColors(Display *dpy, Colormap cmap, XColor *c, int n)
{ (void)dpy; (void)cmap; (void)c; (void)n; return 1; }
int XInstallColormap(Display *dpy, Colormap cmap) { (void)dpy; (void)cmap; return 1; }
int XSetWindowColormap(Display *dpy, Window w, Colormap c) { (void)dpy; (void)w; (void)c; return 1; }

char *XGetDefault(Display *dpy, const char *program, const char *option)
{
  (void)dpy; (void)program; (void)option;
  return NULL;
}

Bool XQueryPointer(Display *dpy, Window w, Window *root, Window *child, int *rx, int *ry,
                   int *wx, int *wy, unsigned int *mask)
{
  (void)dpy; (void)w;
  if(root) *root = 1;
  if(child) *child = 0;
  if(rx) *rx = pointer_x;
  if(ry) *ry = pointer_y;
  if(wx) *wx = pointer_x;
  if(wy) *wy = pointer_y;
  if(mask) *mask = 0;
  return True;
}

/* Key events carry their key symbol in pending_keysym (set before they are handed
 * to Magic): there are no key codes here. */
int XLookupString(XKeyEvent *ev, char *buf, int nbytes, KeySym *keysym, XComposeStatus *status)
{
  unsigned long ks = pending_keysym;
  int ch = -1;
  (void)status;
  if(keysym) *keysym = ks;
  if(ks >= 0x20 && ks <= 0xff) ch = (int)ks;
  else switch(ks) {
    case XK_BackSpace: ch = 8; break;
    case XK_Tab: ch = 9; break;
    case XK_Return: case XK_KP_Enter: ch = 13; break;
    case XK_Escape: ch = 27; break;
    case XK_Delete: ch = 127; break;
    default:
      if(ks >= XK_KP_0 && ks <= XK_KP_9) ch = '0' + (int)(ks - XK_KP_0);
      break;
  }
  if(ch < 0 || nbytes < 1) return 0;
  if((ev->state & ControlMask) && ch >= '@' && ch < 0x7f) ch &= 0x1f;
  buf[0] = (char)ch;
  if(nbytes > 1) buf[1] = '\0';
  return 1;
}

Bool XCheckWindowEvent(Display *dpy, Window w, long mask, XEvent *ev)
{ (void)dpy; (void)w; (void)mask; (void)ev; return False; }
int XPutBackEvent(Display *dpy, XEvent *ev) { (void)dpy; (void)ev; return 0; }
int XSync(Display *dpy, Bool discard) { (void)dpy; (void)discard; return 1; }
int XFlush(Display *dpy) { (void)dpy; return 1; }
int XCloseDisplay(Display *dpy) { (void)dpy; return 0; }

int XStoreName(Display *dpy, Window w, const char *name)
{
  (void)dpy; (void)w;
  snprintf(title, sizeof(title), "%s", name ? name : "");
  return 1;
}

int XSetIconName(Display *dpy, Window w, const char *name) { (void)dpy; (void)w; (void)name; return 1; }
int XSetClassHint(Display *dpy, Window w, XClassHint *hint) { (void)dpy; (void)w; (void)hint; return 1; }

Status XGetGeometry(Display *dpy, Drawable d, Window *root, int *x, int *y, unsigned int *w,
                    unsigned int *h, unsigned int *border, unsigned int *depth)
{
  Drawable_ *dr = drawable_of(d);
  (void)dpy;
  if(root) *root = 1;
  if(x) *x = 0;
  if(y) *y = 0;
  if(w) *w = dr ? (unsigned)dr->width : 1;
  if(h) *h = dr ? (unsigned)dr->height : 1;
  if(border) *border = 0;
  if(depth) *depth = 24;
  return 1;
}

Status XGetWindowAttributes(Display *dpy, Window w, XWindowAttributes *a)
{
  unsigned int width, height;
  memset(a, 0, sizeof(*a));
  XGetGeometry(dpy, w, &a->root, &a->x, &a->y, &width, &height, NULL, NULL);
  a->width = (int)width;
  a->height = (int)height;
  a->depth = 24;
  a->visual = &fake_visual;
  a->colormap = 1;
  a->map_state = IsViewable;
  a->screen = &fake_screen;
  return 1;
}

Status XQueryTree(Display *dpy, Window w, Window *root, Window *parent, Window **children,
                  unsigned int *n)
{
  (void)dpy; (void)w;
  if(root) *root = 1;
  if(parent) *parent = 1;
  if(children) *children = NULL;
  if(n) *n = 0;
  return 1;
}

XImage *XGetImage(Display *dpy, Drawable d, int x, int y, unsigned int w, unsigned int h,
                  unsigned long planes, int format)
{
  (void)dpy; (void)d; (void)x; (void)y; (void)w; (void)h; (void)planes; (void)format;
  return NULL;
}

/* The X11 driver's drawing and GC calls: nothing to do. */
int XDrawPoint(Display *a, Drawable b, GC c, int d, int e) { (void)a; (void)b; (void)c; (void)d; (void)e; return 1; }
int XDrawSegments(Display *a, Drawable b, GC c, XSegment *d, int e) { (void)a; (void)b; (void)c; (void)d; (void)e; return 1; }
int XFillPolygon(Display *a, Drawable b, GC c, XPoint *d, int e, int f, int g) { (void)a; (void)b; (void)c; (void)d; (void)e; (void)f; (void)g; return 1; }
int XFillRectangle(Display *a, Drawable b, GC c, int d, int e, unsigned int f, unsigned int g) { (void)a; (void)b; (void)c; (void)d; (void)e; (void)f; (void)g; return 1; }
int XFillRectangles(Display *a, Drawable b, GC c, XRectangle *d, int e) { (void)a; (void)b; (void)c; (void)d; (void)e; return 1; }
int XSetClipMask(Display *a, GC b, Pixmap c) { (void)a; (void)b; (void)c; return 1; }
int XSetClipOrigin(Display *a, GC b, int c, int d) { (void)a; (void)b; (void)c; (void)d; return 1; }
int XSetClipRectangles(Display *a, GC b, int c, int d, XRectangle *e, int f, int g) { (void)a; (void)b; (void)c; (void)d; (void)e; (void)f; (void)g; return 1; }
int XSetDashes(Display *a, GC b, int c, const char *d, int e) { (void)a; (void)b; (void)c; (void)d; (void)e; return 1; }
int XSetFillStyle(Display *a, GC b, int c) { (void)a; (void)b; (void)c; return 1; }
int XSetFont(Display *a, GC b, Font c) { (void)a; (void)b; (void)c; return 1; }
int XSetForeground(Display *a, GC b, unsigned long c) { (void)a; (void)b; (void)c; return 1; }
int XSetFunction(Display *a, GC b, int c) { (void)a; (void)b; (void)c; return 1; }
int XSetLineAttributes(Display *a, GC b, unsigned int c, int d, int e, int f) { (void)a; (void)b; (void)c; (void)d; (void)e; (void)f; return 1; }
int XSetPlaneMask(Display *a, GC b, unsigned long c) { (void)a; (void)b; (void)c; return 1; }
int XSetStipple(Display *a, GC b, Pixmap c) { (void)a; (void)b; (void)c; return 1; }

/* ------------------------------------------------------------------------------ */
/* Tk windows                                                                      */

typedef struct Handler {
  unsigned long mask;
  Tk_EventProc *proc;
  ClientData data;
  struct Handler *next;
} Handler;

typedef struct CdmWin {
  Tk_FakeWin fake;   /* must come first: Tk's macros read it */
  char path[256];
  Handler *handlers;
  int destroyed;
  struct CdmWin *next;
} CdmWin;

static Tk_Uid t_GetUid(const char *s);
static CdmWin main_window;
static CdmWin *windows = NULL;
static CdmWin *drawing = NULL; /* the first layout window: the one CDePT shows */
static int want_w = 800, want_h = 600; /* the size CDePT shows it at */
static uint32_t *shadow = NULL;        /* pixels as last handed out */
static int shadow_w = 0, shadow_h = 0;
static int force_full = 1;
static int configured = 0; /* Magic has been told the size of the drawing window */

static void set_drw_vars(void)
{
  char buf[32];
  if(!interp) return;
  snprintf(buf, sizeof(buf), "%d", drawing ? Tk_Width((Tk_Window)drawing) : want_w);
  Tcl_SetVar2(interp, "::cdept::drw", "width", buf, TCL_GLOBAL_ONLY);
  snprintf(buf, sizeof(buf), "%d", drawing ? Tk_Height((Tk_Window)drawing) : want_h);
  Tcl_SetVar2(interp, "::cdept::drw", "height", buf, TCL_GLOBAL_ONLY);
  Tcl_SetVar2(interp, "::cdept::drw", "mapped", drawing ? "1" : "0", TCL_GLOBAL_ONLY);
  snprintf(buf, sizeof(buf), "%d", pointer_x);
  Tcl_SetVar2(interp, "::cdept::drw", "pointerx", buf, TCL_GLOBAL_ONLY);
  snprintf(buf, sizeof(buf), "%d", pointer_y);
  Tcl_SetVar2(interp, "::cdept::drw", "pointery", buf, TCL_GLOBAL_ONLY);
}

static void init_window(CdmWin *w, const char *path, CdmWin *parent, int toplevel)
{
  const char *name;
  memset(w, 0, sizeof(*w));
  snprintf(w->path, sizeof(w->path), "%s", path);
  name = strrchr(w->path, '.');
  name = (name && name[1]) ? name + 1 : w->path;
  w->fake.display = (Display *)&fake_display;
  w->fake.screenNum = 0;
  w->fake.visual = &fake_visual;
  w->fake.depth = 24;
  w->fake.window = 0;
  w->fake.parentPtr = (Tk_Window)parent;
  w->fake.pathName = w->path;
  w->fake.nameUid = t_GetUid(name);
  w->fake.classUid = t_GetUid(toplevel ? "Toplevel" : "Frame");
  w->fake.changes.width = 1;
  w->fake.changes.height = 1;
  w->fake.atts.colormap = 1;
  w->fake.flags = toplevel ? TK_TOP_LEVEL : 0;
}

static int tracing = -1;
#define TRACE(...) do { if(tracing < 0) tracing = getenv("CDEPT_MAGIC_TRACE") != NULL; \
  if(tracing) fprintf(stderr, "cdm: " __VA_ARGS__); } while(0)

static void dispatch(CdmWin *w, XEvent *ev, unsigned long mask)
{
  Handler *h, *next;
  if(!w || w->destroyed) return;
  TRACE("event %d to %s (mask %lx, handlers %p)\n", ev->type, w->path, mask, (void *)w->handlers);
  ev->xany.display = (Display *)&fake_display;
  ev->xany.window = w->fake.window;
  for(h = w->handlers; h; h = next) {
    next = h->next;
    if(h->mask & mask) h->proc(h->data, ev);
    if(w->destroyed) return;
  }
}

static void send_configure(CdmWin *w)
{
  XEvent ev;
  memset(&ev, 0, sizeof(ev));
  ev.type = ConfigureNotify;
  ev.xconfigure.x = 0;
  ev.xconfigure.y = 0;
  ev.xconfigure.width = Tk_Width((Tk_Window)w);
  ev.xconfigure.height = Tk_Height((Tk_Window)w);
  TRACE("configure %s %dx%d\n", w->path, ev.xconfigure.width, ev.xconfigure.height);
  dispatch(w, &ev, StructureNotifyMask);
  if(tracing > 0) { Tcl_Eval(interp, "magic::windowpositions"); TRACE("now %s\n", Tcl_GetStringResult(interp)); }
  memset(&ev, 0, sizeof(ev));
  ev.type = Expose;
  ev.xexpose.x = 0;
  ev.xexpose.y = 0;
  ev.xexpose.width = Tk_Width((Tk_Window)w);
  ev.xexpose.height = Tk_Height((Tk_Window)w);
  dispatch(w, &ev, ExposureMask);
}

/* Magic sends a command to the window under its "current point": keep that point
 * where the pointer is (Magic's GUI wrapper does the same on every motion). */
static void set_point(int x, int y)
{
  Tcl_Obj *cmd;
  if(!interp || !drawing || drawing->destroyed) return;
  cmd = Tcl_ObjPrintf("*bypass setpoint %d %d %s", x, y, drawing->path);
  Tcl_IncrRefCount(cmd);
  Tcl_EvalObjEx(interp, cmd, TCL_EVAL_GLOBAL);
  Tcl_DecrRefCount(cmd);
  Tcl_ResetResult(interp);
}

static void first_show(ClientData data)
{
  CdmWin *w = (CdmWin *)data;
  XEvent ev;
  if(w != drawing || w->destroyed) return;
  /* Magic registers its event handler only after mapping the window: until then
   * nobody listens, and this runs again later (cdept::setpoint) */
  if(!w->handlers) return;
  configured = 1;
  memset(&ev, 0, sizeof(ev));
  ev.type = VisibilityNotify;
  ev.xvisibility.state = VisibilityUnobscured;
  dispatch(w, &ev, VisibilityChangeMask);
  send_configure(w);
}

static void make_exist(CdmWin *w)
{
  if(w->fake.window || w == &main_window) return;
  /* every window is screen sized; see cairo_xlib_surface_set_size() */
  w->fake.window = new_drawable(screen_w, screen_h, 1);
  {
    Drawable_ *d = drawable_of(w->fake.window);
    if(d) {
      cairo_t *cr = cairo_create(d->surface);
      cairo_set_source_rgb(cr, 0.8, 0.8, 0.8);
      cairo_paint(cr);
      cairo_destroy(cr);
    }
  }
}

static Tk_Window t_MainWindow(Tcl_Interp *ip) { (void)ip; return (Tk_Window)&main_window; }

static Tk_Window t_CreateWindowFromPath(Tcl_Interp *ip, Tk_Window parent, const char *path,
                                        const char *screen)
{
  CdmWin *w = calloc(1, sizeof(CdmWin));
  (void)ip;
  init_window(w, path, (CdmWin *)parent, screen != NULL);
  TRACE("create %s screen=%s\n", path, screen ? screen : "(null)");
  w->next = windows;
  windows = w;
  if(!drawing && screen != NULL) {
    /* Magic's layout windows are top-level windows; the first one is shown */
    drawing = w;
    w->fake.changes.width = want_w;
    w->fake.changes.height = want_h;
    force_full = 1;
  }
  return (Tk_Window)w;
}

static Tk_Window t_NameToWindow(Tcl_Interp *ip, const char *path, Tk_Window ref)
{
  CdmWin *w;
  (void)ref;
  if(!strcmp(path, ".")) return (Tk_Window)&main_window;
  for(w = windows; w; w = w->next)
    if(!w->destroyed && !strcmp(w->path, path)) return (Tk_Window)w;
  if(ip) Tcl_SetObjResult(ip, Tcl_ObjPrintf("bad window path name \"%s\"", path));
  return NULL;
}

static void t_MakeWindowExist(Tk_Window tw) { make_exist((CdmWin *)tw); }

static void t_MapWindow(Tk_Window tw)
{
  CdmWin *w = (CdmWin *)tw;
  make_exist(w);
  if(w->fake.flags & TK_MAPPED) return;
  w->fake.flags |= TK_MAPPED;
  if(w == drawing) {
    set_drw_vars();
    Tcl_DoWhenIdle(first_show, w);
  }
}

static void t_UnmapWindow(Tk_Window tw) { ((CdmWin *)tw)->fake.flags &= ~TK_MAPPED; }

static void t_DestroyWindow(Tk_Window tw)
{
  CdmWin *w = (CdmWin *)tw;
  Handler *h, *next;
  XEvent ev;
  if(w == &main_window || w->destroyed) return;
  memset(&ev, 0, sizeof(ev));
  ev.type = DestroyNotify;
  dispatch(w, &ev, StructureNotifyMask);
  w->destroyed = 1;
  for(h = w->handlers; h; h = next) { next = h->next; free(h); }
  w->handlers = NULL;
  if(w->fake.window) free_drawable(w->fake.window);
  w->fake.window = 0;
  if(w == drawing) {
    drawing = NULL;
    set_drw_vars();
  }
  /* the structure stays allocated: Magic may still look at it */
}

static void t_GeometryRequest(Tk_Window tw, int width, int height)
{
  ((CdmWin *)tw)->fake.reqWidth = width;
  ((CdmWin *)tw)->fake.reqHeight = height;
}

static void t_MoveResizeWindow(Tk_Window tw, int x, int y, int width, int height)
{
  CdmWin *w = (CdmWin *)tw;
  w->fake.changes.x = x;
  w->fake.changes.y = y;
  if(w == drawing) return; /* its size is CDePT's to choose */
  w->fake.changes.width = width;
  w->fake.changes.height = height;
}

static void t_ResizeWindow(Tk_Window tw, int width, int height)
{
  t_MoveResizeWindow(tw, Tk_X(tw), Tk_Y(tw), width, height);
}

static void t_MoveWindow(Tk_Window tw, int x, int y)
{
  ((CdmWin *)tw)->fake.changes.x = x;
  ((CdmWin *)tw)->fake.changes.y = y;
}

static int t_SetWindowVisual(Tk_Window tw, Visual *v, int depth, Colormap c)
{ (void)tw; (void)v; (void)depth; (void)c; return 1; }
static void t_SetWindowColormap(Tk_Window tw, Colormap c) { (void)tw; (void)c; }
static void t_ChangeWindowAttributes(Tk_Window tw, unsigned long m, XSetWindowAttributes *a)
{ (void)tw; (void)m; (void)a; }
static int t_RestackWindow(Tk_Window tw, int ab, Tk_Window other)
{ (void)tw; (void)ab; (void)other; return TCL_OK; }
static void t_DefineCursor(Tk_Window tw, Tk_Cursor c) { (void)tw; (void)c; }

static void t_CreateEventHandler(Tk_Window tw, unsigned long mask, Tk_EventProc *proc,
                                 ClientData data)
{
  CdmWin *w = (CdmWin *)tw;
  Handler *h;
  if(tracing > 0) { Dl_info di; if(dladdr((void *)proc, &di)) TRACE("handler on %s mask %lx proc %s\n", w->path, mask, di.dli_sname ? di.dli_sname : "?"); }
  for(h = w->handlers; h; h = h->next) {
    if(h->proc == proc && h->data == data) { h->mask = mask; return; }
  }
  h = calloc(1, sizeof(Handler));
  h->mask = mask;
  h->proc = proc;
  h->data = data;
  h->next = w->handlers;
  w->handlers = h;
}

static void t_DeleteEventHandler(Tk_Window tw, unsigned long mask, Tk_EventProc *proc,
                                 ClientData data)
{
  CdmWin *w = (CdmWin *)tw;
  Handler **p = &w->handlers;
  (void)mask;
  while(*p) {
    if((*p)->proc == proc && (*p)->data == data) {
      Handler *gone = *p;
      *p = gone->next;
      free(gone);
    } else p = &(*p)->next;
  }
}

static void t_CreateGenericHandler(Tk_GenericProc *proc, ClientData data) { (void)proc; (void)data; }

static void t_GetVRootGeometry(Tk_Window tw, int *x, int *y, int *w, int *h)
{
  (void)tw;
  *x = 0; *y = 0; *w = screen_w; *h = screen_h;
}

static GC t_GetGC(Tk_Window tw, unsigned long mask, XGCValues *v)
{ (void)tw; return XCreateGC(NULL, 0, mask, v); }
static void t_FreeGC(Display *d, GC gc) { XFreeGC(d, gc); }
static Pixmap t_GetPixmap(Display *d, Drawable dr, int w, int h, int depth)
{ return XCreatePixmap(d, dr, (unsigned)w, (unsigned)h, (unsigned)depth); }
static void t_FreePixmap(Display *d, Pixmap p) { XFreePixmap(d, p); }

/* Fonts: Magic's Cairo driver draws text with cairo; it only needs Tk fonts to exist. */
typedef struct { int size; } FakeFont;
static Tk_Font t_GetFont(Tcl_Interp *ip, Tk_Window tw, const char *spec)
{
  FakeFont *f = calloc(1, sizeof(FakeFont));
  (void)ip; (void)tw; (void)spec;
  f->size = 12;
  return (Tk_Font)f;
}
static void t_FreeFont(Tk_Font f) { free(f); }
static Font t_FontId(Tk_Font f) { (void)f; return 1; }
static void t_GetFontMetrics(Tk_Font f, Tk_FontMetrics *m)
{ (void)f; m->ascent = 12; m->descent = 3; m->linespace = 15; }
static int t_TextWidth(Tk_Font f, const char *s, int n)
{ (void)f; if(n < 0) n = (int)strlen(s); return 7 * n; }
static void t_DrawChars(Display *d, Drawable dr, GC gc, Tk_Font f, const char *s, int n, int x, int y)
{ (void)d; (void)dr; (void)gc; (void)f; (void)s; (void)n; (void)x; (void)y; }

/* Tk_Uid: strings kept forever, one copy each. */
static Tcl_HashTable uids;
static int uids_ready = 0;
static Tk_Uid t_GetUid(const char *s)
{
  int isnew;
  Tcl_HashEntry *e;
  if(!uids_ready) { Tcl_InitHashTable(&uids, TCL_STRING_KEYS); uids_ready = 1; }
  e = Tcl_CreateHashEntry(&uids, s, &isnew);
  return (Tk_Uid)Tcl_GetHashKey(&uids, e);
}

/* Colours */
static XColor colors[256];
static int ncolors = 0;
static XColor *keep_color(unsigned r, unsigned g, unsigned b)
{
  XColor *c;
  int i;
  for(i = 0; i < ncolors; ++i)
    if(colors[i].red == r && colors[i].green == g && colors[i].blue == b) return &colors[i];
  if(ncolors == 256) return &colors[0];
  c = &colors[ncolors++];
  c->red = (unsigned short)r; c->green = (unsigned short)g; c->blue = (unsigned short)b;
  c->pixel = ((r >> 8) << 16) | ((g >> 8) << 8) | (b >> 8);
  c->flags = DoRed | DoGreen | DoBlue;
  return c;
}
static XColor *t_GetColor(Tcl_Interp *ip, Tk_Window tw, Tk_Uid name)
{
  unsigned r = 0, g = 0, b = 0;
  (void)ip; (void)tw;
  if(name && name[0] == '#' && strlen(name) == 7 && sscanf(name + 1, "%2x%2x%2x", &r, &g, &b) == 3)
    return keep_color(r * 257, g * 257, b * 257);
  if(name && !strcasecmp(name, "white")) return keep_color(65535, 65535, 65535);
  return keep_color(0, 0, 0);
}
static XColor *t_GetColorByValue(Tk_Window tw, XColor *c)
{ (void)tw; return keep_color(c->red, c->green, c->blue); }
static char color_name[16];
static CONST84_RETURN char *t_NameOfColor(XColor *c)
{
  snprintf(color_name, sizeof(color_name), "#%02x%02x%02x", c->red >> 8, c->green >> 8, c->blue >> 8);
  return color_name;
}

static Tk_Cursor t_GetCursorFromData(Tcl_Interp *ip, Tk_Window tw, const char *src, const char *mask,
                                     int w, int h, int xh, int yh, Tk_Uid fg, Tk_Uid bg)
{
  static int n = 0;
  (void)ip; (void)tw; (void)src; (void)mask; (void)w; (void)h; (void)xh; (void)yh; (void)fg; (void)bg;
  return (Tk_Cursor)(intptr_t)(++n);
}
static void t_FreeCursor(Display *d, Tk_Cursor c) { (void)d; (void)c; }

/* Images: Magic registers its "layer" image type; images of it are made only by its
 * GUI wrapper, which the helper does not use. */
/* An address inside tclmagic.so (its image type record), to find that library. */
static const void *magic_address = NULL;
static void t_CreateImageType(const Tk_ImageType *type) { if(!magic_address) magic_address = type; }
static void t_ImageChanged(Tk_ImageMaster m, int x, int y, int w, int h, int iw, int ih)
{ (void)m; (void)x; (void)y; (void)w; (void)h; (void)iw; (void)ih; }
static CONST84_RETURN char *t_NameOfImage(Tk_ImageMaster m) { (void)m; return ""; }
static void t_DeleteImage(Tcl_Interp *ip, const char *name) { (void)ip; (void)name; }

static int t_ConfigureWidget(Tcl_Interp *ip, Tk_Window tw, const Tk_ConfigSpec *specs, int argc,
                             CONST84 char **argv, char *rec, int flags)
{
  (void)tw; (void)specs; (void)argc; (void)argv; (void)rec; (void)flags;
  Tcl_SetResult(ip, "not supported in CDePT's Magic", TCL_STATIC);
  return TCL_ERROR;
}
static int t_ConfigureInfo(Tcl_Interp *ip, Tk_Window tw, const Tk_ConfigSpec *specs, char *rec,
                           const char *name, int flags)
{ return t_ConfigureWidget(ip, tw, specs, 0, NULL, rec, flags + 0 * (name != NULL)); }
static int t_ConfigureValue(Tcl_Interp *ip, Tk_Window tw, const Tk_ConfigSpec *specs, char *rec,
                            const char *name, int flags)
{ return t_ConfigureInfo(ip, tw, specs, rec, name, flags); }
static void t_FreeOptions(const Tk_ConfigSpec *specs, char *rec, Display *d, int flags)
{ (void)specs; (void)rec; (void)d; (void)flags; }

static int t_Init(Tcl_Interp *ip) { (void)ip; return TCL_OK; }

/* The table. Slots not named here are filled with traps by fill_traps(). */
static TkStubs stubs = {
  .magic = TCL_STUB_MAGIC,
  .hooks = NULL,
  .tk_MainWindow = t_MainWindow,
  .tk_CreateWindowFromPath = t_CreateWindowFromPath,
  .tk_NameToWindow = t_NameToWindow,
  .tk_MakeWindowExist = t_MakeWindowExist,
  .tk_MapWindow = t_MapWindow,
  .tk_UnmapWindow = t_UnmapWindow,
  .tk_DestroyWindow = t_DestroyWindow,
  .tk_GeometryRequest = t_GeometryRequest,
  .tk_MoveResizeWindow = t_MoveResizeWindow,
  .tk_ResizeWindow = t_ResizeWindow,
  .tk_MoveWindow = t_MoveWindow,
  .tk_SetWindowVisual = t_SetWindowVisual,
  .tk_SetWindowColormap = t_SetWindowColormap,
  .tk_ChangeWindowAttributes = t_ChangeWindowAttributes,
  .tk_RestackWindow = t_RestackWindow,
  .tk_DefineCursor = t_DefineCursor,
  .tk_CreateEventHandler = t_CreateEventHandler,
  .tk_DeleteEventHandler = t_DeleteEventHandler,
  .tk_CreateGenericHandler = t_CreateGenericHandler,
  .tk_GetVRootGeometry = t_GetVRootGeometry,
  .tk_GetGC = t_GetGC,
  .tk_FreeGC = t_FreeGC,
  .tk_GetPixmap = t_GetPixmap,
  .tk_FreePixmap = t_FreePixmap,
  .tk_GetFont = t_GetFont,
  .tk_FreeFont = t_FreeFont,
  .tk_FontId = t_FontId,
  .tk_GetFontMetrics = t_GetFontMetrics,
  .tk_TextWidth = t_TextWidth,
  .tk_DrawChars = t_DrawChars,
  .tk_GetUid = t_GetUid,
  .tk_GetColor = t_GetColor,
  .tk_GetColorByValue = t_GetColorByValue,
  .tk_NameOfColor = t_NameOfColor,
  .tk_GetCursorFromData = t_GetCursorFromData,
  .tk_FreeCursor = t_FreeCursor,
  .tk_CreateImageType = t_CreateImageType,
  .tk_ImageChanged = t_ImageChanged,
  .tk_NameOfImage = t_NameOfImage,
  .tk_DeleteImage = t_DeleteImage,
  .tk_ConfigureWidget = t_ConfigureWidget,
  .tk_ConfigureInfo = t_ConfigureInfo,
  .tk_ConfigureValue = t_ConfigureValue,
  .tk_FreeOptions = t_FreeOptions,
  .tk_Init = t_Init,
  .tk_SafeInit = t_Init,
};

/* One trap per slot, so the message can say which function Magic wanted. */
#define TRAPS 300
#define TRAP(n) static void trap_##n(void) { \
  fprintf(stderr, "cdept-magic: Tk function in stub slot %d is not implemented\n", (n) - 100); \
  abort(); }
#define T10(n) TRAP(n##0) TRAP(n##1) TRAP(n##2) TRAP(n##3) TRAP(n##4) TRAP(n##5) TRAP(n##6) TRAP(n##7) TRAP(n##8) TRAP(n##9)
T10(10) T10(11) T10(12) T10(13) T10(14) T10(15) T10(16) T10(17) T10(18) T10(19) T10(20) T10(21) T10(22) T10(23) T10(24) T10(25) T10(26) T10(27) T10(28) T10(29) T10(30) T10(31) T10(32) T10(33) T10(34) T10(35) T10(36) T10(37) T10(38) T10(39)
#define R10(n) trap_##n##0, trap_##n##1, trap_##n##2, trap_##n##3, trap_##n##4, trap_##n##5, trap_##n##6, trap_##n##7, trap_##n##8, trap_##n##9
static void (*const traps[TRAPS])(void) = {
  R10(10), R10(11), R10(12), R10(13), R10(14), R10(15), R10(16), R10(17), R10(18), R10(19), R10(20), R10(21), R10(22), R10(23), R10(24), R10(25), R10(26), R10(27), R10(28), R10(29), R10(30), R10(31), R10(32), R10(33), R10(34), R10(35), R10(36), R10(37), R10(38), R10(39)
};

static void fill_traps(void)
{
  void (**slot)(void) = (void (**)(void))((char *)&stubs + offsetof(TkStubs, tk_MainLoop));
  void (**end)(void) = (void (**)(void))((char *)&stubs + sizeof(stubs));
  int i = 0;
  /* trap_n reports n: tk_MainLoop is stub 0 */
  for(; slot < end && i < TRAPS; ++slot, ++i) {
    if(*slot == NULL) *slot = traps[i];
  }
}

/* ------------------------------------------------------------------------------ */
/* Tcl side, event loop                                                            */

static void (*ui_sink)(const char *json) = NULL;

static int ui_command(ClientData data, Tcl_Interp *ip, int objc, Tcl_Obj *const objv[])
{
  (void)data;
  if(objc != 2) { Tcl_WrongNumArgs(ip, 1, objv, "json"); return TCL_ERROR; }
  if(ui_sink) ui_sink(Tcl_GetString(objv[1]));
  return TCL_OK;
}

/* cdept::colour name -> #rrggbb, for faketk's `winfo rgb` */
static int colour_command(ClientData data, Tcl_Interp *ip, int objc, Tcl_Obj *const objv[])
{
  XColor *c;
  (void)data;
  if(objc != 2) { Tcl_WrongNumArgs(ip, 1, objv, "name"); return TCL_ERROR; }
  c = t_GetColor(ip, NULL, Tcl_GetString(objv[1]));
  Tcl_SetObjResult(ip, Tcl_NewStringObj(t_NameOfColor(c), -1));
  return TCL_OK;
}

/* cdept::setpoint: put Magic's current point at the pointer (the middle of the window
 * before any input), so that commands reach the layout window. */
static int setpoint_command(ClientData data, Tcl_Interp *ip, int objc, Tcl_Obj *const objv[])
{
  (void)data; (void)objc; (void)objv;
  if(drawing && !drawing->destroyed && !configured && (drawing->fake.flags & TK_MAPPED))
    first_show(drawing);
  if(drawing && !drawing->destroyed) {
    if(pointer_x == 0 && pointer_y == 0) {
      pointer_x = Tk_Width((Tk_Window)drawing) / 2;
      pointer_y = Tk_Height((Tk_Window)drawing) / 2;
    }
    set_point(pointer_x, pointer_y);
  }
  Tcl_ResetResult(ip);
  return TCL_OK;
}

/* Magic 8.3 measures the height of a window caption once, in WindInit(), before any
 * window exists: its Cairo driver's GrTextSize() then fails and leaves the rectangle
 * unset, so `windCaptionPixels` gets whatever was on the stack. It is used for the
 * minimum window height even when windows have no caption: a window of the tab's size
 * is then made as tall as that garbage, and the drawing is cut off at the bottom.
 * The variable is not exported, so it is found in the symbol table of tclmagic.so
 * and set to a caption height here. Returns 1 when set. */
static int fix_caption(int pixels)
{
  const void *sym = magic_address;
  Dl_info info;
  FILE *f;
  Elf64_Ehdr eh;
  Elf64_Shdr *sh = NULL;
  int i, found = 0;
  if(!sym || !dladdr(sym, &info) || !info.dli_fname) return 0;
  f = fopen(info.dli_fname, "rb");
  if(!f) return 0;
  if(fread(&eh, sizeof(eh), 1, f) != 1 || memcmp(eh.e_ident, ELFMAG, SELFMAG) != 0
     || eh.e_ident[EI_CLASS] != ELFCLASS64 || eh.e_shentsize != sizeof(Elf64_Shdr)
     || eh.e_shnum == 0 || eh.e_shnum > 4096) goto done;
  sh = calloc(eh.e_shnum, sizeof(Elf64_Shdr));
  if(fseek(f, (long)eh.e_shoff, SEEK_SET) != 0
     || fread(sh, sizeof(Elf64_Shdr), eh.e_shnum, f) != eh.e_shnum) goto done;
  for(i = 0; i < eh.e_shnum && !found; ++i) {
    Elf64_Shdr *strtab;
    char *names;
    Elf64_Sym sy;
    Elf64_Xword n, k;
    if(sh[i].sh_type != SHT_SYMTAB || sh[i].sh_link >= eh.e_shnum) continue;
    strtab = &sh[sh[i].sh_link];
    if(strtab->sh_size == 0 || strtab->sh_size > (64u << 20)) continue;
    names = malloc(strtab->sh_size + 1);
    if(fseek(f, (long)strtab->sh_offset, SEEK_SET) != 0
       || fread(names, 1, strtab->sh_size, f) != strtab->sh_size) { free(names); continue; }
    names[strtab->sh_size] = 0;
    n = sh[i].sh_entsize ? sh[i].sh_size / sh[i].sh_entsize : 0;
    if(fseek(f, (long)sh[i].sh_offset, SEEK_SET) != 0) { free(names); continue; }
    for(k = 0; k < n; ++k) {
      if(fread(&sy, sizeof(sy), 1, f) != 1) break;
      if(sy.st_name < strtab->sh_size && ELF64_ST_TYPE(sy.st_info) == STT_OBJECT
         && sy.st_size == sizeof(int) && !strcmp(names + sy.st_name, "windCaptionPixels")) {
        int *var = (int *)((char *)info.dli_fbase + sy.st_value);
        TRACE("windCaptionPixels was %d, now %d\n", *var, pixels);
        *var = pixels;
        found = 1;
        break;
      }
    }
    free(names);
  }
done:
  free(sh);
  fclose(f);
  return found;
}

static int fixcaption_command(ClientData data, Tcl_Interp *ip, int objc, Tcl_Obj *const objv[])
{
  int pixels = 18;
  (void)data;
  if(objc == 2 && Tcl_GetIntFromObj(ip, objv[1], &pixels) != TCL_OK) return TCL_ERROR;
  Tcl_SetObjResult(ip, Tcl_NewIntObj(fix_caption(pixels)));
  return TCL_OK;
}

int cdm_init(const char *argv0, int display, int sw, int sh)
{
  if(sw > 0) screen_w = sw;
  if(sh > 0) screen_h = sh;
  setvbuf(stdout, NULL, _IOLBF, 0);
  init_display();
  Tcl_FindExecutable(argv0);
  interp = Tcl_CreateInterp();
  if(Tcl_Init(interp) != TCL_OK) return TCL_ERROR;
  Tcl_Eval(interp, "namespace eval cdept {}");
  Tcl_CreateObjCommand(interp, "cdept::ui", ui_command, NULL, NULL);
  Tcl_CreateObjCommand(interp, "cdept::colour", colour_command, NULL, NULL);
  Tcl_CreateObjCommand(interp, "cdept::setpoint", setpoint_command, NULL, NULL);
  Tcl_CreateObjCommand(interp, "cdept::fixcaption", fixcaption_command, NULL, NULL);
  init_window(&main_window, ".", NULL, 1);
  main_window.fake.flags |= TK_MAPPED;
  main_window.fake.changes.width = screen_w;
  main_window.fake.changes.height = screen_h;
  set_drw_vars();
  if(display) {
    fill_traps();
    if(Tcl_PkgProvideEx(interp, "Tk", "8.6.14", (ClientData)&stubs) != TCL_OK) return TCL_ERROR;
  }
  return TCL_OK;
}

int cdm_eval(const char *script) { return Tcl_EvalEx(interp, script, -1, TCL_EVAL_GLOBAL); }
const char *cdm_result(void) { return Tcl_GetStringResult(interp); }
void cdm_set_ui_sink(void (*sink)(const char *json)) { ui_sink = sink; }

static void fd_readable(ClientData data, int mask)
{
  void (*readable)(void) = (void (*)(void))data;
  (void)mask;
  readable();
}
void cdm_watch_fd(int fd, void (*readable)(void))
{ Tcl_CreateFileHandler(fd, TCL_READABLE, fd_readable, (ClientData)readable); }

static void idle_call(ClientData data) { ((void (*)(void))data)(); }
void cdm_when_idle(void (*call)(void)) { Tcl_DoWhenIdle(idle_call, (ClientData)call); }
int cdm_do_one_event(int wait) { return Tcl_DoOneEvent(TCL_ALL_EVENTS | (wait ? 0 : TCL_DONT_WAIT)); }

static int tick_ms = 0;
static void (*tick_call)(void) = NULL;
static void tick_timer(ClientData data)
{
  (void)data;
  if(tick_call) tick_call();
  Tcl_CreateTimerHandler(tick_ms, tick_timer, NULL);
}
void cdm_every(int ms, void (*tick)(void))
{
  tick_ms = ms;
  tick_call = tick;
  Tcl_CreateTimerHandler(ms, tick_timer, NULL);
}

/* ------------------------------------------------------------------------------ */
/* The drawing window                                                              */

void cdm_display_resize(int width, int height)
{
  if(width < 1) width = 1;
  if(height < 1) height = 1;
  if(width > screen_w) width = screen_w;
  if(height > screen_h) height = screen_h;
  want_w = width;
  want_h = height;
  force_full = 1;
  if(!drawing) { set_drw_vars(); return; }
  if(Tk_Width((Tk_Window)drawing) == width && Tk_Height((Tk_Window)drawing) == height) {
    set_drw_vars();
    return;
  }
  drawing->fake.changes.width = width;
  drawing->fake.changes.height = height;
  set_drw_vars();
  if(drawing->fake.flags & TK_MAPPED) send_configure(drawing);
}

static const char *binding_name(int type, int button)
{
  static char buf[32];
  switch(type) {
    case MotionNotify: return "<Motion>";
    case ButtonRelease: snprintf(buf, sizeof(buf), "<ButtonRelease-%d>", button); return buf;
    case EnterNotify: return "<Enter>";
    case LeaveNotify: return "<Leave>";
    default: return NULL;
  }
}

void cdm_display_event(int type, int x, int y, unsigned long keysym, int button, int state)
{
  XEvent ev;
  unsigned long mask = 0;
  const char *bind;
  pointer_x = x;
  pointer_y = y;
  set_drw_vars();
  if(!drawing || !(drawing->fake.flags & TK_MAPPED)) return;
  set_point(x, y);
  memset(&ev, 0, sizeof(ev));
  ev.type = type;
  switch(type) {
    case KeyPress: case KeyRelease:
      ev.xkey.x = x; ev.xkey.y = y; ev.xkey.x_root = x; ev.xkey.y_root = y;
      ev.xkey.state = (unsigned)state; ev.xkey.keycode = 0; ev.xkey.same_screen = True;
      mask = type == KeyPress ? KeyPressMask : KeyReleaseMask;
      pending_keysym = keysym;
      break;
    case ButtonPress: case ButtonRelease:
      ev.xbutton.x = x; ev.xbutton.y = y; ev.xbutton.x_root = x; ev.xbutton.y_root = y;
      ev.xbutton.state = (unsigned)state; ev.xbutton.button = (unsigned)button;
      ev.xbutton.same_screen = True;
      mask = type == ButtonPress ? ButtonPressMask : ButtonReleaseMask;
      break;
    case MotionNotify:
      ev.xmotion.x = x; ev.xmotion.y = y; ev.xmotion.state = (unsigned)state;
      mask = PointerMotionMask | ((state & 0x1f00) ? ButtonMotionMask : 0);
      break;
    default:
      return;
  }
  dispatch(drawing, &ev, mask);
  pending_keysym = 0;
  /* Tcl level bindings (`bind .magic1 <Motion> ...` of Magic's tools) */
  bind = binding_name(type, button);
  if(bind && interp && drawing) {
    Tcl_Obj *cmd = Tcl_ObjPrintf(
      "if {[llength [info commands ::faketk::dispatch]]} "
      "{::faketk::dispatch %s %s {x %d y %d W %s}}", drawing->path, bind, x, y, drawing->path);
    Tcl_IncrRefCount(cmd);
    if(Tcl_EvalObjEx(interp, cmd, TCL_EVAL_GLOBAL) != TCL_OK)
      fprintf(stderr, "cdept-magic: binding %s: %s\n", bind, Tcl_GetStringResult(interp));
    Tcl_DecrRefCount(cmd);
  }
}

int cdm_display_size(int *width, int *height)
{
  if(!drawing || !(drawing->fake.flags & TK_MAPPED)) return 0;
  *width = Tk_Width((Tk_Window)drawing);
  *height = Tk_Height((Tk_Window)drawing);
  return 1;
}

int cdm_display_take_damage(int *rx, int *ry, int *rw, int *rh)
{
  Drawable_ *d;
  uint32_t *src;
  int w, h, stride, y, x0 = -1, x1 = -1, y0 = -1, y1 = -1;
  if(!drawing || !(d = drawable_of(drawing->fake.window))) return 0;
  w = Tk_Width((Tk_Window)drawing);
  h = Tk_Height((Tk_Window)drawing);
  cairo_surface_flush(d->surface);
  src = (uint32_t *)cairo_image_surface_get_data(d->surface);
  stride = cairo_image_surface_get_stride(d->surface) / 4;
  if(force_full || !shadow || shadow_w != w || shadow_h != h) {
    free(shadow);
    shadow = malloc((size_t)w * (size_t)h * 4);
    shadow_w = w;
    shadow_h = h;
    for(y = 0; y < h; ++y) memcpy(shadow + (size_t)y * w, src + (size_t)y * stride, (size_t)w * 4);
    force_full = 0;
    *rx = 0; *ry = 0; *rw = w; *rh = h;
    return 1;
  }
  for(y = 0; y < h; ++y) {
    uint32_t *a = shadow + (size_t)y * w, *b = src + (size_t)y * stride;
    int l, r;
    if(!memcmp(a, b, (size_t)w * 4)) continue;
    for(l = 0; l < w && ((a[l] ^ b[l]) & 0xffffff) == 0; ++l) ;
    if(l == w) continue;
    for(r = w - 1; r > l && ((a[r] ^ b[r]) & 0xffffff) == 0; --r) ;
    memcpy(a + l, b + l, (size_t)(r - l + 1) * 4);
    if(y0 < 0) y0 = y;
    y1 = y;
    if(x0 < 0 || l < x0) x0 = l;
    if(r > x1) x1 = r;
  }
  if(y0 < 0) return 0;
  *rx = x0; *ry = y0; *rw = x1 - x0 + 1; *rh = y1 - y0 + 1;
  return 1;
}

void cdm_display_read(unsigned char *dst, int dst_stride, int x, int y, int width, int height)
{
  Drawable_ *d;
  uint32_t *src;
  int stride, row, col;
  if(!drawing || !(d = drawable_of(drawing->fake.window))) return;
  src = (uint32_t *)cairo_image_surface_get_data(d->surface);
  stride = cairo_image_surface_get_stride(d->surface) / 4;
  for(row = 0; row < height; ++row) {
    uint32_t *s = src + (size_t)(y + row) * stride + x;
    unsigned char *o = dst + (size_t)row * dst_stride;
    for(col = 0; col < width; ++col) {
      uint32_t p = s[col];
      o[4 * col] = (p >> 16) & 0xff;
      o[4 * col + 1] = (p >> 8) & 0xff;
      o[4 * col + 2] = p & 0xff;
      o[4 * col + 3] = 0xff;
    }
  }
}

const char *cdm_display_path(void) { return drawing ? drawing->path : NULL; }
const char *cdm_display_title(void) { return title; }
