/* xs_mi.h: wide lines, dashes, rectangle outlines and filled arcs, turned into pixels
 * by the X server's own code (shim/mi, see its README.md), so that they come out as
 * on a real X server. Included once, at the end of xshim.c.
 *
 * The X server code produces "spans" (runs of pixels in a row); xs_fill_spans() writes
 * them into the image, honouring the GC's clip, function and fill style.
 *
 * Part of cdept-xschem, GPL-2.0-or-later.
 */
#ifndef CDEPT_XS_MI_H
#define CDEPT_XS_MI_H

#include "mi/xs_mi_compat.h"

/* ---- writing pixels ------------------------------------------------------------------- */

/* One X drawing call in progress: where the pixels go and what changed. */
struct xs_paint {
  xs_drawable *dst;
  GC gc;
  uint32_t *pixels;       /* dst's image */
  int stride;             /* in pixels */
  const uint32_t *tile;   /* FillTiled: the tile's image */
  int tile_stride, tile_width, tile_height;
  const xs_drawable *stipple;
  int x1, y1, x2, y2;     /* box of the pixels written (x2, y2 exclusive); empty: x2 <= x1 */
};

static struct xs_paint xs_paint;

static int xs_paint_begin(xs_drawable *dst, GC gc)
{
  struct xs_paint *p = &xs_paint;
  if(!dst || !dst->surface || !gc) return 0;
  if(gc->function == GXnoop || (gc->has_clip && gc->nclip == 0)) return 0;
  memset(p, 0, sizeof(*p));
  p->dst = dst;
  p->gc = gc;
  if(gc->fill_style == FillTiled) {
    xs_drawable *tile = xs_lookup(gc->tile);
    if(!tile || !tile->surface) return 0;
    cairo_surface_flush(tile->surface);
    p->tile = (const uint32_t *)(const void *)cairo_image_surface_get_data(tile->surface);
    p->tile_stride = cairo_image_surface_get_stride(tile->surface) / 4;
    p->tile_width = tile->width;
    p->tile_height = tile->height;
  } else if(gc->fill_style == FillStippled || gc->fill_style == FillOpaqueStippled) {
    p->stipple = xs_lookup(gc->stipple);
    if(!p->stipple || p->stipple->kind != XS_BITMAP || !p->stipple->bits) return 0;
  }
  cairo_surface_flush(dst->surface);
  p->pixels = (uint32_t *)(void *)cairo_image_surface_get_data(dst->surface);
  p->stride = cairo_image_surface_get_stride(dst->surface) / 4;
  return 1;
}

static void xs_paint_end(void)
{
  struct xs_paint *p = &xs_paint;
  if(p->x2 > p->x1) {
    cairo_surface_mark_dirty_rectangle(p->dst->surface, p->x1, p->y1, p->x2 - p->x1,
                                       p->y2 - p->y1);
    xs_damage(p->dst, p->x1, p->y1, p->x2 - p->x1, p->y2 - p->y1);
  }
  p->dst = NULL;
}

/* Pixels x .. x + width - 1 of row y, already inside the drawable and the clip. */
static void xs_put_run(int x, int y, int width, unsigned long pixel)
{
  struct xs_paint *p = &xs_paint;
  GC gc = p->gc;
  uint32_t *row = p->pixels + (size_t)y * (size_t)p->stride + x;
  uint32_t colour = 0xff000000u | (uint32_t)(pixel & 0xffffff);
  int i;

  if(gc->fill_style == FillTiled) {
    /* The tile starts at the drawable's origin and repeats. */
    const uint32_t *from = p->tile + (size_t)(y % p->tile_height) * (size_t)p->tile_stride;
    if(x + width <= p->tile_width) memcpy(row, from + x, (size_t)width * sizeof(*row));
    else for(i = 0; i < width; ++i) row[i] = from[(x + i) % p->tile_width];
  } else if(p->stipple) {
    const xs_drawable *s = p->stipple;
    const unsigned char *bits = s->bits + (size_t)(y % s->height) * (size_t)s->width;
    uint32_t back = 0xff000000u | (uint32_t)(gc->background & 0xffffff);
    for(i = 0; i < width; ++i) {
      if(bits[(x + i) % s->width]) row[i] = colour;
      else if(gc->fill_style == FillOpaqueStippled) row[i] = back;
    }
  } else {
    switch(gc->function) {
      case GXxor:
        for(i = 0; i < width; ++i) row[i] = (row[i] ^ (colour & 0xffffff)) | 0xff000000u;
        break;
      case GXinvert:
        for(i = 0; i < width; ++i) row[i] = (row[i] ^ 0xffffff) | 0xff000000u;
        break;
      case GXclear:
        for(i = 0; i < width; ++i) row[i] = 0xff000000u;
        break;
      case GXset:
        for(i = 0; i < width; ++i) row[i] = 0xffffffffu;
        break;
      default: /* GXcopy; xschem uses no other */
        for(i = 0; i < width; ++i) row[i] = colour;
        break;
    }
  }
  if(p->x2 <= p->x1) {
    p->x1 = x; p->y1 = y; p->x2 = x + width; p->y2 = y + 1;
  } else {
    if(x < p->x1) p->x1 = x;
    if(y < p->y1) p->y1 = y;
    if(x + width > p->x2) p->x2 = x + width;
    if(y + 1 > p->y2) p->y2 = y + 1;
  }
}

/* A run of pixels as the X server code asks for it: clipped, then written. */
static void xs_put_span(int x, int y, int width, unsigned long pixel)
{
  struct xs_paint *p = &xs_paint;
  GC gc = p->gc;
  int x2 = x + width, i;
  if(y < 0 || y >= p->dst->height) return;
  if(x < 0) x = 0;
  if(x2 > p->dst->width) x2 = p->dst->width;
  if(x2 <= x) return;
  if(!gc->has_clip) {
    xs_put_run(x, y, x2 - x, pixel);
    return;
  }
  /* xschem clips to one rectangle. Were there several that overlap, a pixel could be
   * written twice, which only matters for GXxor. */
  for(i = 0; i < gc->nclip; ++i) {
    int cx1 = gc->clip_x + gc->clip[i].x, cy1 = gc->clip_y + gc->clip[i].y;
    int cx2 = cx1 + gc->clip[i].width, cy2 = cy1 + gc->clip[i].height;
    int a = x > cx1 ? x : cx1, b = x2 < cx2 ? x2 : cx2;
    if(y < cy1 || y >= cy2 || b <= a) continue;
    xs_put_run(a, y, b - a, pixel);
  }
}

static void xs_fill_spans(DrawablePtr drawable, GCPtr gc, int nspans, DDXPointPtr points,
    int *widths, int sorted)
{
  int i;
  (void)drawable; (void)sorted;
  for(i = 0; i < nspans; ++i) xs_put_span(points[i].x, points[i].y, widths[i], gc->fgPixel);
}

static void xs_fill_rects(DrawablePtr drawable, GCPtr gc, int nrects, xRectangle *rects)
{
  int i, row;
  (void)drawable;
  for(i = 0; i < nrects; ++i) {
    for(row = 0; row < rects[i].height; ++row) {
      xs_put_span(rects[i].x, rects[i].y + row, rects[i].width, gc->fgPixel);
    }
  }
}

static void xs_poly_point(DrawablePtr drawable, GCPtr gc, int mode, int npoints, xPoint *points)
{
  int i;
  (void)drawable; (void)mode;
  for(i = 0; i < npoints; ++i) xs_put_span(points[i].x, points[i].y, 1, gc->fgPixel);
}

/* ---- the X server code ------------------------------------------------------------------- */

#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-parameter"
#pragma GCC diagnostic ignored "-Wunused-function"
#pragma GCC diagnostic ignored "-Wsign-compare"
#pragma GCC diagnostic ignored "-Wmissing-field-initializers"
#pragma GCC diagnostic ignored "-Wunused-but-set-variable"
#pragma GCC diagnostic ignored "-Wimplicit-fallthrough"
#include "mi/midash.c"
#include "mi/miwideline.c"
#include "mi/mipolyrect.c"
#include "mi/mifillarc.c"
#include "mi/mipoly.c"
/* the arc code has a function and two macros of the same names */
#undef Dsin
#undef Dcos
#define fillSpans xs_miarc_fillSpans
#include "mi/miarc.c"
#undef fillSpans
#pragma GCC diagnostic pop

static void xs_polylines(DrawablePtr drawable, GCPtr gc, int mode, int npoints,
    DDXPointPtr points)
{
  if(gc->lineStyle == LineSolid) miWideLine(drawable, gc, mode, npoints, points);
  else miWideDash(drawable, gc, mode, npoints, points);
}

static const GCOps xs_mi_ops = { xs_fill_spans, xs_fill_rects, xs_poly_point, xs_polylines,
                                 NULL };

/* The X server's view of one of our GCs. `dashes` must have room for
 * 2 * XS_MAX_DASHES bytes. */
static void xs_mi_gc(GCPtr out, GC gc, unsigned char *dashes)
{
  int i, n = gc->ndashes;
  memset(out, 0, sizeof(*out));
  out->alu = (unsigned char)gc->function;
  out->lineWidth = (unsigned short)gc->line_width;
  out->lineStyle = gc->line_style;
  out->capStyle = gc->cap_style;
  out->joinStyle = gc->join_style;
  out->fillStyle = gc->fill_style;
  out->fillRule = gc->fill_rule;
  out->arcMode = ArcPieSlice;
  out->fgPixel = gc->foreground;
  out->bgPixel = gc->background;
  for(i = 0; i < n; ++i) dashes[i] = (unsigned char)(gc->dashes[i] < 1 ? 1 : gc->dashes[i]);
  /* a list of odd length stands for itself twice */
  if(n & 1) {
    for(i = 0; i < n; ++i) dashes[n + i] = dashes[i];
    n *= 2;
  }
  out->dash = dashes;
  out->numInDashList = (unsigned short)n;
  out->dashOffset = gc->dash_offset;
  out->ops = &xs_mi_ops;
  out->gc = gc;
}

static struct xs_mi_drawable xs_mi_drawable(xs_drawable *dst)
{
  struct xs_mi_drawable drawable;
  memset(&drawable, 0, sizeof(drawable));
  drawable.width = (unsigned short)dst->width;
  drawable.height = (unsigned short)dst->height;
  drawable.target = dst;
  return drawable;
}

/* XDrawLines, XDrawSegments (two points each) for lines of width 1 and more. */
static int xs_mi_lines(xs_drawable *dst, GC gc, int mode, int npoints, const XPoint *points)
{
  struct xs_mi_gc mgc;
  struct xs_mi_drawable drawable = xs_mi_drawable(dst);
  unsigned char dashes[2 * XS_MAX_DASHES];
  if(npoints < 1 || !xs_paint_begin(dst, gc)) return 0;
  xs_mi_gc(&mgc, gc, dashes);
  /* XPoint and DDXPointRec are the same two shorts */
  xs_polylines(&drawable, &mgc, mode, npoints, (DDXPointPtr)(void *)points);
  xs_paint_end();
  return 1;
}

/* Like xs_mi_lines() for many two-point lines, each with caps of its own. */
static int xs_mi_segments(xs_drawable *dst, GC gc, const XSegment *segments, int nsegments)
{
  struct xs_mi_gc mgc;
  struct xs_mi_drawable drawable = xs_mi_drawable(dst);
  unsigned char dashes[2 * XS_MAX_DASHES];
  int i;
  if(nsegments < 1 || !xs_paint_begin(dst, gc)) return 0;
  xs_mi_gc(&mgc, gc, dashes);
  for(i = 0; i < nsegments; ++i) {
    DDXPointRec ends[2];
    ends[0].x = segments[i].x1;
    ends[0].y = segments[i].y1;
    ends[1].x = segments[i].x2;
    ends[1].y = segments[i].y2;
    xs_polylines(&drawable, &mgc, CoordModeOrigin, 2, ends);
  }
  xs_paint_end();
  return 1;
}

static int xs_mi_rectangles(xs_drawable *dst, GC gc, const XRectangle *rects, int nrects)
{
  struct xs_mi_gc mgc;
  struct xs_mi_drawable drawable = xs_mi_drawable(dst);
  unsigned char dashes[2 * XS_MAX_DASHES];
  if(nrects < 1 || !xs_paint_begin(dst, gc)) return 0;
  xs_mi_gc(&mgc, gc, dashes);
  miPolyRectangle(&drawable, &mgc, nrects, (xRectangle *)(void *)rects);
  xs_paint_end();
  return 1;
}

static int xs_mi_fill_arcs(xs_drawable *dst, GC gc, const XArc *arcs, int narcs)
{
  struct xs_mi_gc mgc;
  struct xs_mi_drawable drawable = xs_mi_drawable(dst);
  unsigned char dashes[2 * XS_MAX_DASHES];
  if(narcs < 1 || !xs_paint_begin(dst, gc)) return 0;
  xs_mi_gc(&mgc, gc, dashes);
  miPolyFillArc(&drawable, &mgc, narcs, (xArc *)(void *)arcs);
  xs_paint_end();
  return 1;
}

/* Arc outlines of width 1 and more. The X server code changes the arcs it is given. */
static int xs_mi_arcs(xs_drawable *dst, GC gc, const XArc *arcs, int narcs)
{
  struct xs_mi_gc mgc;
  struct xs_mi_drawable drawable;
  unsigned char dashes[2 * XS_MAX_DASHES];
  xArc *copy;
  if(narcs < 1 || !xs_paint_begin(dst, gc)) return 0;
  copy = malloc((size_t)narcs * sizeof(*copy));
  if(copy) {
    memcpy(copy, arcs, (size_t)narcs * sizeof(*copy));
    drawable = xs_mi_drawable(dst);
    xs_mi_gc(&mgc, gc, dashes);
    miPolyArc(&drawable, &mgc, narcs, copy);
    free(copy);
  }
  xs_paint_end();
  return 1;
}

static int xs_mi_fill_polygon(xs_drawable *dst, GC gc, int shape, int mode, int npoints,
    const XPoint *points)
{
  struct xs_mi_gc mgc;
  struct xs_mi_drawable drawable;
  unsigned char dashes[2 * XS_MAX_DASHES];
  DDXPointPtr copy;
  if(npoints < 3 || !xs_paint_begin(dst, gc)) return 0;
  /* the X server code turns relative coordinates into absolute ones in place */
  copy = malloc((size_t)npoints * sizeof(*copy));
  if(copy) {
    memcpy(copy, points, (size_t)npoints * sizeof(*copy));
    drawable = xs_mi_drawable(dst);
    xs_mi_gc(&mgc, gc, dashes);
    miFillPolygon(&drawable, &mgc, shape, mode, npoints, copy);
    free(copy);
  }
  xs_paint_end();
  return 1;
}

/* Lines of width 0 ("thin") that are horizontal or vertical: every pixel from the first
 * point to the second, without the second one for CapNotLast; dashes count pixels from
 * the first point. xschem draws its grid axes this way. Slanted thin lines are left to
 * the caller. */
static void xs_thin_axis_segments(xs_drawable *dst, GC gc, const XSegment *segments,
    int nsegments)
{
  int i, k, pattern = 0, dashed = gc && gc->line_style != LineSolid && gc->ndashes > 0;
  if(nsegments < 1 || !xs_paint_begin(dst, gc)) return;
  for(k = 0; dashed && k < gc->ndashes; ++k) pattern += gc->dashes[k] < 1 ? 1 : (int)gc->dashes[k];
  for(i = 0; i < nsegments; ++i) {
    int x = segments[i].x1, y = segments[i].y1;
    int dx = segments[i].x2 - x, dy = segments[i].y2 - y;
    int length, step_x, step_y, n;
    if(dx != 0 && dy != 0) continue;
    step_x = (dx > 0) - (dx < 0);
    step_y = (dy > 0) - (dy < 0);
    length = abs(dx) + abs(dy) + 1;
    if(gc->cap_style == CapNotLast && length > 1) length--;
    for(n = 0; n < length; ++n, x += step_x, y += step_y) {
      unsigned long pixel = gc->foreground;
      if(dashed) {
        /* which dash this pixel falls in: even ones are drawn */
        int at = (gc->dash_offset + n) % pattern, index = 0;
        while(at >= (gc->dashes[index] < 1 ? 1 : (int)gc->dashes[index])) {
          at -= gc->dashes[index] < 1 ? 1 : (int)gc->dashes[index];
          index++;
        }
        if(index & 1) {
          if(gc->line_style != LineDoubleDash) continue;
          pixel = gc->background;
        }
      }
      xs_put_span(x, y, 1, pixel);
    }
  }
  xs_paint_end();
}

#endif /* CDEPT_XS_MI_H */
