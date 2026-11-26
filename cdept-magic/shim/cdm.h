/* cdm.h: what the Rust side of the Magic helper calls in the C side (cdm.c).
 *
 * Magic itself is not compiled here: the helper loads the `tclmagic.so` of the Magic
 * installation into its own Tcl interpreter, exactly as Magic's `magicexec` does. What
 * this C file adds is the part a real `wish` would provide: a Tk package (the C stub
 * table Magic calls through) and an X display. Both are stand-ins: windows are cairo
 * image surfaces in memory, input arrives from CDePT as calls below, and the Tcl side
 * of Tk is the stand-in of the xschem helper (faketk.tcl).
 *
 * Every function must be called on the thread that called cdm_init().
 *
 * Part of cdept-magic, GPL-2.0-or-later.
 */
#ifndef CDM_H
#define CDM_H

/* Create the Tcl interpreter and the Tk stand-in (package Tk with its stub table).
 * display = 0: no X display (Magic must then run with -dnull). screen_width and
 * screen_height bound the size of the drawing window. Returns a Tcl status code. */
int cdm_init(const char *argv0, int display, int screen_width, int screen_height);

/* Evaluate a script at global level; result or error message via cdm_result(). */
int cdm_eval(const char *script);
const char *cdm_result(void);

/* Event loop (as in the xschem helper). */
void cdm_watch_fd(int fd, void (*readable)(void));
void cdm_when_idle(void (*call)(void));
int cdm_do_one_event(int wait);
/* Call tick() every `ms` milliseconds from the event loop. */
void cdm_every(int ms, void (*tick)(void));

/* `cdept::ui <json>` hands its argument to sink (one UiUpdate). */
void cdm_set_ui_sink(void (*sink)(const char *json));

/* ---- the drawing window: Magic's first layout window ---- */

/* CDePT shows the window at this size. Magic gets ConfigureNotify and Expose. */
void cdm_display_resize(int width, int height);

/* Input in X protocol numbers (libcdept::xschem::input). */
void cdm_display_event(int type, int x, int y, unsigned long keysym, int button, int state);

/* Current size of the drawing window; 0 while there is none. */
int cdm_display_size(int *width, int *height);

/* The rectangle that changed since the last call (found by comparing with the pixels
 * handed out last time); 0 if nothing did. */
int cdm_display_take_damage(int *x, int *y, int *width, int *height);

/* Copy a rectangle of the window as R G B A bytes (alpha 255) to dst, rows of
 * dst_stride bytes; dst points at pixel (x, y) of the destination. */
void cdm_display_read(unsigned char *dst, int dst_stride, int x, int y, int width, int height);

/* Tk path of the drawing window, or NULL. */
const char *cdm_display_path(void);

/* Title Magic gave its window (XStoreName): the cell name. Empty when none. */
const char *cdm_display_title(void);

#endif /* CDM_H */
