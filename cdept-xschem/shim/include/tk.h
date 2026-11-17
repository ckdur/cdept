/* tk.h: stand-in for the Tk C API that xschem uses.
 *
 * The helper has no Tk. These calls answer for the one drawing surface the helper owns.
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#ifndef CDEPT_TK_SHIM_H
#define CDEPT_TK_SHIM_H

#include <tcl.h>
#include "xshim.h"

typedef struct xs_tkwin *Tk_Window;

int Tk_Init(Tcl_Interp *interp);
void Tk_MainLoop(void);
void Tk_Main(int argc, char **argv, Tcl_AppInitProc *init);

Tk_Window Tk_MainWindow(Tcl_Interp *interp);
Tk_Window Tk_NameToWindow(Tcl_Interp *interp, const char *path, Tk_Window main);
void Tk_MakeWindowExist(Tk_Window win);
void Tk_DestroyWindow(Tk_Window win);
Window Tk_WindowId(Tk_Window win);
Display *Tk_Display(Tk_Window win);
Visual *Tk_Visual(Tk_Window win);
Colormap Tk_Colormap(Tk_Window win);
int Tk_Depth(Tk_Window win);
int Tk_ScreenNumber(Tk_Window win);
void Tk_ChangeWindowAttributes(Tk_Window win, unsigned long valuemask,
    XSetWindowAttributes *attributes);

XColor *Tk_GetColor(Tcl_Interp *interp, Tk_Window win, const char *name);
Pixmap Tk_GetPixmap(Display *dpy, Drawable d, int width, int height, int depth);
void Tk_FreePixmap(Display *dpy, Pixmap pixmap);

#endif /* CDEPT_TK_SHIM_H */
