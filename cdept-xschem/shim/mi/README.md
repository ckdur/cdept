# X server drawing code

`miwideline.c`, `miwideline.h`, `mifpoly.h`, `midash.c`, `mipolyrect.c`, `mifillarc.c`
and `mifillarc.h` are copied unmodified from the X.Org X server (`mi/`, release
xorg-server-21.1.8, https://gitlab.freedesktop.org/xorg/xserver). They keep their own
copyright notices and licences (MIT/X11 style, The Open Group, Digital Equipment
Corporation, MIT).

They are what an X server uses to turn wide lines, dashes, rectangles outlines and
filled arcs into pixels. The helper uses the same code so that xschem's picture is the
same, pixel for pixel, as on a real X server.

The files are not compiled on their own: `../xs_mi.h` includes them into `xshim.c`
after `xs_mi_compat.h`, which stands in for the X server's own types (drawable,
graphics context, span filling). The other headers here (`mi.h`, `gcstruct.h`, …) only
include `xs_mi_compat.h`.

To update: replace the copied files with a newer release and build.
