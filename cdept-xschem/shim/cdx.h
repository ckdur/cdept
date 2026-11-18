/* cdx.h: what the Rust side of the helper calls in the C side.
 *
 * engine.c implements start-up, Tcl access and the event loop; display.c implements the
 * drawing window. xschem and its Tcl interpreter are single-threaded: every function
 * here must be called on the thread that called cdx_init().
 *
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#ifndef CDX_H
#define CDX_H

/* ---- start-up and Tcl (engine.c) ---- */

/* Start xschem. `argv` uses xschem's own command line options; argv[0] is the program
 * name. display = 0: no drawing (as `xschem --no_x`). display = 1: xschem draws into
 * the drawing window below, through the stand-in X and Tk.
 * Returns a Tcl status code (TCL_OK = 0); on failure cdx_result() has the message. */
int cdx_init(int argc, char **argv, int display);

/* Evaluate a script at global level. Returns the Tcl status code; the result or the
 * error message is read with cdx_result(), valid until the next call into Tcl. */
int cdx_eval(const char *script);
const char *cdx_result(void);

const char *cdx_xschem_version(void);

/* ---- event loop (engine.c) ---- */

/* Call readable() from the Tcl event loop whenever `fd` has data to read. This also
 * holds inside nested loops (vwait, tkwait): a dialog that xschem waits for is
 * answered through the same socket. */
void cdx_watch_fd(int fd, void (*readable)(void));

/* Call call() once from the Tcl event loop when it has nothing else to do; like the
 * file watch, also from nested loops. */
void cdx_when_idle(void (*call)(void));

/* Run one round of the Tcl event loop: timers, idle callbacks, watched files.
 * wait != 0 blocks until there is something to do. Returns nonzero if work was done. */
int cdx_do_one_event(int wait);

/* The Tcl command `cdept::ui <json>` hands its argument to `sink`: one widget update
 * (UiUpdate in libcdept::xschem::protocol), written by shim/tcl/faketk.tcl. */
void cdx_set_ui_sink(void (*sink)(const char *json));

/* ---- drawing window (display.c) ---- */

/* The window has this size now, in pixels. xschem is told (ConfigureNotify, Expose)
 * and redraws. The first call creates the window. */
void cdx_display_resize(int width, int height);

/* Pointer or keyboard input, numbered as in the X protocol (see
 * libcdept::xschem::input): event type, pointer position, key symbol, button, and the
 * modifier/button mask from before the event. */
void cdx_display_event(int type, int x, int y, unsigned long keysym, int button, int state);

/* Current size. Returns 0 while there is no window. */
int cdx_display_size(int *width, int *height);

/* The rectangle that changed since the last call, if any (returns 0 if nothing did).
 * Calling this clears it. */
int cdx_display_take_damage(int *x, int *y, int *width, int *height);

/* The same as several rectangles: fills `rects` with up to `max` groups of x, y, width,
 * height that together cover what changed, and returns how many (0 if nothing did).
 * Cheaper for the caller when the changes are far apart: a cross-hair cursor is two
 * thin strips, not the whole window. Calling this clears the damage, too. */
int cdx_display_take_damage_list(int *rects, int max);

/* Copy a rectangle of the window to `dst`: rows of dst_stride bytes, pixels as
 * R G B A bytes with premultiplied alpha. dst points at the pixel (x, y) of the
 * destination, i.e. the caller has already added the offset. The rectangle must lie
 * inside the window. */
void cdx_display_read(unsigned char *dst, int dst_stride, int x, int y, int width, int height);

#endif /* CDX_H */
