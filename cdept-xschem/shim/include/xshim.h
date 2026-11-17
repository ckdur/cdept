/* xshim.h: stand-in for the parts of Xlib that xschem uses.
 *
 * xschem is compiled against this header instead of <X11/Xlib.h> and friends. The types
 * keep the Xlib layout and the constants keep the X protocol values, so xschem's code
 * (and xschem.tcl, which passes event numbers and state masks) behaves the same.
 *
 * Every function is renamed with an xs_ prefix through the macros below. libcairo
 * itself links libX11 on most systems, and without the prefix our definitions would
 * replace the real ones inside that library.
 *
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#ifndef CDEPT_XSHIM_H
#define CDEPT_XSHIM_H

#include <stddef.h>

/* ---- basic types ------------------------------------------------------------------ */
typedef unsigned long XID;
typedef XID Window;
typedef XID Drawable;
typedef XID Pixmap;
typedef XID Cursor;
typedef XID Colormap;
typedef XID KeySym;
typedef XID Font;
typedef unsigned long Atom;
typedef unsigned long VisualID;
typedef unsigned long Time;
typedef int Bool;
typedef int Status;
typedef char *XPointer;

#define True 1
#define False 0
#define None 0L
#define Success 0

typedef struct xs_display Display;
typedef struct xs_gc *GC;
typedef struct xs_visual { VisualID visualid; int depth; } Visual;
typedef struct xs_screen { Display *display; Window root; int width, height; } Screen;

typedef struct {
  unsigned long pixel;
  unsigned short red, green, blue;
  char flags;
  char pad;
} XColor;

typedef struct { short x1, y1, x2, y2; } XSegment;
typedef struct { short x, y; } XPoint;
typedef struct { short x, y; unsigned short width, height; } XRectangle;
typedef struct { short x, y; unsigned short width, height; short angle1, angle2; } XArc;

typedef struct {
  int function;
  unsigned long plane_mask;
  unsigned long foreground;
  unsigned long background;
  int line_width;
  int line_style;
  int cap_style;
  int join_style;
  int fill_style;
  int fill_rule;
  int arc_mode;
  Pixmap tile;
  Pixmap stipple;
  int ts_x_origin;
  int ts_y_origin;
  Font font;
  int subwindow_mode;
  Bool graphics_exposures;
  int clip_x_origin;
  int clip_y_origin;
  Pixmap clip_mask;
  int dash_offset;
  char dashes;
} XGCValues;

typedef struct {
  Pixmap background_pixmap;
  unsigned long background_pixel;
  Pixmap border_pixmap;
  unsigned long border_pixel;
  int bit_gravity;
  int win_gravity;
  int backing_store;
  unsigned long backing_planes;
  unsigned long backing_pixel;
  Bool save_under;
  long event_mask;
  long do_not_propagate_mask;
  Bool override_redirect;
  Colormap colormap;
  Cursor cursor;
} XSetWindowAttributes;

typedef struct {
  int x, y;
  int width, height;
  int border_width;
  int depth;
  Visual *visual;
  Window root;
  int class;
  int bit_gravity;
  int win_gravity;
  int backing_store;
  unsigned long backing_planes;
  unsigned long backing_pixel;
  Bool save_under;
  Colormap colormap;
  Bool map_installed;
  int map_state;
  long all_event_masks;
  long your_event_mask;
  long do_not_propagate_mask;
  Bool override_redirect;
  Screen *screen;
} XWindowAttributes;

typedef struct {
  long flags;
  Bool input;
  int initial_state;
  Pixmap icon_pixmap;
  Window icon_window;
  int icon_x, icon_y;
  Pixmap icon_mask;
  XID window_group;
} XWMHints;

typedef struct {
  int key_click_percent;
  int bell_percent;
  unsigned int bell_pitch, bell_duration;
  unsigned long led_mask;
  int global_auto_repeat;
  char auto_repeats[32];
} XKeyboardState;

typedef struct {
  int type;
  Display *display;
  XID resourceid;
  unsigned long serial;
  unsigned char error_code;
  unsigned char request_code;
  unsigned char minor_code;
} XErrorEvent;

typedef struct {
  Visual *visual;
  VisualID visualid;
  int screen;
  int depth;
  int class;
  unsigned long red_mask, green_mask, blue_mask;
  int colormap_size;
  int bits_per_rgb;
} XVisualInfo;

typedef struct {
  int type;
  unsigned long serial;
  Bool send_event;
  Display *display;
  Window window;
  Atom message_type;
  int format;
  union { char b[20]; short s[10]; long l[5]; } data;
} XClientMessageEvent;

typedef union {
  int type;
  XClientMessageEvent xclient;
  long pad[24];
} XEvent;

typedef int (*XErrorHandler)(Display *, XErrorEvent *);

/* ---- event types (X protocol numbers; xschem.tcl passes them as %T) --------------- */
#define KeyPress 2
#define KeyRelease 3
#define ButtonPress 4
#define ButtonRelease 5
#define MotionNotify 6
#define EnterNotify 7
#define LeaveNotify 8
#define FocusIn 9
#define FocusOut 10
#define Expose 12
#define ConfigureNotify 22
#define ClientMessage 33

/* ---- key and button state masks ---------------------------------------------------- */
#define ShiftMask (1 << 0)
#define LockMask (1 << 1)
#define ControlMask (1 << 2)
#define Mod1Mask (1 << 3)
#define Mod2Mask (1 << 4)
#define Mod3Mask (1 << 5)
#define Mod4Mask (1 << 6)
#define Mod5Mask (1 << 7)
#define Button1Mask (1 << 8)
#define Button2Mask (1 << 9)
#define Button3Mask (1 << 10)
#define Button4Mask (1 << 11)
#define Button5Mask (1 << 12)

#define Button1 1
#define Button2 2
#define Button3 3
#define Button4 4
#define Button5 5

/* ---- event masks --------------------------------------------------------------------- */
#define NoEventMask 0L
#define KeyPressMask (1L << 0)
#define KeyReleaseMask (1L << 1)
#define ButtonPressMask (1L << 2)
#define ButtonReleaseMask (1L << 3)
#define EnterWindowMask (1L << 4)
#define LeaveWindowMask (1L << 5)
#define PointerMotionMask (1L << 6)
#define ExposureMask (1L << 15)
#define StructureNotifyMask (1L << 17)
#define SubstructureNotifyMask (1L << 19)
#define SubstructureRedirectMask (1L << 20)

/* ---- graphics context ---------------------------------------------------------------- */
#define GXclear 0x0
#define GXcopy 0x3
#define GXnoop 0x5
#define GXxor 0x6
#define GXset 0xf

#define LineSolid 0
#define LineOnOffDash 1
#define LineDoubleDash 2

#define CapNotLast 0
#define CapButt 1
#define CapRound 2
#define CapProjecting 3

#define JoinMiter 0
#define JoinRound 1
#define JoinBevel 2

#define FillSolid 0
#define FillTiled 1
#define FillStippled 2
#define FillOpaqueStippled 3

#define EvenOddRule 0
#define WindingRule 1

#define ArcChord 0
#define ArcPieSlice 1

#define Complex 0
#define Nonconvex 1
#define Convex 2

#define CoordModeOrigin 0
#define CoordModePrevious 1

#define Unsorted 0
#define YSorted 1
#define YXSorted 2
#define YXBanded 3

#define GCFunction (1L << 0)
#define GCPlaneMask (1L << 1)
#define GCForeground (1L << 2)
#define GCBackground (1L << 3)
#define GCLineWidth (1L << 4)
#define GCLineStyle (1L << 5)
#define GCCapStyle (1L << 6)
#define GCJoinStyle (1L << 7)
#define GCFillStyle (1L << 8)
#define GCFillRule (1L << 9)
#define GCTile (1L << 10)
#define GCStipple (1L << 11)
#define GCTileStipXOrigin (1L << 12)
#define GCTileStipYOrigin (1L << 13)
#define GCFont (1L << 14)
#define GCSubwindowMode (1L << 15)
#define GCGraphicsExposures (1L << 16)
#define GCClipXOrigin (1L << 17)
#define GCClipYOrigin (1L << 18)
#define GCClipMask (1L << 19)
#define GCDashOffset (1L << 20)
#define GCDashList (1L << 21)
#define GCArcMode (1L << 22)

/* ---- windows --------------------------------------------------------------------------- */
#define CWBackPixmap (1L << 0)
#define CWBackPixel (1L << 1)
#define CWBorderPixmap (1L << 2)
#define CWBorderPixel (1L << 3)
#define CWBitGravity (1L << 4)
#define CWWinGravity (1L << 5)
#define CWBackingStore (1L << 6)
#define CWBackingPlanes (1L << 7)
#define CWBackingPixel (1L << 8)
#define CWOverrideRedirect (1L << 9)
#define CWSaveUnder (1L << 10)
#define CWEventMask (1L << 11)
#define CWDontPropagate (1L << 12)
#define CWColormap (1L << 13)
#define CWCursor (1L << 14)

#define NotUseful 0
#define WhenMapped 1
#define Always 2

#define InputOutput 1
#define InputOnly 2
#define CopyFromParent 0L

#define IsUnmapped 0
#define IsUnviewable 1
#define IsViewable 2

#define TrueColor 4
#define ZPixmap 2

#define DoRed (1 << 0)
#define DoGreen (1 << 1)
#define DoBlue (1 << 2)

#define InputHint (1L << 0)
#define StateHint (1L << 1)
#define IconPixmapHint (1L << 2)
#define IconWindowHint (1L << 3)
#define IconPositionHint (1L << 4)
#define IconMaskHint (1L << 5)
#define WindowGroupHint (1L << 6)

#define XA_ATOM ((Atom)4)
#define XA_CARDINAL ((Atom)6)
#define XA_STRING ((Atom)31)
#define XA_WINDOW ((Atom)33)

#define XpmSuccess 0

/* ---- key symbols used by xschem (X11 keysymdef values) --------------------------------- */
#define XK_BackSpace 0xff08
#define XK_Tab 0xff09
#define XK_Return 0xff0d
#define XK_Escape 0xff1b
#define XK_Delete 0xffff
#define XK_Home 0xff50
#define XK_Left 0xff51
#define XK_Up 0xff52
#define XK_Right 0xff53
#define XK_Down 0xff54
#define XK_Page_Up 0xff55
#define XK_Page_Down 0xff56
#define XK_End 0xff57
#define XK_Print 0xff61
#define XK_Insert 0xff63
#define XK_ISO_Left_Tab 0xfe20
#define XK_Shift_L 0xffe1
#define XK_Shift_R 0xffe2
#define XK_Control_L 0xffe3
#define XK_Control_R 0xffe4
#define XK_Alt_L 0xffe9
#define XK_Alt_R 0xffea
#define XK_slash 0x002f

/* ---- function renames ---------------------------------------------------------------- */
#define XAllocColor xs_XAllocColor
#define XAllocNamedColor xs_XAllocNamedColor
#define XAllocWMHints xs_XAllocWMHints
#define XChangeGC xs_XChangeGC
#define XCloseDisplay xs_XCloseDisplay
#define XCopyArea xs_XCopyArea
#define XCreateBitmapFromData xs_XCreateBitmapFromData
#define XCreateGC xs_XCreateGC
#define XCreatePixmap xs_XCreatePixmap
#define XCreateWindow xs_XCreateWindow
#define XDefaultScreen xs_XDefaultScreen
#define XDestroyWindow xs_XDestroyWindow
#define XDrawArc xs_XDrawArc
#define XDrawArcs xs_XDrawArcs
#define XDrawLine xs_XDrawLine
#define XDrawLines xs_XDrawLines
#define XDrawPoints xs_XDrawPoints
#define XDrawRectangle xs_XDrawRectangle
#define XDrawRectangles xs_XDrawRectangles
#define XDrawSegments xs_XDrawSegments
#define XExtendedMaxRequestSize xs_XExtendedMaxRequestSize
#define XFillArc xs_XFillArc
#define XFillArcs xs_XFillArcs
#define XFillPolygon xs_XFillPolygon
#define XFillRectangle xs_XFillRectangle
#define XFillRectangles xs_XFillRectangles
#define XFlush xs_XFlush
#define XFree xs_XFree
#define XFreeGC xs_XFreeGC
#define XFreePixmap xs_XFreePixmap
#define XGetErrorText xs_XGetErrorText
#define XGetKeyboardControl xs_XGetKeyboardControl
#define XGetWindowAttributes xs_XGetWindowAttributes
#define XInternAtom xs_XInternAtom
#define XLookupColor xs_XLookupColor
#define XMapRaised xs_XMapRaised
#define XMatchVisualInfo xs_XMatchVisualInfo
#define XMaxRequestSize xs_XMaxRequestSize
#define XOpenDisplay xs_XOpenDisplay
#define XPending xs_XPending
#define XQueryColor xs_XQueryColor
#define XQueryPointer xs_XQueryPointer
#define XQueryTree xs_XQueryTree
#define XSendEvent xs_XSendEvent
#define XSetBackground xs_XSetBackground
#define XSetClipMask xs_XSetClipMask
#define XSetClipRectangles xs_XSetClipRectangles
#define XSetDashes xs_XSetDashes
#define XSetErrorHandler xs_XSetErrorHandler
#define XSetFillStyle xs_XSetFillStyle
#define XSetForeground xs_XSetForeground
#define XSetLineAttributes xs_XSetLineAttributes
#define XSetStipple xs_XSetStipple
#define XSetTile xs_XSetTile
#define XSetWMHints xs_XSetWMHints
#define XSync xs_XSync
#define XpmCreatePixmapFromData xs_XpmCreatePixmapFromData
#define XpmWriteFileFromPixmap xs_XpmWriteFileFromPixmap
#define cairo_xlib_surface_create xs_cairo_surface_create
#define cairo_xlib_surface_get_width xs_cairo_surface_get_width
#define cairo_xlib_surface_get_height xs_cairo_surface_get_height
#define cairo_xlib_surface_set_size xs_cairo_surface_set_size

/* ---- display ---------------------------------------------------------------------------- */
Display *XOpenDisplay(const char *name);
int XCloseDisplay(Display *dpy);
int XDefaultScreen(Display *dpy);
Window xs_root_window(Display *dpy);
int XFlush(Display *dpy);
int XSync(Display *dpy, Bool discard);
int XPending(Display *dpy);
int XFree(void *data);
long XMaxRequestSize(Display *dpy);
long XExtendedMaxRequestSize(Display *dpy);
XErrorHandler XSetErrorHandler(XErrorHandler handler);
int XGetErrorText(Display *dpy, int code, char *buffer, int length);
Atom XInternAtom(Display *dpy, const char *name, Bool only_if_exists);
Status XSendEvent(Display *dpy, Window w, Bool propagate, long mask, XEvent *event);
int XGetKeyboardControl(Display *dpy, XKeyboardState *state);
Status XMatchVisualInfo(Display *dpy, int screen, int depth, int class, XVisualInfo *info);

#define DefaultRootWindow(dpy) xs_root_window(dpy)
#define DefaultScreen(dpy) XDefaultScreen(dpy)
int xs_display_width(Display *dpy);
int xs_display_height(Display *dpy);
#define DisplayWidth(dpy, screen) xs_display_width(dpy)
#define DisplayHeight(dpy, screen) xs_display_height(dpy)
/* Pixels are 0x00RRGGBB. */
#define BlackPixel(dpy, screen) 0x000000UL
#define WhitePixel(dpy, screen) 0xffffffUL

/* ---- windows and pixmaps ---------------------------------------------------------------- */
Window XCreateWindow(Display *dpy, Window parent, int x, int y, unsigned int width,
    unsigned int height, unsigned int border_width, int depth, unsigned int class,
    Visual *visual, unsigned long valuemask, XSetWindowAttributes *attributes);
int XDestroyWindow(Display *dpy, Window w);
int XMapRaised(Display *dpy, Window w);
Status XGetWindowAttributes(Display *dpy, Window w, XWindowAttributes *attributes);
Status XQueryTree(Display *dpy, Window w, Window *root, Window *parent, Window **children,
    unsigned int *nchildren);
Bool XQueryPointer(Display *dpy, Window w, Window *root, Window *child, int *root_x,
    int *root_y, int *win_x, int *win_y, unsigned int *mask);
XWMHints *XAllocWMHints(void);
int XSetWMHints(Display *dpy, Window w, XWMHints *hints);
Pixmap XCreatePixmap(Display *dpy, Drawable d, unsigned int width, unsigned int height,
    unsigned int depth);
Pixmap XCreateBitmapFromData(Display *dpy, Drawable d, const char *data, unsigned int width,
    unsigned int height);
int XFreePixmap(Display *dpy, Pixmap pixmap);
int XpmCreatePixmapFromData(Display *dpy, Drawable d, char **data, Pixmap *pixmap,
    Pixmap *mask, void *attributes);
int XpmWriteFileFromPixmap(Display *dpy, const char *filename, Pixmap pixmap, Pixmap mask,
    void *attributes);

/* ---- colours -------------------------------------------------------------------------- */
Status XAllocColor(Display *dpy, Colormap colormap, XColor *color);
Status XAllocNamedColor(Display *dpy, Colormap colormap, const char *name, XColor *screen,
    XColor *exact);
Status XLookupColor(Display *dpy, Colormap colormap, const char *name, XColor *exact,
    XColor *screen);
int XQueryColor(Display *dpy, Colormap colormap, XColor *color);

/* ---- graphics contexts ----------------------------------------------------------------- */
GC XCreateGC(Display *dpy, Drawable d, unsigned long valuemask, XGCValues *values);
int XChangeGC(Display *dpy, GC gc, unsigned long valuemask, XGCValues *values);
int XFreeGC(Display *dpy, GC gc);
int XSetForeground(Display *dpy, GC gc, unsigned long foreground);
int XSetBackground(Display *dpy, GC gc, unsigned long background);
int XSetLineAttributes(Display *dpy, GC gc, unsigned int line_width, int line_style,
    int cap_style, int join_style);
int XSetDashes(Display *dpy, GC gc, int dash_offset, const char *dash_list, int n);
int XSetFillStyle(Display *dpy, GC gc, int fill_style);
int XSetTile(Display *dpy, GC gc, Pixmap tile);
int XSetStipple(Display *dpy, GC gc, Pixmap stipple);
int XSetClipMask(Display *dpy, GC gc, Pixmap pixmap);
int XSetClipRectangles(Display *dpy, GC gc, int clip_x_origin, int clip_y_origin,
    XRectangle *rectangles, int n, int ordering);

/* ---- drawing ---------------------------------------------------------------------------- */
int XDrawLine(Display *dpy, Drawable d, GC gc, int x1, int y1, int x2, int y2);
int XDrawLines(Display *dpy, Drawable d, GC gc, XPoint *points, int npoints, int mode);
int XDrawSegments(Display *dpy, Drawable d, GC gc, XSegment *segments, int nsegments);
int XDrawPoints(Display *dpy, Drawable d, GC gc, XPoint *points, int npoints, int mode);
int XDrawRectangle(Display *dpy, Drawable d, GC gc, int x, int y, unsigned int width,
    unsigned int height);
int XDrawRectangles(Display *dpy, Drawable d, GC gc, XRectangle *rectangles, int n);
int XDrawArc(Display *dpy, Drawable d, GC gc, int x, int y, unsigned int width,
    unsigned int height, int angle1, int angle2);
int XDrawArcs(Display *dpy, Drawable d, GC gc, XArc *arcs, int narcs);
int XFillRectangle(Display *dpy, Drawable d, GC gc, int x, int y, unsigned int width,
    unsigned int height);
int XFillRectangles(Display *dpy, Drawable d, GC gc, XRectangle *rectangles, int n);
int XFillArc(Display *dpy, Drawable d, GC gc, int x, int y, unsigned int width,
    unsigned int height, int angle1, int angle2);
int XFillArcs(Display *dpy, Drawable d, GC gc, XArc *arcs, int narcs);
int XFillPolygon(Display *dpy, Drawable d, GC gc, XPoint *points, int npoints, int shape,
    int mode);
int XCopyArea(Display *dpy, Drawable src, Drawable dest, GC gc, int src_x, int src_y,
    unsigned int width, unsigned int height, int dest_x, int dest_y);

#endif /* CDEPT_XSHIM_H */
