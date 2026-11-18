/* xs_mi_compat.h: stands in for the X server's own types, for the X server drawing
 * code copied into this folder (see README.md). A "drawable" is one of the helper's
 * windows or pixmaps, a "graphics context" wraps the helper's GC, and the pixels are
 * written by the functions of ../xs_mi.h.
 *
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#ifndef CDEPT_XS_MI_COMPAT_H
#define CDEPT_XS_MI_COMPAT_H

#include <limits.h>
#include <stdlib.h>
#include <string.h>
#include "xshim.h"
#include "xs_internal.h"

#ifndef TRUE
#define TRUE 1
#define FALSE 0
#endif
#define MAXSHORT SHRT_MAX
#define MINSHORT SHRT_MIN

/* Protocol structures, laid out like XPoint, XRectangle and XArc. */
typedef struct { short x, y; } DDXPointRec, *DDXPointPtr;
typedef DDXPointRec xPoint;
typedef struct { short x, y; unsigned short width, height; } xRectangle;
typedef struct { short x, y; unsigned short width, height; short angle1, angle2; } xArc;

typedef unsigned int CARD32;
typedef unsigned short CARD16;
#define MAXINT INT_MAX
#define MININT INT_MIN
#define HAVE_CBRT 1
typedef struct xs_mi_drawable *DrawablePtr, *PixmapPtr;
typedef struct xs_mi_gc *GCPtr;

/* Only the arc code's detour for raster operations other than "copy" uses a screen,
 * to make a scratch bitmap. xshim.c never sends such arcs here. */
typedef struct xs_mi_screen {
  PixmapPtr (*CreatePixmap)(struct xs_mi_screen *, int width, int height, int depth, int usage);
  void (*DestroyPixmap)(PixmapPtr);
} ScreenRec, *ScreenPtr;

typedef struct xs_mi_drawable {
  short x, y;          /* position on the screen: always 0 here */
  unsigned short width, height;
  unsigned long serialNumber;
  ScreenPtr pScreen;
  xs_drawable *target;
} DrawableRec;

typedef struct {
  void (*FillSpans)(DrawablePtr, GCPtr, int nspans, DDXPointPtr points, int *widths, int sorted);
  void (*PolyFillRect)(DrawablePtr, GCPtr, int nrects, xRectangle *rects);
  void (*PolyPoint)(DrawablePtr, GCPtr, int mode, int npoints, xPoint *points);
  void (*Polylines)(DrawablePtr, GCPtr, int mode, int npoints, DDXPointPtr points);
  void (*PushPixels)(GCPtr, PixmapPtr, DrawablePtr, int width, int height, int x, int y);
} GCOps;

struct xs_mi_gc {
  unsigned char alu;        /* the GX function */
  unsigned short lineWidth;
  int lineStyle, capStyle, joinStyle, fillStyle, fillRule, arcMode;
  unsigned long serialNumber;
  ScreenPtr pScreen;
  unsigned long fgPixel, bgPixel;
  int miTranslate;
  unsigned char *dash;
  unsigned short numInDashList;
  int dashOffset;
  const GCOps *ops;
  GC gc;                    /* the helper's GC: clip, tile, stipple */
};

/* The server changes the foreground this way to paint the gaps of double dashes. */
typedef union { unsigned long val; void *ptr; } ChangeGCVal;
#define NullClient 0
#define ChangeGC(client, pGC, mask, value) ((pGC)->fgPixel = (value)->val)
#define ValidateGC(pDrawable, pGC) ((void)0)

/* glibc declares reallocarray only without _XOPEN_SOURCE, which xschem's build sets. */
static inline void *xs_reallocarray(void *ptr, size_t n, size_t size)
{
  if(size && n > (size_t)-1 / size) return NULL;
  return realloc(ptr, n * size);
}
#define reallocarray xs_reallocarray
#define xallocarray(n, size) xs_reallocarray(NULL, (n), (size))

#define GXcopyInverted 0xc
#define CREATE_PIXMAP_USAGE_SCRATCH 1
#define GetScratchGC(depth, pScreen) ((GCPtr)0)
#define FreeScratchGC(pGC) ((void)0)
#define miClearDrawable(pDraw, pGC) ((void)0)
#ifndef min
#define min(a, b) ((a) < (b) ? (a) : (b))
#define max(a, b) ((a) > (b) ? (a) : (b))
#endif

/* Lines and arcs of width 0 never reach this code: xshim.c draws them itself. */
#define miZeroPolyArc(pDraw, pGC, narcs, parcs) ((void)0)
#define miZeroLine(pDrawable, pGC, mode, npt, pPts) ((void)0)
#define miZeroDashLine(pDrawable, pGC, mode, npt, pPts) ((void)0)

void miStepDash(int dist, int *pDashIndex, unsigned char *pDash, int numInDashList,
                int *pDashOffset);
void miWideLine(DrawablePtr pDrawable, GCPtr pGC, int mode, int npt, DDXPointPtr pPts);
void miWideDash(DrawablePtr pDrawable, GCPtr pGC, int mode, int npt, DDXPointPtr pPts);
void miPolylines(DrawablePtr pDrawable, GCPtr pGC, int mode, int npt, DDXPointPtr pPts);
void miPolyRectangle(DrawablePtr pDraw, GCPtr pGC, int nrects, xRectangle *pRects);
void miPolyFillArc(DrawablePtr pDraw, GCPtr pGC, int narcs, xArc *parcs);
void miPolyArc(DrawablePtr pDraw, GCPtr pGC, int narcs, xArc *parcs);
void miFillPolygon(DrawablePtr dst, GCPtr pgc, int shape, int mode, int count, DDXPointPtr pPts);

#endif /* CDEPT_XS_MI_COMPAT_H */
