# tkmin.tcl: the least stand-in for Tk that lets xschem start with drawing on.
#
# The helper has no Tk. This file defines the Tk commands as (nearly) empty procs, so
# xschem.tcl can build its widgets, and xschem's C code finds the window it draws in.
# Nothing is shown and no dialog works: use it to test drawing and input only.
# The real stand-in is faketk.tcl; this one is loaded when that file is missing or
# when the environment variable CDEPT_XSCHEM_TK is "tkmin".
#
# Part of cdept-xschem, GPL-2.0-or-later.
#
# ---- What the C side (shim/xshim.c, shim/display.c, xschem's own C) needs from the
# ---- Tcl side. faketk.tcl must provide the same.
#
# Loaded by the stub of Tk_Init, before xschem.tcl is read. Already defined then:
#   ::cdept::drw(width) (height) (mapped) (pointerx) (pointery)
#       the drawing window as C knows it; C updates them on resize and input.
#       `winfo width|height .drw`, `winfo ismapped .|.drw` and `winfo pointerx|y` must
#       answer from these. Before the first resize: 1 x 1, not mapped.
#   ::env(DISPLAY) is not empty (xschem.tcl builds its widgets only then).
#   cdept::png file, cdept::event type x y keysym button state, cdept::resize w h,
#   cdept::damage ?-list?: test commands, see display.c.
#
# Commands xschem's C code calls and that must behave:
#   winfo exists <path>      1 for windows that were created and not destroyed.
#                            Tk_NameToWindow() asks this; ".drw" must exist once
#                            xschem.tcl's build_widgets has run.
#   winfo ismapped .         0 until the first resize, then 1. xschem postpones its
#                            first "zoom full" while this is 0.
#   winfo id <path>          a string like 0x1 (parsed with sscanf "0x%x").
#   winfo width|height <p>   size of windows other than .drw (previews).
#   winfo toplevel <path>, winfo parent <path>
#   <path> configure ...     for ".drw configure -cursor x" and the status bar
#                            widgets (".statusbar.N configure -text ...").
#   tkwait visibility .drw   must return at once.
#   grab current             the window holding a grab, else empty. While a window
#                            other than .drw holds one (a modal dialog), C drops the
#                            input for the drawing window, as Tk would.
#   update, wm, pack, destroy, grab, toplevel, tk_messageBox, ...: any Tk command may
#   be called; none may fail.
#
# Optional hook: if the proc ::cdept::drw_event exists, C calls
#   ::cdept::drw_event type x y keysym button state
# for every input event in the drawing window INSTEAD of calling xschem's callback()
# itself (type, button and state are X11 numbers). Define it to run the event
# bindings recorded from xschem.tcl. Without it C does what those bindings do:
#   xschem callback .drw <type> <x> <y> <keysym> <button> 0 <state>
# with the second press of a double click sent as type -3, `destroy .ctxmenu` on
# Escape, and the Tcl bits of the <Enter> and <Leave> bindings.
# Resizing never goes through Tcl: C calls callback(".drw", ConfigureNotify, ...).

namespace eval tkmin {
  # window path -> class, for windows that exist
  variable windows
  array set windows {. Toplevel}
  # image name -> 1
  variable images
  array set images {}
  variable scaling 1.3333333333333333
}

set tk_version 8.6
set tk_patchLevel 8.6.14
set tk_library {}
set tk_strictMotif 0

# The command of a widget: accepts any subcommand. A few must give an answer of the
# right shape for the code that asks.
proc tkmin::widget_command {path sub args} {
  switch -- $sub {
    cget {return {}}
    index {return 0}
    size {return 0}
    get {return {}}
    curselection {return {}}
    nearest {return 0}
    bbox {return {0 0 1 1}}
    xview - yview {
      if {[llength $args] == 0} {return {0.0 1.0}}
      return {}
    }
    entrycget - itemcget {return {}}
    type {return {}}
    winfo {return {}}
    add {
      # panedwindow/notebook "add" and menu "add": nothing to do
      return {}
    }
    invoke - configure - entryconfigure - delete - insert - select - deselect -
    activate - see - selection - icursor - tag - mark - post - unpost - flash -
    itemconfigure - create - coords - move - raise - lower - scan - set - edit -
    current - state - instate - heading - column - sash - paneconfigure - forget -
    panecget - panes - window - image - search - compare - dlineinfo - count -
    dump - replace - toggle - tk_popup - identify - step - start - stop - tab - tabs -
    hide - item - children - exists - focus - parent - detach - validate - addtag -
    dtag - find - gettags - bind - canvasx - canvasy - postscript - yposition {
      return {}
    }
  }
  return {}
}

proc tkmin::create {class path args} {
  variable windows
  set windows($path) $class
  if {[llength [info commands ::$path]]} {catch {rename ::$path {}}}
  interp alias {} ::$path {} tkmin::widget_command $path
  return $path
}

foreach {command class} {
  toplevel Toplevel frame Frame labelframe Labelframe label Label button Button
  checkbutton Checkbutton radiobutton Radiobutton entry Entry text Text
  listbox Listbox scrollbar Scrollbar scale Scale spinbox Spinbox canvas Canvas
  menu Menu menubutton Menubutton message Message panedwindow Panedwindow
  ttk::frame TFrame ttk::labelframe TLabelframe ttk::label TLabel ttk::button TButton
  ttk::checkbutton TCheckbutton ttk::radiobutton TRadiobutton ttk::entry TEntry
  ttk::combobox TCombobox ttk::scrollbar TScrollbar ttk::scale TScale
  ttk::spinbox TSpinbox ttk::notebook TNotebook ttk::panedwindow TPanedwindow
  ttk::separator TSeparator ttk::progressbar TProgressbar ttk::treeview Treeview
  ttk::menubutton TMenubutton ttk::sizegrip TSizegrip
  tk_optionMenu Menubutton
} {
  namespace eval [namespace qualifiers ::$command] {}
  interp alias {} ::$command {} tkmin::create $class
}

proc destroy {args} {
  foreach path $args {
    foreach name [array names tkmin::windows] {
      if {$name eq $path || [string match $path.* $name] || ($path eq "." && $name ne ".")} {
        if {$name eq "."} continue
        unset tkmin::windows($name)
        catch {rename ::$name {}}
      }
    }
  }
  return {}
}

proc winfo {sub args} {
  set path [lindex $args 0]
  switch -- $sub {
    exists {return [info exists tkmin::windows($path)]}
    ismapped - viewable {
      if {$path eq "." || $path eq ".drw"} {return $::cdept::drw(mapped)}
      return [info exists tkmin::windows($path)]
    }
    width - reqwidth {
      if {$path eq ".drw"} {return $::cdept::drw(width)}
      return 1
    }
    height - reqheight {
      if {$path eq ".drw"} {return $::cdept::drw(height)}
      return 1
    }
    pointerx - x - rootx {
      if {$sub eq "pointerx"} {return $::cdept::drw(pointerx)}
      return 0
    }
    pointery - y - rooty {
      if {$sub eq "pointery"} {return $::cdept::drw(pointery)}
      return 0
    }
    pointerxy {return [list $::cdept::drw(pointerx) $::cdept::drw(pointery)]}
    screenwidth - vrootwidth {return 1920}
    screenheight - vrootheight {return 1080}
    vrootx - vrooty {return 0}
    id {return 0x1}
    toplevel {
      # the nearest ancestor that is a toplevel
      set p $path
      while {$p ne "." && $p ne ""} {
        if {[info exists tkmin::windows($p)] && $tkmin::windows($p) eq "Toplevel"} {return $p}
        set p [winfo parent $p]
      }
      return .
    }
    parent {
      if {$path eq "."} {return {}}
      set cut [string last . $path]
      if {$cut <= 0} {return .}
      return [string range $path 0 [expr {$cut - 1}]]
    }
    children {
      set out {}
      foreach name [array names tkmin::windows] {
        if {$name ne "." && [winfo parent $name] eq $path} {lappend out $name}
      }
      return [lsort $out]
    }
    class {
      if {[info exists tkmin::windows($path)]} {return $tkmin::windows($path)}
      return {}
    }
    name {return [lindex [split $path .] end]}
    geometry {return 1x1+0+0}
    rgb {return {0 0 0}}
    containing {return {}}
    pixels - fpixels {return [expr {int([string trimright [lindex $args 1] cimp])}]}
    screen {return :0.0}
    depth - screendepth {return 24}
    manager {return pack}
    server {return {cdept-xschem}}
    cells {return 256}
    visual - screenvisual {return truecolor}
    atom - atomname - interps - pathname - colormapfull {return {}}
  }
  return 0
}

proc tk {sub args} {
  switch -- $sub {
    scaling {
      if {[llength $args]} {set tkmin::scaling [lindex $args end]}
      return $tkmin::scaling
    }
    windowingsystem {return x11}
    appname {return xschem}
    inactive {return 0}
    useinputmethods {return 0}
    caret {return {}}
    busy {return {}}
    fontchooser {return {}}
  }
  return {}
}

proc font {sub args} {
  switch -- $sub {
    measure {return [expr {8 * [string length [lindex $args end]]}]}
    metrics {
      set values {-ascent 12 -descent 4 -linespace 16 -fixed 0}
      foreach arg $args {
        if {[dict exists $values $arg]} {return [dict get $values $arg]}
      }
      return $values
    }
    actual - configure {
      set values {-family Sans -size 10 -weight normal -slant roman -underline 0 -overstrike 0}
      # one option asked: its value; none: all of them; several: a set, no answer
      if {$sub eq "configure" && [llength $args] > 2} {return {}}
      set option [lindex $args end]
      if {[dict exists $values $option]} {return [dict get $values $option]}
      return $values
    }
    create {
      set name [lindex $args 0]
      if {$name eq "" || [string index $name 0] eq "-"} {set name font[incr tkmin::fonts]}
      return $name
    }
    names {return {TkDefaultFont TkTextFont TkFixedFont TkMenuFont}}
    families {return {Sans Monospace}}
    delete {return {}}
  }
  return {}
}

proc image {sub args} {
  switch -- $sub {
    create {
      # image create photo ?name? ?options?
      set name [lindex $args 1]
      if {$name eq "" || [string index $name 0] eq "-"} {set name image[incr tkmin::imageid]}
      set tkmin::images($name) 1
      interp alias {} ::$name {} tkmin::widget_command $name
      return $name
    }
    names {return [array names tkmin::images]}
    width - height {return 16}
    delete {
      foreach name $args {
        unset -nocomplain tkmin::images($name)
        catch {rename ::$name {}}
      }
      return {}
    }
    types {return {photo bitmap}}
    type {return photo}
    inuse {return 0}
  }
  return {}
}

proc wm {sub args} {
  switch -- $sub {
    geometry {return 800x600+0+0}
    state {return normal}
    title - iconname {
      if {[llength $args] > 1} {return {}}
      return {}
    }
    attributes {return {}}
    overrideredirect - resizable {return {}}
    stackorder {return .}
    frame {return 0x1}
  }
  return {}
}

proc tkwait {args} {return {}}
proc focus {args} {return {}}
proc grab {args} {return {}}
proc bind {args} {return {}}
proc bindtags {args} {return {}}
proc pack {args} {return {}}
proc grid {args} {return {}}
proc place {args} {return {}}
proc raise {args} {return {}}
proc lower {args} {return {}}
proc option {args} {return {}}
proc event {args} {return {}}
proc bell {args} {return {}}
proc selection {args} {return {}}
proc clipboard {args} {return {}}
proc send {args} {return {}}
proc tk_focusNext {args} {return .}
proc tk_focusPrev {args} {return .}
proc tk_focusFollowsMouse {args} {return {}}
proc tk_setPalette {args} {return {}}
proc tk_bisque {args} {return {}}
proc tk_popup {args} {return {}}
proc tk_menuSetFocus {args} {return {}}
proc tk_textCopy {args} {return {}}
proc tk_textCut {args} {return {}}
proc tk_textPaste {args} {return {}}
proc tk_chooseColor {args} {return {}}
proc tk_chooseDirectory {args} {return {}}
proc tk_getOpenFile {args} {return {}}
proc tk_getSaveFile {args} {return {}}
proc tk_dialog {args} {return 0}
# A message box cannot be shown: say what it said, and answer "ok".
proc tk_messageBox {args} {
  if {[dict exists $args -message]} {
    puts stderr "cdept-xschem (no dialogs with tkmin): [dict get $args -message]"
  }
  return ok
}
namespace eval ttk {
  proc style {args} {return {}}
  proc setTheme {args} {return {}}
}
namespace eval tk {
  proc unsupported1 {args} {return {}}
}

# "." takes widget subcommands too.
interp alias {} ::. {} tkmin::widget_command .
