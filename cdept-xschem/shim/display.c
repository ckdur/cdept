/* display.c: xschem's drawing window as the Rust side sees it (the cdx_display_*
 * functions of cdx.h), and a few Tcl commands for tests.
 *
 * The window is the stand-in X window behind the Tk path ".drw" (see xshim.c). Resizing
 * and input are delivered the way Tk's event bindings in xschem.tcl would deliver them:
 * by calling xschem's callback().
 *
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#include "xschem.h"
#include "cdx.h"
#include "xs_internal.h"

#define DRW ".drw"

/* Two presses of the same button count as a double click when this close. */
#define DOUBLE_CLICK_MS 400
#define DOUBLE_CLICK_PIXELS 4

extern int xs_pointer_x, xs_pointer_y;

static int mapped = 0;

static xs_drawable *drawing_window(void)
{
  if(!has_x || !xctx) return NULL;
  return xs_window_of_path(DRW);
}

static void set_drw_var(const char *name, int value)
{
  Tcl_SetVar2Ex(interp, "::cdept::drw", name, Tcl_NewIntObj(value), TCL_GLOBAL_ONLY);
}

/* Run a bit of Tcl that xschem.tcl's bindings run around an event. Failures do not
 * matter: the widgets it touches may not exist. */
static void quiet_eval(const char *script)
{
  Tcl_InterpState saved = Tcl_SaveInterpState(interp, TCL_OK);
  Tcl_EvalEx(interp, script, -1, TCL_EVAL_GLOBAL);
  Tcl_RestoreInterpState(interp, saved);
}

/* The view xschem was left with after the helper fitted it to the window; see
 * cdx_display_resize(). */
static int fitted = 0;
static double fit_zoom = 0.0, fit_xorigin = 0.0, fit_yorigin = 0.0;

/* Zoom so that the whole schematic is in the window, texts included.
 *
 * xschem's own full zoom uses the sizes of texts as measured when the file was loaded,
 * at whatever zoom it had then. Text sizes do not scale evenly with the zoom (font
 * hinting), so long texts can end up outside the window. Stock xschem has this too,
 * but there the window has its size before the file is loaded and the error is small.
 * Here: measure at the zoom in use, fit, and repeat while that zooms further out; as
 * soon as what was measured fits the view it was measured in, keep that view. */
static void fit_view(void)
{
  const double shrink = 0.97; /* what xschem's full zoom leaves around the schematic */
  int flags = 1 + 2 * tclgetboolvar("zoom_full_center");
  int round;
  for(round = 0; round < 6; ++round) {
    double zoom = xctx->zoom;
    quiet_eval("xschem update_all_sym_bboxes");
    zoom_full(0, 0, flags, shrink);
    if(round > 0 && xctx->zoom <= zoom) {
      /* Keep the zoom we had, and place the schematic in it as xschem's full zoom
       * places it (see zoom_full() in actions.c). */
      xRect box;
      double width, height;
      xctx->zoom = zoom;
      xctx->mooz = 1 / zoom;
      change_linewidth(-1.0);
      calc_drawing_bbox(&box, 0);
      width = zoom * (xctx->areaw - 4 * INT_LINE_W(xctx->lw));
      height = zoom * (xctx->areah - 4 * INT_LINE_W(xctx->lw));
      if(flags & 2) {
        xctx->xorigin = -box.x1 + (width - (box.x2 - box.x1)) / 2;
        xctx->yorigin = -box.y1 + (height - (box.y2 - box.y1)) / 2;
      } else {
        xctx->xorigin = -box.x1 + (1 - shrink) / 2 * width;
        xctx->yorigin = -box.y1 + height - (box.y2 - box.y1) - (1 - shrink) / 2 * height;
      }
      break;
    }
  }
  draw();
  fitted = 1;
  fit_zoom = xctx->zoom;
  fit_xorigin = xctx->xorigin;
  fit_yorigin = xctx->yorigin;
}

void cdx_display_resize(int width, int height)
{
  xs_drawable *win = drawing_window();
  int first_zoom, untouched;
  if(!win) return;
  if(width < 1) width = 1;
  if(height < 1) height = 1;
  if(mapped && width == win->width && height == win->height) return;
  xs_window_resize(win, width, height);
  mapped = 1;
  set_drw_var("width", width);
  set_drw_var("height", height);
  set_drw_var("mapped", 1);
  /* What `bind .drw <Configure>` does: xschem makes its off-screen pixmap anew for
   * this size and redraws. */
  first_zoom = xctx->pending_fullzoom;
  untouched = fitted && xctx->zoom == fit_zoom && xctx->xorigin == fit_xorigin &&
              xctx->yorigin == fit_yorigin;
  callback(DRW, ConfigureNotify, 0, 0, 0, width, height, 0);
  /* Fit the view when xschem has just zoomed to fit for the first time, and when the
   * window changes size again while the view is still that fit (a tab being laid out
   * gets its size in steps). A view the user chose is left alone, as in xschem. */
  if((first_zoom > 0 && xctx->pending_fullzoom < first_zoom) || untouched) fit_view();
}

static long now_ms(void)
{
  Tcl_Time now;
  Tcl_GetTime(&now);
  return now.sec * 1000L + now.usec / 1000L;
}

/* The second press of a double click is reported as event -3 instead of ButtonPress,
 * as `bind .drw <Double-Button-N>` does. */
static int is_double_click(int button, int x, int y)
{
  static long last_time = 0;
  static int last_button = 0, last_x = 0, last_y = 0;
  long now = now_ms();
  int twice = button == last_button && now - last_time <= DOUBLE_CLICK_MS &&
              abs(x - last_x) <= DOUBLE_CLICK_PIXELS && abs(y - last_y) <= DOUBLE_CLICK_PIXELS;
  last_button = twice ? 0 : button; /* a third press starts over */
  last_time = now;
  last_x = x;
  last_y = y;
  return twice;
}

/* Whether another window holds a Tk grab (a modal dialog is open). With Tk the drawing
 * window gets no input then. */
static int grabbed_elsewhere(void)
{
  Tcl_InterpState saved = Tcl_SaveInterpState(interp, TCL_OK);
  int elsewhere = 0;
  if(Tcl_EvalEx(interp, "grab current", -1, TCL_EVAL_GLOBAL) == TCL_OK) {
    const char *holder = Tcl_GetStringResult(interp);
    elsewhere = holder[0] && strcmp(holder, DRW);
  }
  Tcl_RestoreInterpState(interp, saved);
  return elsewhere;
}

/* A few keys make xschem's C code open another window or tab inside this process.
 * The Tcl side (cdept::key_filter in cdept.tcl), when loaded, gets every key press
 * first and says whether it dealt with it. */
static int key_taken(unsigned long keysym, int state)
{
  char script[96];
  int taken = 0;
  Tcl_InterpState saved;
  if(!Tcl_FindCommand(interp, "::cdept::key_filter", NULL, 0)) return 0;
  saved = Tcl_SaveInterpState(interp, TCL_OK);
  snprintf(script, sizeof(script), "::cdept::key_filter %lu %d", keysym, state);
  if(Tcl_EvalEx(interp, script, -1, TCL_EVAL_GLOBAL) == TCL_OK) {
    if(Tcl_GetBooleanFromObj(NULL, Tcl_GetObjResult(interp), &taken) != TCL_OK) taken = 0;
  } else {
    fprintf(stderr, "cdept-xschem: cdept::key_filter: %s\n", Tcl_GetStringResult(interp));
  }
  Tcl_RestoreInterpState(interp, saved);
  return taken;
}

void cdx_display_event(int type, int x, int y, unsigned long keysym, int button, int state)
{
  char script[160];
  if(!drawing_window() || !mapped) return;
  if(grabbed_elsewhere()) return;

  switch(type) {
    case KeyPress: case KeyRelease: case ButtonPress: case ButtonRelease: case MotionNotify:
    case EnterNotify:
      xs_pointer_x = x;
      xs_pointer_y = y;
      set_drw_var("pointerx", x);
      set_drw_var("pointery", y);
      break;
    default:
      break;
  }

  /* The Tcl side may take over (to run the bindings it recorded): see tkmin.tcl. */
  if(Tcl_FindCommand(interp, "::cdept::drw_event", NULL, 0)) {
    snprintf(script, sizeof(script), "::cdept::drw_event %d %d %d %lu %d %d", type, x, y,
             keysym, button, state);
    if(Tcl_EvalEx(interp, script, -1, TCL_EVAL_GLOBAL) != TCL_OK) {
      fprintf(stderr, "cdept-xschem: cdept::drw_event: %s\n", Tcl_GetStringResult(interp));
    }
    return;
  }

  /* Otherwise: what the bindings of xschem.tcl's set_bindings do. */
  switch(type) {
    case ButtonPress:
      if(button >= Button1 && button <= Button3 && is_double_click(button, x, y)) type = -3;
      callback(DRW, type, x, y, 0, button, 0, state);
      break;
    case ButtonRelease:
      callback(DRW, type, x, y, 0, button, 0, state);
      break;
    case MotionNotify: {
      int selecting = xctx->ui_state & STARTSELECT;
      callback(DRW, type, x, y, 0, 0, 0, state);
      /* xschem starts selecting by area on the first movement with the button down
       * and draws the rubber band from the second on. The movements of a quick drag
       * can arrive merged into one: hand that one over again, as a second movement
       * to the same place, so that the band shows. */
      if(!selecting && (xctx->ui_state & STARTSELECT)) callback(DRW, type, x, y, 0, 0, 0, state);
      break;
    }
    case KeyPress:
      if(keysym == XK_Escape) quiet_eval("destroy .ctxmenu");
      if(key_taken(keysym, state)) break;
      callback(DRW, type, x, y, (KeySym)keysym, 0, 0, state);
      break;
    case KeyRelease:
      callback(DRW, type, x, y, (KeySym)keysym, 0, 0, state);
      break;
    case EnterNotify:
      quiet_eval("if {[winfo exists .ins]} {set file_chooser(enter) 1}\n"
                 "if {$close_ctxmenu_on_leave} {destroy .ctxmenu}");
      callback(DRW, type, x, y, 0, 0, 0, 0);
      break;
    case LeaveNotify:
      callback(DRW, type, x, y, 0, 0, 0, state);
      quiet_eval("graph_show_measure stop");
      break;
    default:
      /* Focus changes only matter with several xschem windows. */
      break;
  }
}

int cdx_display_size(int *width, int *height)
{
  xs_drawable *win = drawing_window();
  if(!win || !mapped) return 0;
  *width = win->width;
  *height = win->height;
  return 1;
}

int cdx_display_take_damage(int *x, int *y, int *width, int *height)
{
  xs_drawable *win = drawing_window();
  if(!win || !mapped) return 0;
  return xs_take_damage(win, x, y, width, height);
}

int cdx_display_take_damage_list(int *rects, int max)
{
  xs_drawable *win = drawing_window();
  if(!win || !mapped) return 0;
  return xs_take_damage_list(win, rects, max);
}

/* Copy a rectangle of a window as R G B A bytes (see cdx_display_read()). */
static void read_window(xs_drawable *win, unsigned char *dst, int dst_stride, int x, int y,
    int width, int height)
{
  const unsigned char *data;
  int row, col, stride;

  if(!win || !win->surface) return;
  if(x < 0 || y < 0 || x + width > win->width || y + height > win->height) return;
  cairo_surface_flush(win->surface);
  data = cairo_image_surface_get_data(win->surface);
  stride = cairo_image_surface_get_stride(win->surface);
  for(row = 0; row < height; ++row) {
    /* cairo: one native-endian 32 bit word per pixel, 0xAARRGGBB */
    const unsigned int *src =
        (const unsigned int *)(const void *)(data + (size_t)(y + row) * (size_t)stride) + x;
    unsigned char *out = dst + (size_t)row * (size_t)dst_stride;
    for(col = 0; col < width; ++col) {
      unsigned int p = src[col];
      out[0] = (unsigned char)(p >> 16);
      out[1] = (unsigned char)(p >> 8);
      out[2] = (unsigned char)p;
      out[3] = 255; /* the window is opaque */
      out += 4;
    }
  }
}

void cdx_display_read(unsigned char *dst, int dst_stride, int x, int y, int width, int height)
{
  read_window(drawing_window(), dst, dst_stride, x, y, width, height);
}

/* ---- picture panes in dialogs -------------------------------------------------------------- */

#define MAX_PREVIEWS 16

static const char *preview_paths[MAX_PREVIEWS];
static xs_drawable *preview_wins[MAX_PREVIEWS];
static int npreviews = 0;

int cdx_preview_list(void)
{
  npreviews = has_x ? xs_preview_windows(preview_paths, preview_wins, MAX_PREVIEWS) : 0;
  return npreviews;
}

const char *cdx_preview_info(int index, int *width, int *height, int *changed)
{
  xs_drawable *win;
  int x, y, w, h;
  if(index < 0 || index >= npreviews) return NULL;
  win = preview_wins[index];
  *width = win->width;
  *height = win->height;
  *changed = xs_take_damage(win, &x, &y, &w, &h);
  return preview_paths[index];
}

void cdx_preview_read(int index, unsigned char *dst, int dst_stride)
{
  xs_drawable *win;
  if(index < 0 || index >= npreviews) return;
  win = preview_wins[index];
  read_window(win, dst, dst_stride, 0, 0, win->width, win->height);
}

/* ---- Tcl commands for tests ---------------------------------------------------------------- */

static int ints_from(Tcl_Interp *ip, int objc, Tcl_Obj *const objv[], long *values, int n,
    const char *usage)
{
  int i;
  if(objc != n + 1) {
    Tcl_WrongNumArgs(ip, 1, objv, usage);
    return TCL_ERROR;
  }
  for(i = 0; i < n; ++i) {
    if(Tcl_GetLongFromObj(ip, objv[i + 1], &values[i]) != TCL_OK) return TCL_ERROR;
  }
  return TCL_OK;
}

/* cdept::png file ?path?: write the drawing window, or the window of the Tk widget
 * `path` (a picture pane), as a PNG image. */
static int png_command(ClientData data, Tcl_Interp *ip, int objc, Tcl_Obj *const objv[])
{
  xs_drawable *win;
  cairo_status_t status;
  (void)data;
  if(objc != 2 && objc != 3) {
    Tcl_WrongNumArgs(ip, 1, objv, "file ?path?");
    return TCL_ERROR;
  }
  win = objc == 3 ? xs_window_of_path(Tcl_GetString(objv[2])) : drawing_window();
  if(!win || !win->surface) {
    Tcl_SetResult(ip, "there is no such window", TCL_STATIC);
    return TCL_ERROR;
  }
  cairo_surface_flush(win->surface);
  status = cairo_surface_write_to_png(win->surface, Tcl_GetString(objv[1]));
  if(status != CAIRO_STATUS_SUCCESS) {
    Tcl_SetResult(ip, (char *)cairo_status_to_string(status), TCL_VOLATILE);
    return TCL_ERROR;
  }
  return TCL_OK;
}

/* cdept::colours ?path?: how many different colours the drawing window (or the
 * window of the Tk widget `path`) shows, counted up to 64. Lets tests tell a picture
 * from an empty window. */
static int colours_command(ClientData data, Tcl_Interp *ip, int objc, Tcl_Obj *const objv[])
{
  unsigned int seen[64];
  int nseen = 0, x, y, i, stride;
  const unsigned char *pixels;
  xs_drawable *win;
  (void)data;
  if(objc != 1 && objc != 2) {
    Tcl_WrongNumArgs(ip, 1, objv, "?path?");
    return TCL_ERROR;
  }
  win = objc == 2 ? xs_window_of_path(Tcl_GetString(objv[1])) : drawing_window();
  if(!win || !win->surface) {
    Tcl_SetResult(ip, "there is no such window", TCL_STATIC);
    return TCL_ERROR;
  }
  cairo_surface_flush(win->surface);
  pixels = cairo_image_surface_get_data(win->surface);
  stride = cairo_image_surface_get_stride(win->surface);
  for(y = 0; y < win->height && nseen < 64; ++y) {
    const unsigned int *row = (const unsigned int *)(const void *)(pixels + (size_t)y * (size_t)stride);
    for(x = 0; x < win->width && nseen < 64; ++x) {
      for(i = 0; i < nseen; ++i) if(seen[i] == row[x]) break;
      if(i == nseen) seen[nseen++] = row[x];
    }
  }
  Tcl_SetObjResult(ip, Tcl_NewIntObj(nseen));
  return TCL_OK;
}

/* cdept::window_resize path width height: the window of the Tk widget `path` (a
 * picture pane) has this size now. Its pixels start black; the Tcl side then runs the
 * widget's <Configure> bindings, through which xschem redraws. Returns 1 if there is
 * such a window, 0 if C has not made one (yet). */
static int window_resize_command(ClientData data, Tcl_Interp *ip, int objc,
    Tcl_Obj *const objv[])
{
  int width, height;
  xs_drawable *win;
  (void)data;
  if(objc != 4) {
    Tcl_WrongNumArgs(ip, 1, objv, "path width height");
    return TCL_ERROR;
  }
  if(Tcl_GetIntFromObj(ip, objv[2], &width) != TCL_OK ||
     Tcl_GetIntFromObj(ip, objv[3], &height) != TCL_OK) return TCL_ERROR;
  if(width < 1 || height < 1 || width > 16384 || height > 16384) {
    Tcl_SetResult(ip, "bad window size", TCL_STATIC);
    return TCL_ERROR;
  }
  win = xs_window_of_path(Tcl_GetString(objv[1]));
  if(win && win != drawing_window()) xs_window_resize(win, width, height);
  Tcl_SetObjResult(ip, Tcl_NewIntObj(win != NULL));
  return TCL_OK;
}

/* cdept::previews: the picture panes as a list of {path width height}. */
static int previews_command(ClientData data, Tcl_Interp *ip, int objc, Tcl_Obj *const objv[])
{
  const char *paths[MAX_PREVIEWS];
  xs_drawable *wins[MAX_PREVIEWS];
  Tcl_Obj *result = Tcl_NewListObj(0, NULL);
  int i, n;
  (void)data;
  if(objc != 1) {
    Tcl_WrongNumArgs(ip, 1, objv, "");
    return TCL_ERROR;
  }
  n = has_x ? xs_preview_windows(paths, wins, MAX_PREVIEWS) : 0;
  for(i = 0; i < n; ++i) {
    Tcl_Obj *item = Tcl_NewListObj(0, NULL);
    Tcl_ListObjAppendElement(ip, item, Tcl_NewStringObj(paths[i], -1));
    Tcl_ListObjAppendElement(ip, item, Tcl_NewIntObj(wins[i]->width));
    Tcl_ListObjAppendElement(ip, item, Tcl_NewIntObj(wins[i]->height));
    Tcl_ListObjAppendElement(ip, result, item);
  }
  Tcl_SetObjResult(ip, result);
  return TCL_OK;
}

/* cdept::event type x y keysym button state: input, as cdx_display_event(). */
static int event_command(ClientData data, Tcl_Interp *ip, int objc, Tcl_Obj *const objv[])
{
  long v[6];
  (void)data;
  if(ints_from(ip, objc, objv, v, 6, "type x y keysym button state") != TCL_OK) return TCL_ERROR;
  cdx_display_event((int)v[0], (int)v[1], (int)v[2], (unsigned long)v[3], (int)v[4], (int)v[5]);
  Tcl_ResetResult(ip);
  return TCL_OK;
}

/* cdept::resize width height: as cdx_display_resize(). */
static int resize_command(ClientData data, Tcl_Interp *ip, int objc, Tcl_Obj *const objv[])
{
  long v[2];
  (void)data;
  if(ints_from(ip, objc, objv, v, 2, "width height") != TCL_OK) return TCL_ERROR;
  cdx_display_resize((int)v[0], (int)v[1]);
  Tcl_ResetResult(ip);
  return TCL_OK;
}

/* cdept::damage ?-list?: the changed rectangle as "x y width height" (empty if none),
 * or with -list the changed rectangles as a list of such; and forget them, as
 * cdx_display_take_damage() and cdx_display_take_damage_list(). */
static int damage_command(ClientData data, Tcl_Interp *ip, int objc, Tcl_Obj *const objv[])
{
  int rects[4 * XS_MAX_DAMAGE];
  int i, n, as_list = objc == 2 && !strcmp(Tcl_GetString(objv[1]), "-list");
  Tcl_Obj *result = Tcl_NewListObj(0, NULL);
  (void)data;
  if(objc > 2 || (objc == 2 && !as_list)) {
    Tcl_WrongNumArgs(ip, 1, objv, "?-list?");
    return TCL_ERROR;
  }
  n = cdx_display_take_damage_list(rects, as_list ? XS_MAX_DAMAGE : 1);
  for(i = 0; i < n; ++i) {
    Tcl_Obj *rect = as_list ? Tcl_NewListObj(0, NULL) : result;
    int k;
    for(k = 0; k < 4; ++k) Tcl_ListObjAppendElement(ip, rect, Tcl_NewIntObj(rects[4 * i + k]));
    if(as_list) Tcl_ListObjAppendElement(ip, result, rect);
  }
  Tcl_SetObjResult(ip, result);
  return TCL_OK;
}

/* cdept::colour name: the colour as "#rrggbb", as the stand-in X resolves it ("#f00",
 * "red", "Light Sky Blue", "grey50"). An error for a name it does not know. */
static int colour_command(ClientData data, Tcl_Interp *ip, int objc, Tcl_Obj *const objv[])
{
  XColor exact, screen;
  char text[8];
  (void)data;
  if(objc != 2) {
    Tcl_WrongNumArgs(ip, 1, objv, "name");
    return TCL_ERROR;
  }
  if(!XLookupColor(display, 0, Tcl_GetString(objv[1]), &exact, &screen)) {
    Tcl_AppendResult(ip, "unknown color name \"", Tcl_GetString(objv[1]), "\"", NULL);
    return TCL_ERROR;
  }
  snprintf(text, sizeof(text), "#%06lx", exact.pixel & 0xffffff);
  Tcl_SetObjResult(ip, Tcl_NewStringObj(text, -1));
  return TCL_OK;
}

void xs_register_display_commands(Tcl_Interp *ip)
{
  Tcl_CreateObjCommand(ip, "cdept::png", png_command, NULL, NULL);
  Tcl_CreateObjCommand(ip, "cdept::event", event_command, NULL, NULL);
  Tcl_CreateObjCommand(ip, "cdept::resize", resize_command, NULL, NULL);
  Tcl_CreateObjCommand(ip, "cdept::damage", damage_command, NULL, NULL);
  Tcl_CreateObjCommand(ip, "cdept::colour", colour_command, NULL, NULL);
  Tcl_CreateObjCommand(ip, "cdept::window_resize", window_resize_command, NULL, NULL);
  Tcl_CreateObjCommand(ip, "cdept::previews", previews_command, NULL, NULL);
  Tcl_CreateObjCommand(ip, "cdept::colours", colours_command, NULL, NULL);
}
