# faketk.tcl: a stand-in for Tk, written in plain Tcl 8.6, for the xschem helper.
#
# The helper has no Tk and no display server. xschem's user interface code (xschem.tcl
# and the Tcl that xschem's C code evaluates) runs unmodified against this file: the Tk
# commands exist, they record the widgets that get created and their state, and report
# them to CDePT, which draws them with its own widgets. See docs/XSCHEM.md section 8
# and libcdept/src/xschem/protocol.rs (UiUpdate, UiEvent).
#
# Implemented (state is kept and reported):
#   widgets   toplevel frame labelframe label message button checkbutton radiobutton
#             entry spinbox text listbox scale scrollbar canvas panedwindow menubutton
#             menu, ttk::combobox and the other ttk:: widget commands (as their classic
#             counterparts), each with configure/cget and its own subcommands
#             (entry/text/listbox contents and selection, text marks and tags, menu
#             entries, invoke/select/deselect/toggle, ...)
#   layout    pack grid place (recorded, with order; no geometry is computed)
#   others    bind bindtags event wm winfo focus grab tkwait update vwait destroy
#             image (photo data is kept) clipboard selection option font tk
#             tk_messageBox tk_getOpenFile tk_getSaveFile tk_chooseDirectory
#             tk_chooseColor tk_popup tk_dialog
# Only stubbed (accepted, nothing happens): raise lower bell send tk_setPalette
#   tk_focusNext/Prev tk_focusFollowsMouse ttk::style, scrolling (xview/yview/see),
#   canvas items, fonts metrics (fixed numbers), winfo sizes of anything but the
#   drawing window (estimates), undo in text widgets.
#
# To CDePT: `cdept::ui <json>`, one UiUpdate per call, sent from an idle handler (and
# before every wait) for whatever changed:
#   menu       menu bar of `.` (. configure -menu), with the state of check/radio entries
#   images     photo images, before the first widget that uses them
#   toolbar    the buttons packed in `.toolbar` (tooltip = text given to `balloon`)
#   status     the labels and entries packed in `.statusbar`; the entries (SNAP, GRID)
#              are `editable`: event text <path> <value>, then key <path> Return
#   title      wm title .
#   cursor     cursor of `.drw` (or of `.` when .drw has none)
#   window     every other toplevel that is not withdrawn: the whole widget tree
#   window_closed
#   popup      tk_popup / menu post, and toplevels with `wm overrideredirect` that hold
#              only buttons (xschem's context menu); the entries are reported with
#              `menu` = the menu or toplevel path and `index` = position
#   clipboard  clipboard append
#   log        text added to xschem's info window (.infotext), and errors raised by
#              callbacks (what Tk would show in its error dialog)
# What the user changes in a window (event text, select, focus) is not sent back; only
# what xschem's code changes in response is.
# xschem's own tabs (.tabs), when there are several, are reported as text buttons at
# the end of the tool bar, the current one `checked`.
# Toplevels named *.balloon (xschem's tool tips) are never reported.
#
# From CDePT: `faketk::event <kind> <args>`:
#   menu <menu path> <index>      popup_closed            invoke <path>
#   text <path> <text>            select <path> <indices> activate <path> <index>
#   key <path> <keysym>           focus <path>            close <toplevel>
#   (keysym may carry modifiers: Control-Return, Shift-Tab, ...)
# Notes for the sender:
#   - key: runs the KeyPress and then the KeyRelease bindings of the widget, its
#     class, its toplevel and `all`. Send the keys that can have a binding (Return,
#     Escape, Tab, arrows, and letters when the focus is not in a text field).
#   - a key typed into a Text widget: send `text` with the content as the key left it
#     (Return has added its newline), then `key`. xschem's bindings count on that.
#   - select is a click on a list line: it also runs the widget's button bindings
#     (xschem's file dialog enters a folder on a single click). activate is the
#     second click of a double click and only runs the double click binding.
#   - menu with a toplevel path chooses the index-th button of a popup made of
#     buttons; popup_closed destroys such a toplevel.
#
# Special windows (class of the root widget):
#   FileDialog   tk_getOpenFile/tk_getSaveFile/tk_chooseDirectory. options: mode
#                (open save directory) title initialdir initialfile filetypes
#                defaultextension multiple. Answer: event text <window> <path or {}>.
#   MessageBox   tk_messageBox. options: icon type message detail default title; the
#                children are its buttons. Answer: event invoke <button>.
#   ColorDialog  tk_chooseColor: an entry with the colour plus OK/Cancel buttons.
#
#   LibraryPicker  CDePT's library / cell / view picker, in place of xschem's file
#                browsers: see cdept.tcl, which is read after xschem.tcl and also
#                turns "open in a new window or tab" into `open` reports.
#
# Menu entries carry `image` (an image name) and `color` (xschem colours the Netlist,
# Simulate and Waves entries to show a state, and the entries of the Layers menu).
#
# While xschem starts (::cdept::starting, set by the C side) nobody can answer: Tk's
# own dialogs then put their message in the log and return their default answer, and
# any other window that xschem waits for is logged and closed unanswered.
#
# With the C side (see the head of tkmin.tcl for what it expects):
#   ::cdept::drw(width|height|mapped|pointerx|pointery), kept by C, answer
#     `winfo width|height .drw`, `winfo ismapped .`, `winfo pointerx|y`
#   faketk::drawing_size w h        the same by hand, when C does not provide them
#   faketk::pointer x y
#   faketk::dispatch path seq dict  run the bindings of an event (see there); not
#                                   used for the drawing window: C calls xschem's
#                                   callback() itself (no ::cdept::drw_event here)
#   faketk::window_id path          the value `winfo id` returns ("0x..")
#   cdept::colour name              a colour as #rrggbb, with all X colour names
#                                   (C side, drawing mode); used by `winfo rgb`
#   faketk::flush                   send pending updates now
#
# Debugging: set the environment variable CDEPT_FAKETK_DEBUG to get a line on stderr
# for every Tk feature used that is not implemented.
#
# Part of cdept-xschem, GPL-2.0-or-later.

namespace eval faketk {
  variable debug [info exists ::env(CDEPT_FAKETK_DEBUG)]
  variable class            ;# path -> Tk class
  variable kids             ;# path -> children, in creation order
  variable opt              ;# path -> dict of options (canonical names, with dash)
  variable lay              ;# path -> dict: manager + its options, without dash
  variable slaves           ;# master -> slaves in stacking order
  variable content          ;# entry/text/spinbox/combobox/scale content
  variable items            ;# listbox lines (when there is no -listvariable)
  variable sel              ;# listbox: selected indices; entry: {first last}
  variable binds            ;# tag -> dict: sequence -> script
  variable btags            ;# path -> binding tags, when set explicitly
  variable menu             ;# menu path -> list of entries (dicts: type + options)
  variable wmi              ;# "path,what" -> value (title, state, protocol,...)
  variable img              ;# image name -> dict: type format data
  variable imgsent          ;# image name -> data already reported
  variable marks            ;# "path,mark" -> offset (text widgets)
  variable tags             ;# "path,tag" -> list of offsets: start end start end...
  variable tagopt           ;# "path,tag" -> options
  variable vtraces          ;# path -> variables traced for it
  variable mtraced          ;# variable -> 1: traced for the menus
  variable waits            ;# toplevel -> number of tkwait calls pending on it
  variable waitvars         ;# path -> variables to set when it is destroyed
  variable dirty            ;# what changed since the last flush
  variable last             ;# what -> json last sent
  variable shown            ;# toplevel -> window|popup: reported and not closed
  variable warned           ;# feature -> 1: already logged
  variable spec             ;# "window,result|done": Tk's own dialogs being waited for
  variable focus .
  variable grabbed {}
  variable pending 0
  variable flushing 0
  variable serial 0
  variable drawing {800 600}
  variable pointer {0 0}
  variable popup {}         ;# path of the menu or toplevel shown as popup
  variable logtext {}       ;# text for a `log` report
  variable infotext {}      ;# what the info window held at the last report
  variable clip {}
  variable primary {}
  variable options          ;# option database: pattern -> value
  variable fonts            ;# font name -> options
  array set class {}; array set kids {}; array set opt {}; array set lay {}
  array set slaves {}; array set content {}; array set items {}; array set sel {}
  array set binds {}; array set btags {}; array set menu {}; array set wmi {}
  array set img {}; array set imgsent {}; array set marks {}; array set tags {}
  array set tagopt {}; array set vtraces {}; array set mtraced {}; array set waits {}
  array set waitvars {}; array set dirty {}; array set last {}; array set shown {}
  array set warned {}; array set options {}; array set fonts {}; array set spec {}
}

# ---------------------------------------------------------------------------------
# Small helpers
# ---------------------------------------------------------------------------------

# Log once per feature, in debug mode.
proc faketk::todo {what} {
  variable debug; variable warned
  if {!$debug || [info exists warned($what)]} return
  set warned($what) 1
  puts stderr "faketk: not implemented: $what"
}

proc faketk::dbg {text} {
  variable debug
  if {$debug} {puts stderr "faketk: $text"}
}

proc faketk::exists {w} {
  variable class
  info exists class($w)
}

proc faketk::need {w} {
  variable class
  if {![info exists class($w)]} {
    return -code error "bad window path name \"$w\""
  }
}

proc faketk::parent {w} {
  if {$w eq "."} {return {}}
  set i [string last . $w]
  if {$i <= 0} {return .}
  string range $w 0 [expr {$i - 1}]
}

# The toplevel a widget is in.
proc faketk::top {w} {
  variable class
  while {$w ne "" && $w ne "."} {
    if {[info exists class($w)] && $class($w) eq "Toplevel"} {return $w}
    set w [parent $w]
  }
  return .
}

# Global variable name, as a fully qualified name usable from this namespace.
proc faketk::gv {name} {
  if {[string match ::* $name]} {return $name}
  return ::$name
}

proc faketk::vget {name {default {}}} {
  set n [gv $name]
  if {[info exists $n] && ![array exists $n]} {return [set $n]}
  return $default
}

proc faketk::vset {name value} {
  set [gv $name] $value
}

# Run a script at global level, as Tk does for commands and bindings. Errors are
# reported like Tk's background errors: logged, not raised.
proc faketk::run {script} {
  if {[string trim $script] eq ""} return
  set code [catch {uplevel #0 $script} result]
  if {$code == 1} {problem $result}
  return $code
}

# An error in a callback or binding. Tk would show a dialog (bgerror); here it goes to
# the log of the helper and, as a `log` report, to CDePT.
proc faketk::problem {message} {
  variable logtext
  if {[llength [info commands ::bgerror]]} {
    if {![catch {::bgerror $message}]} return
  }
  puts stderr "faketk: error in callback: $message"
  append logtext "Error: [lindex [split $message \n] 0]\n"
  mark log
}

# ---------------------------------------------------------------------------------
# JSON
# ---------------------------------------------------------------------------------

namespace eval faketk {
  variable jmap {}
  for {set i 0} {$i < 32} {incr i} {
    lappend jmap [format %c $i] [format {\u%04x} $i]
  }
  # after the generic control codes, so these win (string map takes the first match
  # in list order for equal positions: put the specific ones first)
  set jmap [linsert $jmap 0 \\ \\\\ \" \\\" \n \\n \t \\t \r \\r]
  lappend jmap \x7f {\u007f}
}

proc faketk::jstr {s} {
  variable jmap
  return "\"[string map $jmap $s]\""
}

proc faketk::jbool {b} {
  expr {$b ? "true" : "false"}
}

# {key jsonvalue key jsonvalue ...}
proc faketk::jobj {pairs} {
  set out {}
  foreach {k v} $pairs {lappend out "[jstr $k]:$v"}
  return "{[join $out ,]}"
}

proc faketk::jarr {values} {
  return "\[[join $values ,]\]"
}

# A dict of strings.
proc faketk::jmapobj {d} {
  set out {}
  dict for {k v} $d {lappend out "[jstr $k]:[jstr $v]"}
  return "{[join $out ,]}"
}

proc faketk::emit {json} {
  if {[llength [info commands ::cdept::ui]]} {
    if {[catch {::cdept::ui $json} err]} {dbg "cdept::ui: $err"}
  }
}

# ---------------------------------------------------------------------------------
# What changed: collected here, sent by flush
# ---------------------------------------------------------------------------------

proc faketk::schedule {} {
  variable pending
  if {!$pending} {
    set pending 1
    after idle ::faketk::flush
  }
}

proc faketk::mark {what} {
  variable dirty
  set dirty($what) 1
  schedule
}

# A widget changed: find out which report it belongs to.
proc faketk::touch {w} {
  variable class
  if {![info exists class($w)]} return
  if {$class($w) eq "Menu"} {mark menu; return}
  set t [top $w]
  if {$t ne "."} {
    mark win,$t
    if {$w eq ".infotext.f1.text"} {mark log}
    return
  }
  if {$w eq "."} {mark cursor; return}
  switch -glob -- $w {
    .toolbar* - .tabs* {mark toolbar}
    .statusbar* {mark status}
    .drw {mark cursor}
    default {}
  }
}

# ---------------------------------------------------------------------------------
# Options
# ---------------------------------------------------------------------------------

namespace eval faketk {
  # short names and abbreviations Tk accepts
  variable alias
  array set alias {
    -bd -borderwidth -bg -background -fg -foreground -vcmd -validatecommand
    -invcmd -invalidcommand
  }
  # defaults by class; anything else defaults to the empty string
  variable defaults
  array set defaults {
    * {-state normal -text {} -borderwidth 0 -relief flat -font TkDefaultFont
       -background #d9d9d9 -foreground black -cursor {} -takefocus 0 -width 0
       -height 0 -padx 1 -pady 1 -anchor center -justify left -image {} -compound none
       -highlightthickness 0 -textvariable {} -command {}}
    Toplevel {-menu {} -class Toplevel}
    Menu {-tearoff 1 -type normal -postcommand {}}
    Checkbutton {-onvalue 1 -offvalue 0 -indicatoron 1 -variable {}}
    Radiobutton {-value {} -indicatoron 1 -variable selectedButton}
    Entry {-show {} -exportselection 1 -validate none -width 20 -background white}
    Spinbox {-from 0 -to 0 -increment 1 -values {} -wrap 0 -width 20 -format {}}
    TCombobox {-values {} -width 20 -exportselection 1}
    Text {-wrap char -undo 0 -autoseparators 1 -width 80 -height 24 -tabs {}
          -exportselection 1 -background white -insertbackground black}
    Listbox {-selectmode browse -exportselection 1 -listvariable {} -width 20
             -height 10 -activestyle dotbox -background white}
    Scale {-from 0 -to 100 -resolution 1 -orient vertical -variable {} -digits 0
           -label {} -length 100 -showvalue 1}
    Scrollbar {-orient vertical}
    Panedwindow {-orient horizontal}
    Canvas {-background #d9d9d9}
    Label {}
  }
}

proc faketk::optname {o} {
  variable alias
  if {[info exists alias($o)]} {return $alias($o)}
  return $o
}

proc faketk::default {cls o} {
  variable defaults
  if {[info exists defaults($cls)] && [dict exists $defaults($cls) $o]} {
    return [dict get $defaults($cls) $o]
  }
  if {[dict exists $defaults(*) $o]} {return [dict get $defaults(*) $o]}
  return {}
}

proc faketk::cget {w o} {
  variable opt; variable class
  set o [optname $o]
  if {[dict exists $opt($w) $o]} {return [dict get $opt($w) $o]}
  return [default $class($w) $o]
}

# Options that link the widget to a global variable.
proc faketk::varopts {} {return {-textvariable -variable -listvariable}}

proc faketk::vtrace {w args} {
  catch {::faketk::touch $w}
}

# (Re)install the variable traces of a widget.
proc faketk::link {w} {
  variable vtraces; variable class
  if {[info exists vtraces($w)]} {
    foreach n $vtraces($w) {
      catch {trace remove variable $n write [list ::faketk::vtrace $w]}
    }
  }
  set vtraces($w) {}
  foreach o [varopts] {
    set v [cget $w $o]
    if {$v eq ""} continue
    if {$o eq "-variable" && $class($w) ni {Checkbutton Radiobutton Scale}} continue
    set n [gv $v]
    if {[catch {trace add variable $n write [list ::faketk::vtrace $w]}]} continue
    lappend vtraces($w) $n
  }
}

proc faketk::configure {w arglist} {
  variable opt; variable class
  set n [llength $arglist]
  if {$n == 0} {
    set out {}
    dict for {o v} $opt($w) {
      lappend out [list $o [string range $o 1 end] [string totitle [string range $o 1 end]] \
                       [default $class($w) $o] $v]
    }
    return $out
  }
  if {$n == 1} {
    set o [optname [lindex $arglist 0]]
    return [list $o [string range $o 1 end] [string totitle [string range $o 1 end]] \
                [default $class($w) $o] [cget $w $o]]
  }
  if {$n % 2} {
    return -code error "value for \"[lindex $arglist end]\" missing"
  }
  set relink 0
  foreach {o v} $arglist {
    set o [optname $o]
    dict set opt($w) $o $v
    if {$o in [varopts]} {set relink 1}
    switch -- $o {
      -textvariable {
        # Tk creates the variable from what the widget holds.
        if {$v ne "" && ![info exists [gv $v]]} {
          catch {vset $v [text_of $w]}
        }
      }
      -text {
        # a label showing a variable ignores -text
      }
      -menu {
        if {$class($w) eq "Toplevel"} {mark menu}
      }
    }
  }
  if {$relink} {link $w}
  touch $w
  return {}
}

# The text a widget shows: label text, or the content of an entry-like widget.
proc faketk::text_of {w} {
  variable content; variable class
  set v [cget $w -textvariable]
  switch -- $class($w) {
    Entry - Spinbox - TCombobox - TEntry {
      if {$v ne ""} {return [vget $v]}
      return $content($w)
    }
    Text {return $content($w)}
    Scale {
      set sv [cget $w -variable]
      if {$sv ne ""} {return [vget $sv [cget $w -from]]}
      return $content($w)
    }
    default {
      if {$v ne ""} {return [vget $v]}
      return [cget $w -text]
    }
  }
}

proc faketk::set_text {w value} {
  variable content; variable class
  set v [cget $w -textvariable]
  if {$class($w) eq "Scale"} {set v [cget $w -variable]}
  if {$v ne "" && $class($w) ne "Text"} {
    vset $v $value
  } else {
    set content($w) $value
  }
  touch $w
}

# ---------------------------------------------------------------------------------
# Creating and destroying widgets
# ---------------------------------------------------------------------------------

proc faketk::create {cls w arglist} {
  variable class; variable kids; variable opt; variable lay; variable content
  variable items; variable sel; variable wmi
  if {![string match .* $w]} {
    return -code error "bad window path name \"$w\""
  }
  if {[info exists class($w)]} {
    set p [parent $w]
    return -code error "window name \"[string range $w [expr {[string last . $w] + 1}] end]\"\
      already exists in parent"
  }
  set p [parent $w]
  if {![info exists class($p)]} {
    return -code error "bad window path name \"$p\""
  }
  set class($w) $cls
  set kids($w) {}
  lappend kids($p) $w
  set opt($w) {}
  set lay($w) {}
  set content($w) {}
  set items($w) {}
  set sel($w) {}
  if {$cls eq "Toplevel"} {
    set wmi($w,title) [string range $w [expr {[string last . $w] + 1}] end]
    set wmi($w,state) normal
  }
  if {$cls eq "Checkbutton"} {
    # Tk's default variable is named after the widget
    dict set opt($w) -variable [string range $w [expr {[string last . $w] + 1}] end]
  }
  interp alias {} $w {} ::faketk::w $w
  if {[catch {configure $w $arglist} err]} {
    destroy_one $w
    return -code error $err
  }
  if {$cls eq "Scale"} {set content($w) [cget $w -from]}
  link $w
  touch $w
  return $w
}

proc faketk::destroy_one {w} {
  variable class; variable kids; variable opt; variable lay; variable slaves
  variable content; variable items; variable sel; variable binds; variable btags
  variable menu; variable wmi; variable vtraces; variable waits; variable waitvars
  variable focus; variable grabbed; variable shown; variable popup
  variable marks; variable tags; variable tagopt
  if {![info exists class($w)]} return
  foreach k $kids($w) {destroy_one $k}
  # <Destroy> bindings set on the widget itself
  set script [binding $w <Destroy>]
  if {$script ne ""} {run [subst_event $script [dict create W $w T 17]]}
  set cls $class($w)
  set t [top $w]
  if {[info exists vtraces($w)]} {
    foreach n $vtraces($w) {
      catch {trace remove variable $n write [list ::faketk::vtrace $w]}
    }
  }
  forget $w
  set p [parent $w]
  if {[info exists kids($p)]} {
    set i [lsearch -exact $kids($p) $w]
    if {$i >= 0} {set kids($p) [lreplace $kids($p) $i $i]}
  }
  foreach a {class kids opt lay slaves content items sel binds btags menu vtraces waits} {
    unset -nocomplain ${a}($w)
  }
  array unset wmi $w,*
  array unset marks $w,*
  array unset tags $w,*
  array unset tagopt $w,*
  catch {interp alias {} $w {}}
  if {$focus eq $w} {set focus [expr {$t eq $w ? "." : $t}]}
  if {$grabbed eq $w} {set grabbed {}}
  if {$popup eq $w} {set popup {}}
  if {$cls eq "Toplevel"} {
    mark win,$w
  } elseif {$cls eq "Menu"} {
    mark menu
  } else {
    if {$t eq "."} {
      switch -glob -- $w {
        .toolbar* - .tabs* {mark toolbar}
        .statusbar* {mark status}
      }
    } else {
      mark win,$t
    }
  }
  if {[info exists waitvars($w)]} {
    set vars $waitvars($w)
    unset waitvars($w)
    foreach v $vars {set $v 1}
  }
  if {[info exists ::faketk::spec($w,done)] && !$::faketk::spec($w,done)} {
    set ::faketk::spec($w,done) 1
  }
}

proc ::destroy {args} {
  foreach w $args {
    if {$w eq "."} {
      # Tk ends the application: xschem does this only when quitting.
      foreach k $::faketk::kids(.) {::faketk::destroy_one $k}
      continue
    }
    ::faketk::destroy_one $w
  }
  return {}
}

# ---------------------------------------------------------------------------------
# Geometry managers: recorded only
# ---------------------------------------------------------------------------------

# Take a widget out of its manager.
proc faketk::forget {w} {
  variable lay; variable slaves
  if {![info exists lay($w)] || $lay($w) eq ""} return
  set m [expr {[dict exists $lay($w) in] ? [dict get $lay($w) in] : [parent $w]}]
  if {[info exists slaves($m)]} {
    set i [lsearch -exact $slaves($m) $w]
    if {$i >= 0} {set slaves($m) [lreplace $slaves($m) $i $i]}
  }
  set lay($w) {}
  touch $w
  if {[exists $m]} {touch $m}
}

# Split "w w w -opt val -opt val" into widgets and options.
proc faketk::split_geometry_args {arglist} {
  set ws {}
  set i 0
  foreach a $arglist {
    if {[string match -?* $a] && ![string is double -strict $a] && $a ne "-"} break
    lappend ws $a
    incr i
  }
  return [list $ws [lrange $arglist $i end]]
}

proc faketk::manage {manager w options} {
  variable lay; variable slaves
  need $w
  set old $lay($w)
  if {$old ne "" && [dict get $old manager] ne $manager} {
    forget $w
    set old {}
  }
  set d $old
  dict set d manager $manager
  foreach {o v} $options {
    dict set d [string range $o 1 end] $v
  }
  set m [expr {[dict exists $d in] ? [dict get $d in] : [parent $w]}]
  if {$old ne ""} {
    set om [expr {[dict exists $old in] ? [dict get $old in] : [parent $w]}]
    if {$om ne $m && [info exists slaves($om)]} {
      set i [lsearch -exact $slaves($om) $w]
      if {$i >= 0} {set slaves($om) [lreplace $slaves($om) $i $i]}
    }
  }
  if {![info exists slaves($m)]} {set slaves($m) {}}
  set i [lsearch -exact $slaves($m) $w]
  set before [expr {[dict exists $d before] ? [dict get $d before] : ""}]
  set after [expr {[dict exists $d after] ? [dict get $d after] : ""}]
  dict unset d before
  dict unset d after
  if {$before ne "" || $after ne ""} {
    if {$i >= 0} {set slaves($m) [lreplace $slaves($m) $i $i]}
    set ref [expr {$before ne "" ? $before : $after}]
    set j [lsearch -exact $slaves($m) $ref]
    if {$j < 0} {
      lappend slaves($m) $w
    } else {
      if {$after ne ""} {incr j}
      set slaves($m) [linsert $slaves($m) $j $w]
    }
  } elseif {$i < 0} {
    lappend slaves($m) $w
  }
  set lay($w) $d
  touch $w
  if {[exists $m]} {touch $m}
}

proc faketk::geometry_info {w manager} {
  variable lay
  need $w
  if {$lay($w) eq "" || [dict get $lay($w) manager] ne $manager} {
    if {$manager eq "pack"} {
      return -code error "window \"$w\" isn't packed"
    }
    return {}
  }
  set out {}
  if {![dict exists $lay($w) in]} {lappend out -in [parent $w]}
  dict for {k v} $lay($w) {
    if {$k eq "manager"} continue
    lappend out -$k $v
  }
  return $out
}

proc faketk::slaves_of {m manager} {
  variable slaves; variable lay
  set out {}
  if {[info exists slaves($m)]} {
    foreach s $slaves($m) {
      if {[dict get $lay($s) manager] eq $manager} {lappend out $s}
    }
  }
  return $out
}

proc ::pack {args} {
  set sub [lindex $args 0]
  switch -- $sub {
    configure {return [::faketk::pack_do [lrange $args 1 end]]}
    forget {foreach w [lrange $args 1 end] {if {[::faketk::exists $w]} {::faketk::forget $w}}; return {}}
    info {return [::faketk::geometry_info [lindex $args 1] pack]}
    propagate {return 1}
    slaves - content {return [::faketk::slaves_of [lindex $args 1] pack]}
    default {return [::faketk::pack_do $args]}
  }
}

proc faketk::pack_do {arglist} {
  lassign [split_geometry_args $arglist] ws options
  if {[llength $options] % 2} {
    return -code error "extra option \"[lindex $options end]\" (option with no value?)"
  }
  foreach w $ws {
    variable lay
    need $w
    # Tk's defaults, so that readers need not know them
    set o $options
    if {$lay($w) eq "" || [dict get $lay($w) manager] ne "pack"} {
      set o [concat {-side top} $options]
    }
    manage pack $w $o
  }
  return {}
}

proc ::grid {args} {
  set sub [lindex $args 0]
  switch -- $sub {
    configure {return [::faketk::grid_do [lrange $args 1 end]]}
    forget - remove {
      foreach w [lrange $args 1 end] {if {[::faketk::exists $w]} {::faketk::forget $w}}
      return {}
    }
    info {return [::faketk::geometry_info [lindex $args 1] grid]}
    slaves - content {return [::faketk::slaves_of [lindex $args 1] grid]}
    propagate {return 1}
    size {return {0 0}}
    bbox {return {0 0 0 0}}
    location {return {0 0}}
    anchor {return nw}
    columnconfigure - columnconfig - rowconfigure - rowconfig {
      # weights and minimum sizes: kept on the master for whoever lays out
      set m [lindex $args 1]
      if {[::faketk::exists $m] && [llength $args] > 3} {
        set kind [expr {[string match col* $sub] ? "column" : "row"}]
        foreach index [lindex $args 2] {
          foreach {o v} [lrange $args 3 end] {
            dict set ::faketk::opt($m) -grid$kind$index[string range $o 1 end] $v
          }
        }
        ::faketk::touch $m
      }
      return {}
    }
    default {return [::faketk::grid_do $args]}
  }
}

proc faketk::grid_do {arglist} {
  variable lay; variable slaves
  # widgets may be mixed with the placeholders - x ^
  set ws {}
  set i 0
  foreach a $arglist {
    if {[string match -?* $a]} break
    lappend ws $a
    incr i
  }
  set options [lrange $arglist $i end]
  if {[llength $options] % 2} {
    return -code error "extra option \"[lindex $options end]\" (option with no value?)"
  }
  set given [dict create]
  foreach {o v} $options {dict set given $o $v}
  set first [lindex $ws 0]
  if {$first eq "" || $first in {- x ^}} {return {}}
  need $first
  set m [expr {[dict exists $given -in] ? [dict get $given -in] : [parent $first]}]
  # default row: the first unused one
  if {[dict exists $given -row]} {
    set row [dict get $given -row]
  } else {
    set row 0
    foreach s [slaves_of $m grid] {
      if {$s in $ws} continue
      set r [dict get $lay($s) row]
      set span [expr {[dict exists $lay($s) rowspan] ? [dict get $lay($s) rowspan] : 1}]
      if {$r + $span > $row} {set row [expr {$r + $span}]}
    }
  }
  set col [expr {[dict exists $given -column] ? [dict get $given -column] : 0}]
  set prev {}
  foreach w $ws {
    switch -- $w {
      - {
        if {$prev ne ""} {
          set span [expr {[dict exists $lay($prev) columnspan] ? [dict get $lay($prev) columnspan] : 1}]
          dict set lay($prev) columnspan [expr {$span + 1}]
        }
        incr col
      }
      x {incr col}
      ^ {incr col}
      default {
        need $w
        set o $options
        set already [expr {$lay($w) ne "" && [dict get $lay($w) manager] eq "grid"}]
        set multi [expr {[llength $ws] > 1}]
        if {![dict exists $given -row] && (!$already || $multi)} {lappend o -row $row}
        if {$multi || (![dict exists $given -column] && !$already)} {lappend o -column $col}
        manage grid $w $o
        set span [expr {[dict exists $lay($w) columnspan] ? [dict get $lay($w) columnspan] : 1}]
        incr col $span
        set prev $w
      }
    }
  }
  return {}
}

proc ::place {args} {
  set sub [lindex $args 0]
  switch -- $sub {
    forget {foreach w [lrange $args 1 end] {if {[::faketk::exists $w]} {::faketk::forget $w}}; return {}}
    info {return [::faketk::geometry_info [lindex $args 1] place]}
    slaves - content {return [::faketk::slaves_of [lindex $args 1] place]}
    configure {set args [lrange $args 1 end]}
  }
  lassign [::faketk::split_geometry_args $args] ws options
  foreach w $ws {::faketk::manage place $w $options}
  return {}
}

# ---------------------------------------------------------------------------------
# Bindings
# ---------------------------------------------------------------------------------

namespace eval faketk {
  variable evtypes
  array set evtypes {
    Key KeyPress KeyPress KeyPress KeyRelease KeyRelease Button ButtonPress
    ButtonPress ButtonPress ButtonRelease ButtonRelease Motion Motion Enter Enter
    Leave Leave FocusIn FocusIn FocusOut FocusOut Expose Expose Configure Configure
    Map Map Unmap Unmap Destroy Destroy Visibility Visibility MouseWheel MouseWheel
    Property Property Activate Activate Deactivate Deactivate Reparent Reparent
    Gravity Gravity Circulate Circulate Colormap Colormap Create Create
  }
  variable evmods
  array set evmods {
    Control Control Shift Shift Alt Alt Meta Alt M Alt Lock Lock Mod1 Alt M1 Alt
    Mod2 Mod2 M2 Mod2 Mod3 Mod3 M3 Mod3 Mod4 Mod4 M4 Mod4 Mod5 Mod5 M5 Mod5
    Command Command Option Option Double Double Triple Triple Quadruple Quadruple
    Button1 B1 B1 B1 Button2 B2 B2 B2 Button3 B3 B3 B3 Button4 B4 B4 B4 Button5 B5
    B5 B5 Any {} Extended Extended
  }
  # X event type numbers, for %T
  variable evnum
  array set evnum {
    KeyPress 2 KeyRelease 3 ButtonPress 4 ButtonRelease 5 Motion 6 Enter 7 Leave 8
    FocusIn 9 FocusOut 10 Expose 12 Visibility 15 Destroy 17 Unmap 18 Map 19
    Configure 22 MouseWheel 38
  }
}

# Canonical form of an event sequence: <Control-Key-a> and <Control-a> are the same
# binding. Sequences of several events and virtual events are kept as written.
proc faketk::normseq {seq} {
  variable evtypes; variable evmods
  if {[string match <<*>> $seq] || ![regexp {^<([^<>]+)>$} $seq -> body]} {
    return $seq
  }
  set mods {}
  set type {}
  set detail {}
  set parts [split $body -]
  # a detail may itself be "-" (the minus key): <Key-minus> is usual, but be safe
  set n [llength $parts]
  for {set i 0} {$i < $n} {incr i} {
    set p [lindex $parts $i]
    if {$type eq "" && $detail eq "" && [info exists evmods($p)] && $i < $n - 1} {
      if {$evmods($p) ne ""} {lappend mods $evmods($p)}
    } elseif {$type eq "" && $detail eq "" && [info exists evtypes($p)]} {
      set type $evtypes($p)
    } else {
      set detail [join [lrange $parts $i end] -]
      break
    }
  }
  if {$type eq ""} {
    if {[string is integer -strict $detail] && $detail >= 1 && $detail <= 9} {
      set type ButtonPress
    } else {
      set type KeyPress
    }
  }
  set out [lsort -unique $mods]
  lappend out $type
  if {$detail ne ""} {lappend out $detail}
  return "<[join $out -]>"
}

proc ::bind {tag args} {
  variable ::faketk::binds
  if {![info exists binds($tag)]} {set binds($tag) [dict create]}
  set n [llength $args]
  if {$n == 0} {return [dict keys $binds($tag)]}
  set seq [::faketk::normseq [lindex $args 0]]
  if {$n == 1} {
    if {[dict exists $binds($tag) $seq]} {return [dict get $binds($tag) $seq]}
    return {}
  }
  set script [lindex $args 1]
  if {$script eq ""} {
    dict unset binds($tag) $seq
  } elseif {[string index $script 0] eq "+"} {
    set old [expr {[dict exists $binds($tag) $seq] ? [dict get $binds($tag) $seq] : ""}]
    dict set binds($tag) $seq "$old\n[string range $script 1 end]"
  } else {
    dict set binds($tag) $seq $script
  }
  return {}
}

proc ::bindtags {w args} {
  variable ::faketk::btags
  ::faketk::need $w
  if {[llength $args]} {
    set btags($w) [lindex $args 0]
    return {}
  }
  return [::faketk::tags_of $w]
}

proc faketk::tags_of {w} {
  variable btags; variable class
  if {[info exists btags($w)]} {return $btags($w)}
  set t [top $w]
  if {$t eq $w} {return [list $w $class($w) all]}
  return [list $w $class($w) $t all]
}

# The script bound to exactly this sequence on this tag.
proc faketk::binding {tag seq} {
  variable binds
  set seq [normseq $seq]
  if {[info exists binds($tag)] && [dict exists $binds($tag) $seq]} {
    return [dict get $binds($tag) $seq]
  }
  return {}
}

# Replace the % fields of a binding script. `fields` maps the letter to its value;
# letters not given become what Tk gives for an event without that field.
proc faketk::subst_event {script fields} {
  set map [list %% %]
  foreach c {W T x y X Y b k K N A s t w h D d E f m o p a c B R S i P} {
    if {[dict exists $fields $c]} {
      set v [dict get $fields $c]
    } else {
      switch -- $c {
        x - y - X - Y - b - k - N - s - t - w - h - D - E {set v 0}
        A - K {set v {}}
        default {set v ??}
      }
    }
    if {$c eq "A"} {
      # Tk quotes %A so that it stays one word
      set v [list $v]
      if {$v eq ""} {set v "{}"}
    }
    lappend map %$c $v
  }
  string map $map $script
}

# Run the bindings an event triggers on a widget: for each of its binding tags the
# most specific matching sequence, in order, until one does `break`.
# `candidates` lists sequences from most to least specific.
proc faketk::fire {w candidates fields} {
  if {![exists $w]} {return 0}
  if {![dict exists $fields W]} {dict set fields W $w}
  set ran 0
  foreach tag [tags_of $w] {
    foreach seq $candidates {
      set script [binding $tag $seq]
      if {$script eq ""} continue
      set ran 1
      set code [catch {uplevel #0 [subst_event $script $fields]} result]
      if {$code == 1} {problem $result}
      if {$code == 3} {return 1}  ;# break: no further tags
      break
    }
    if {![exists $w]} break
  }
  return $ran
}

# For the C side: run the bindings of an event on a widget.
#   faketk::dispatch .drw <ButtonPress-1> {x 10 y 20 b 1 s 0}
# The sequence names the event with its detail and modifiers; less specific bindings
# (<ButtonPress>, <Button-1> without the modifier, ...) are tried after it. %T is
# filled in from the event type unless given.
proc faketk::dispatch {w seq {fields {}}} {
  variable evnum
  set seq [normseq $seq]
  set candidates [list $seq]
  if {[regexp {^<(.*)>$} $seq -> body] && ![string match <<*>> $seq]} {
    set parts [split $body -]
    variable evtypes
    set ti -1
    for {set i 0} {$i < [llength $parts]} {incr i} {
      if {[info exists evtypes([lindex $parts $i])]} {set ti $i; break}
    }
    if {$ti >= 0} {
      set type [lindex $parts $ti]
      set mods [lrange $parts 0 [expr {$ti - 1}]]
      set detail [join [lrange $parts [expr {$ti + 1}] end] -]
      if {![dict exists $fields T] && [info exists evnum($type)]} {
        dict set fields T $evnum($type)
      }
      # drop the detail, then the modifiers one subset at a time (all or none is
      # enough for what xschem binds)
      if {$detail ne ""} {
        lappend candidates "<[join [concat $mods [list $type]] -]>"
      }
      if {[llength $mods]} {
        if {$detail ne ""} {lappend candidates "<$type-$detail>"}
        lappend candidates "<$type>"
      }
    }
  }
  fire $w $candidates $fields
}

proc ::event {sub args} {
  switch -- $sub {
    generate {
      lassign $args w seq
      set fields [dict create]
      foreach {o v} [lrange $args 2 end] {
        switch -- $o {
          -x {dict set fields x $v} -y {dict set fields y $v}
          -keysym {dict set fields K $v} -button {dict set fields b $v}
          -state {dict set fields s $v} -data {dict set fields d $v}
          -rootx {dict set fields X $v} -rooty {dict set fields Y $v}
          -delta {dict set fields D $v}
        }
      }
      if {[::faketk::exists $w]} {::faketk::dispatch $w $seq $fields}
    }
    add - delete - info {}
  }
  return {}
}

# ---------------------------------------------------------------------------------
# The widget command
# ---------------------------------------------------------------------------------

proc faketk::w {w args} {
  variable class
  if {![info exists class($w)]} {
    return -code error "invalid command name \"$w\""
  }
  set sub [lindex $args 0]
  set rest [lrange $args 1 end]
  switch -- $sub {
    configure - config {return [configure $w $rest]}
    cget {return [cget $w [lindex $rest 0]]}
  }
  set cls $class($w)
  switch -- $cls {
    Entry - Spinbox - TCombobox - TEntry {return [w_entry $w $sub $rest]}
    Text {return [w_text $w $sub $rest]}
    Listbox {return [w_listbox $w $sub $rest]}
    Button - Checkbutton - Radiobutton - TButton - TCheckbutton - TRadiobutton -
    Menubutton {return [w_button $w $sub $rest]}
    Menu {return [w_menu $w $sub $rest]}
    Scale - TScale {return [w_scale $w $sub $rest]}
    Scrollbar - TScrollbar {
      switch -- $sub {
        get {return {0.0 1.0}}
        default {return {}}
      }
    }
    Panedwindow - TPanedwindow {return [w_paned $w $sub $rest]}
    Canvas {return [w_canvas $w $sub $rest]}
    default {
      switch -- $sub {
        state - instate {return {}}
      }
      todo "$cls $sub"
      return {}
    }
  }
}

# --- buttons -----------------------------------------------------------------------

# Whether a check or radio button is on.
proc faketk::is_on {w} {
  variable class
  set v [cget $w -variable]
  if {$v eq ""} {return 0}
  switch -- $class($w) {
    Checkbutton - TCheckbutton {
      return [expr {[vget $v] eq [cget $w -onvalue]}]
    }
    Radiobutton - TRadiobutton {
      return [expr {[info exists [gv $v]] && [vget $v] eq [cget $w -value]}]
    }
  }
  return 0
}

proc faketk::w_button {w sub rest} {
  variable class
  set cls $class($w)
  switch -- $sub {
    invoke {
      if {[cget $w -state] eq "disabled"} {return {}}
      switch -- $cls {
        Checkbutton - TCheckbutton {
          set v [cget $w -variable]
          if {$v ne ""} {
            vset $v [expr {[is_on $w] ? [cget $w -offvalue] : [cget $w -onvalue]}]
          }
        }
        Radiobutton - TRadiobutton {
          set v [cget $w -variable]
          if {$v ne ""} {vset $v [cget $w -value]}
        }
      }
      touch $w
      set command [cget $w -command]
      if {$command ne ""} {
        return [uplevel #0 $command]
      }
      return {}
    }
    select {
      set v [cget $w -variable]
      if {$v ne ""} {
        vset $v [expr {[string match *Check* $cls] ? [cget $w -onvalue] : [cget $w -value]}]
      }
      touch $w
    }
    deselect {
      set v [cget $w -variable]
      if {$v ne ""} {
        vset $v [expr {[string match *Check* $cls] ? [cget $w -offvalue] : ""}]
      }
      touch $w
    }
    toggle {
      set v [cget $w -variable]
      if {$v ne ""} {
        vset $v [expr {[is_on $w] ? [cget $w -offvalue] : [cget $w -onvalue]}]
      }
      touch $w
    }
    flash - state - instate {}
    default {todo "$cls $sub"}
  }
  return {}
}

# --- entry, spinbox, combobox -----------------------------------------------------------

proc faketk::entry_index {w idx} {
  variable sel
  set text [text_of $w]
  set len [string length $text]
  switch -regexp -- $idx {
    {^end$} {return $len}
    {^insert$} {return [expr {[info exists ::faketk::marks($w,insert)] ? min($::faketk::marks($w,insert),$len) : $len}]}
    {^anchor$} {return 0}
    {^sel\.first$} {
      if {[llength $sel($w)] != 2} {return -code error "selection isn't in widget $w"}
      return [lindex $sel($w) 0]
    }
    {^sel\.last$} {
      if {[llength $sel($w)] != 2} {return -code error "selection isn't in widget $w"}
      return [lindex $sel($w) 1]
    }
    {^@} {return 0}
    {^-?[0-9]+$} {return [expr {max(0, min($idx, $len))}]}
  }
  return -code error "bad entry index \"$idx\""
}

proc faketk::w_entry {w sub rest} {
  variable sel; variable marks; variable class
  switch -- $sub {
    get {return [text_of $w]}
    set {
      # ttk::combobox and spinbox
      set_text $w [lindex $rest 0]
      return {}
    }
    current {
      set values [cget $w -values]
      if {[llength $rest]} {
        set_text $w [lindex $values [lindex $rest 0]]
        return {}
      }
      return [lsearch -exact $values [text_of $w]]
    }
    insert {
      lassign $rest idx string
      set text [text_of $w]
      set i [entry_index $w $idx]
      set_text $w [string range $text 0 [expr {$i - 1}]]$string[string range $text $i end]
      set marks($w,insert) [expr {$i + [string length $string]}]
      return {}
    }
    delete {
      lassign $rest first last
      set text [text_of $w]
      set a [entry_index $w $first]
      set b [expr {$last eq "" ? $a + 1 : [entry_index $w $last]}]
      if {$b > $a} {
        set_text $w [string range $text 0 [expr {$a - 1}]][string range $text $b end]
        set marks($w,insert) $a
        set sel($w) {}
      }
      return {}
    }
    index {return [entry_index $w [lindex $rest 0]]}
    icursor {
      set marks($w,insert) [entry_index $w [lindex $rest 0]]
      return {}
    }
    selection {
      switch -- [lindex $rest 0] {
        range {
          set a [entry_index $w [lindex $rest 1]]
          set b [entry_index $w [lindex $rest 2]]
          set sel($w) [expr {$b > $a ? [list $a $b] : {}}]
        }
        clear {set sel($w) {}}
        present {return [expr {[llength $sel($w)] == 2}]}
        from - to - adjust - element {}
      }
      return {}
    }
    invoke {
      # spinbox buttons
      set command [cget $w -command]
      if {$command ne ""} {run $command}
      return {}
    }
    validate {return 1}
    xview - scan - bbox - state - instate - identify {return {}}
    default {todo "$class($w) $sub"; return {}}
  }
}

proc faketk::w_scale {w sub rest} {
  switch -- $sub {
    get {return [text_of $w]}
    set {
      set_text $w [lindex $rest 0]
      set command [cget $w -command]
      if {$command ne ""} {run [concat $command [list [lindex $rest 0]]]}
      return {}
    }
    coords {return {0 0}}
    identify {return {}}
    default {todo "Scale $sub"; return {}}
  }
}

proc faketk::w_paned {w sub rest} {
  switch -- $sub {
    add {
      lassign [split_geometry_args $rest] ws options
      foreach p $ws {manage pane $p $options}
      return {}
    }
    forget - remove {foreach p $rest {if {[exists $p]} {forget $p}}; return {}}
    panes {return [slaves_of $w pane]}
    paneconfigure - paneconfig {
      set p [lindex $rest 0]
      if {[llength $rest] > 2} {manage pane $p [lrange $rest 1 end]}
      return {}
    }
    panecget {
      variable lay
      set p [lindex $rest 0]
      set k [string range [lindex $rest 1] 1 end]
      if {[dict exists $lay($p) $k]} {return [dict get $lay($p) $k]}
      return {}
    }
    sash {
      if {[lindex $rest 0] eq "coord"} {return {0 0}}
      return {}
    }
    sashpos {return 0}
    proxy - identify {return {}}
    default {todo "Panedwindow $sub"; return {}}
  }
}

# Canvas items are accepted and forgotten: xschem draws its schematic itself, and uses
# no canvas of its own. `create` still returns ids so that callers keep working.
proc faketk::w_canvas {w sub rest} {
  variable serial
  switch -- $sub {
    create {return [incr serial]}
    bbox {return {0 0 0 0}}
    coords - find - gettags - itemcget - type {return {}}
    canvasx - canvasy {return [lindex $rest 0]}
    xview - yview {return {0.0 1.0}}
    default {return {}}
  }
}

# --- listbox ----------------------------------------------------------------------------------------

proc faketk::lb_items {w} {
  variable items
  set v [cget $w -listvariable]
  if {$v ne ""} {return [vget $v]}
  return $items($w)
}

proc faketk::lb_set {w list} {
  variable items
  set v [cget $w -listvariable]
  if {$v ne ""} {vset $v $list} else {set items($w) $list}
  touch $w
}

proc faketk::lb_index {w idx {for_insert 0}} {
  variable sel; variable marks
  set n [llength [lb_items $w]]
  switch -regexp -- $idx {
    {^end$} {return [expr {$for_insert ? $n : $n - 1}]}
    {^active$} {return [expr {[info exists marks($w,active)] ? $marks($w,active) : 0}]}
    {^anchor$} {return [expr {[info exists marks($w,anchor)] ? $marks($w,anchor) : 0}]}
    {^@} {return 0}
    {^-?[0-9]+$} {return $idx}
  }
  return -code error "bad listbox index \"$idx\": must be active, anchor, end, @x,y, or a number"
}

proc faketk::w_listbox {w sub rest} {
  variable sel; variable marks
  switch -- $sub {
    insert {
      set list [lb_items $w]
      set i [lb_index $w [lindex $rest 0] 1]
      set i [expr {max(0, min($i, [llength $list]))}]
      # selected lines below move down
      set add [llength $rest]
      incr add -1
      set new {}
      foreach s $sel($w) {lappend new [expr {$s >= $i ? $s + $add : $s}]}
      set sel($w) $new
      lb_set $w [linsert $list $i {*}[lrange $rest 1 end]]
      return {}
    }
    delete {
      set list [lb_items $w]
      set a [lb_index $w [lindex $rest 0]]
      set b [expr {[llength $rest] > 1 ? [lb_index $w [lindex $rest 1]] : $a}]
      set a [expr {max(0, $a)}]
      set b [expr {min($b, [llength $list] - 1)}]
      if {$b < $a} {return {}}
      set gone [expr {$b - $a + 1}]
      set new {}
      foreach s $sel($w) {
        if {$s < $a} {lappend new $s} elseif {$s > $b} {lappend new [expr {$s - $gone}]}
      }
      set sel($w) $new
      lb_set $w [lreplace $list $a $b]
      return {}
    }
    get {
      set list [lb_items $w]
      set a [lb_index $w [lindex $rest 0]]
      if {[llength $rest] > 1} {
        return [lrange $list $a [lb_index $w [lindex $rest 1]]]
      }
      return [lindex $list $a]
    }
    size {return [llength [lb_items $w]]}
    curselection {
      set n [llength [lb_items $w]]
      set out {}
      foreach s [lsort -integer -unique $sel($w)] {if {$s >= 0 && $s < $n} {lappend out $s}}
      return $out
    }
    selection {
      set what [lindex $rest 0]
      switch -- $what {
        set {
          set a [lb_index $w [lindex $rest 1]]
          set b [expr {[llength $rest] > 2 ? [lb_index $w [lindex $rest 2]] : $a}]
          if {$b < $a} {lassign [list $b $a] a b}
          for {set i $a} {$i <= $b} {incr i} {
            if {$i >= 0 && $i ni $sel($w)} {lappend sel($w) $i}
          }
          touch $w
        }
        clear {
          set a [lb_index $w [lindex $rest 1]]
          set b [expr {[llength $rest] > 2 ? [lb_index $w [lindex $rest 2]] : $a}]
          if {$b < $a} {lassign [list $b $a] a b}
          set new {}
          foreach s $sel($w) {if {$s < $a || $s > $b} {lappend new $s}}
          set sel($w) $new
          touch $w
        }
        includes {
          return [expr {[lb_index $w [lindex $rest 1]] in $sel($w)}]
        }
        anchor {set marks($w,anchor) [lb_index $w [lindex $rest 1]]}
      }
      return {}
    }
    activate {
      set marks($w,active) [lb_index $w [lindex $rest 0]]
      return {}
    }
    index {return [lb_index $w [lindex $rest 0] 1]}
    nearest {return 0}
    itemconfigure - itemconfig {
      # per line colours: kept as an option so that they can be shown one day
      return {}
    }
    itemcget {return {}}
    see - scan {return {}}
    xview - yview {
      if {[llength $rest]} {return {}}
      return {0.0 1.0}
    }
    bbox {return {0 0 10 10}}
    default {todo "Listbox $sub"; return {}}
  }
}

# --- text -----------------------------------------------------------------------------------------
#
# The content is kept without the newline Tk adds at the end; positions are character
# offsets, where offset = length means "before the final newline" and length + 1 is
# `end`.

proc faketk::t_lc2off {text line col} {
  set lines [split $text \n]
  set n [llength $lines]
  if {$line < 1} {return 0}
  if {$line > $n} {return [expr {[string length $text] + 1}]}
  set off 0
  for {set i 0} {$i < $line - 1} {incr i} {
    incr off [expr {[string length [lindex $lines $i]] + 1}]
  }
  set ll [string length [lindex $lines [expr {$line - 1}]]]
  if {$col eq "end" || $col > $ll} {set col $ll}
  expr {$off + $col}
}

proc faketk::t_off2lc {text off} {
  set len [string length $text]
  if {$off < 0} {set off 0}
  if {$off > $len} {
    return "[expr {[llength [split $text \n]] + 1}].0"
  }
  set before [string range $text 0 [expr {$off - 1}]]
  set line [expr {[regexp -all \n $before] + 1}]
  set col [expr {$off - [string last \n $before] - 1}]
  return $line.$col
}

proc faketk::t_index {w idx} {
  variable content; variable marks; variable tags
  set text $content($w)
  set len [string length $text]
  set rest [string trim $idx]
  if {[regexp {^(-?[0-9]+)\.([0-9]+|end)(.*)$} $rest -> l c rest]} {
    set off [t_lc2off $text $l $c]
  } elseif {[regexp {^end(?![A-Za-z0-9_.])(.*)$} $rest -> rest]} {
    set off [expr {$len + 1}]
  } elseif {[regexp {^@-?[0-9]+,-?[0-9]+(.*)$} $rest -> rest]} {
    set off 0
  } elseif {[regexp {^([^\s.+-]+)\.(first|last)(.*)$} $rest -> tag which rest]} {
    if {![info exists tags($w,$tag)] || $tags($w,$tag) eq ""} {
      return -code error "text doesn't contain any characters tagged with \"$tag\""
    }
    set off [lindex $tags($w,$tag) [expr {$which eq "first" ? 0 : "end"}]]
  } elseif {[regexp {^([^\s+-]+)(.*)$} $rest -> name rest]} {
    if {[info exists marks($w,$name)]} {
      set off $marks($w,$name)
    } else {
      return -code error "bad text index \"$idx\""
    }
  } else {
    return -code error "bad text index \"$idx\""
  }
  while 1 {
    set rest [string trimleft $rest]
    if {$rest eq ""} break
    if {[regexp {^([+-])\s*([0-9]+)\s*(?:display|any)?\s*([a-z]*)(.*)$} $rest -> sign n unit rest]} {
      if {$unit eq "" || [string match ${unit}* chars] || [string match ${unit}* indices]} {
        set off [expr {$sign eq "+" ? $off + $n : $off - $n}]
      } elseif {[string match ${unit}* lines]} {
        lassign [split [t_off2lc $text [expr {min($off, $len)}]] .] l c
        set off [t_lc2off $text [expr {$sign eq "+" ? $l + $n : $l - $n}] $c]
      } else {
        return -code error "bad text index \"$idx\""
      }
    } elseif {[regexp {^(?:display|any)?\s*(linestart|lineend|wordstart|wordend)(.*)$} $rest -> what rest]} {
      set o [expr {min($off, $len)}]
      switch -- $what {
        linestart {set off [expr {[string last \n $text [expr {$o - 1}]] + 1}]}
        lineend {
          set e [string first \n $text $o]
          set off [expr {$e < 0 ? $len : $e}]
        }
        wordstart {set off [string wordstart $text $o]}
        wordend {set off [string wordend $text $o]}
      }
    } else {
      return -code error "bad text index \"$idx\""
    }
    if {$off < 0} {set off 0}
    if {$off > $len + 1} {set off [expr {$len + 1}]}
  }
  return $off
}

# Shift marks and tags after `n` characters were inserted at `off`.
proc faketk::t_inserted {w off n} {
  variable marks; variable tags
  foreach key [array names marks $w,*] {
    if {$marks($key) >= $off} {incr marks($key) $n}
  }
  foreach key [array names tags $w,*] {
    set new {}
    foreach {a b} $tags($key) {
      if {$a >= $off} {incr a $n}
      if {$b > $off} {incr b $n}
      lappend new $a $b
    }
    set tags($key) $new
  }
}

# Shift marks and tags after the range a..b was deleted.
proc faketk::t_deleted {w a b} {
  variable marks; variable tags
  set n [expr {$b - $a}]
  foreach key [array names marks $w,*] {
    if {$marks($key) >= $b} {
      incr marks($key) -$n
    } elseif {$marks($key) > $a} {
      set marks($key) $a
    }
  }
  foreach key [array names tags $w,*] {
    set new {}
    foreach {s e} $tags($key) {
      foreach v {s e} {
        if {[set $v] >= $b} {incr $v -$n} elseif {[set $v] > $a} {set $v $a}
      }
      if {$e > $s} {lappend new $s $e}
    }
    set tags($key) $new
  }
}

proc faketk::t_tag_add {w tag a b} {
  variable tags
  if {$b <= $a} return
  if {![info exists tags($w,$tag)]} {set tags($w,$tag) {}}
  # merge with what is there
  set ranges [list $a $b]
  foreach {s e} $tags($w,$tag) {lappend ranges $s $e}
  set pairs {}
  foreach {s e} $ranges {lappend pairs [list $s $e]}
  set out {}
  foreach pair [lsort -integer -index 0 $pairs] {
    lassign $pair s e
    if {[llength $out] && $s <= [lindex $out end]} {
      if {$e > [lindex $out end]} {lset out end $e}
    } else {
      lappend out $s $e
    }
  }
  set tags($w,$tag) $out
}

proc faketk::t_tag_remove {w tag a b} {
  variable tags
  if {![info exists tags($w,$tag)]} return
  set out {}
  foreach {s e} $tags($w,$tag) {
    if {$e <= $a || $s >= $b} {
      lappend out $s $e
    } else {
      if {$s < $a} {lappend out $s $a}
      if {$e > $b} {lappend out $b $e}
    }
  }
  set tags($w,$tag) $out
}

proc faketk::t_insert {w off string taglist} {
  variable content
  set len [string length $content($w)]
  if {$off > $len} {set off $len}
  set n [string length $string]
  if {$n == 0} {return $off}
  set content($w) [string range $content($w) 0 [expr {$off - 1}]]$string[string range $content($w) $off end]
  t_inserted $w $off $n
  foreach tag $taglist {t_tag_add $w $tag $off [expr {$off + $n}]}
  return [expr {$off + $n}]
}

proc faketk::w_text {w sub rest} {
  variable content; variable marks; variable tags; variable tagopt
  switch -- $sub {
    insert {
      set off [t_index $w [lindex $rest 0]]
      foreach {chars taglist} [lrange $rest 1 end] {
        set off [t_insert $w $off $chars $taglist]
      }
      set marks($w,modified) 1
      touch $w
      return {}
    }
    delete {
      set len [string length $content($w)]
      foreach {i1 i2} [lrange $rest 0 end] {
        set a [t_index $w $i1]
        set b [expr {$i2 eq "" ? $a + 1 : [t_index $w $i2]}]
        if {$b > $len} {set b $len}
        if {$b > $a} {
          set content($w) [string range $content($w) 0 [expr {$a - 1}]][string range $content($w) $b end]
          t_deleted $w $a $b
          set len [string length $content($w)]
        }
      }
      set marks($w,modified) 1
      touch $w
      return {}
    }
    replace {
      lassign $rest i1 i2 chars
      set a [t_index $w $i1]
      set b [t_index $w $i2]
      w_text $w delete [list $i1 $i2]
      t_insert $w $a $chars [lindex $rest 3]
      touch $w
      return {}
    }
    get {
      while {[string match -* [lindex $rest 0]]} {set rest [lrange $rest 1 end]}
      set full "$content($w)\n"
      set out {}
      foreach {i1 i2} $rest {
        set a [t_index $w $i1]
        set b [expr {$i2 eq "" ? $a + 1 : [t_index $w $i2]}]
        lappend out [string range $full $a [expr {$b - 1}]]
      }
      if {[llength $out] == 1} {return [lindex $out 0]}
      return $out
    }
    index {return [t_off2lc $content($w) [t_index $w [lindex $rest 0]]]}
    compare {
      lassign $rest i1 op i2
      return [expr "[t_index $w $i1] $op [t_index $w $i2]"]
    }
    count {
      set lines 0
      while {[string match -* [lindex $rest 0]]} {
        if {[string match -*lines [lindex $rest 0]]} {set lines 1}
        set rest [lrange $rest 1 end]
      }
      lassign $rest i1 i2
      set a [t_index $w $i1]
      set b [t_index $w $i2]
      if {$lines} {
        set full "$content($w)\n"
        return [regexp -all \n [string range $full $a [expr {$b - 1}]]]
      }
      return [expr {$b - $a}]
    }
    mark {
      set what [lindex $rest 0]
      switch -- $what {
        set {
          set marks($w,[lindex $rest 1]) [t_index $w [lindex $rest 2]]
          if {[lindex $rest 1] eq "insert"} {touch $w}
        }
        unset {foreach m [lrange $rest 1 end] {unset -nocomplain marks($w,$m)}}
        names {
          set out {}
          foreach key [array names marks $w,*] {
            set m [string range $key [expr {[string length $w] + 1}] end]
            if {$m ni {modified}} {lappend out $m}
          }
          return $out
        }
        gravity {return right}
      }
      return {}
    }
    tag {
      set what [lindex $rest 0]
      set tag [lindex $rest 1]
      switch -- $what {
        add {
          foreach {i1 i2} [lrange $rest 2 end] {
            set a [t_index $w $i1]
            set b [expr {$i2 eq "" ? $a + 1 : [t_index $w $i2]}]
            set len [string length $content($w)]
            t_tag_add $w $tag [expr {min($a, $len)}] [expr {min($b, $len)}]
          }
          if {![info exists tags($w,$tag)]} {set tags($w,$tag) {}}
        }
        remove {
          foreach {i1 i2} [lrange $rest 2 end] {
            set a [t_index $w $i1]
            set b [expr {$i2 eq "" ? $a + 1 : [t_index $w $i2]}]
            t_tag_remove $w $tag $a $b
          }
        }
        ranges {
          set out {}
          if {[info exists tags($w,$tag)]} {
            foreach off $tags($w,$tag) {lappend out [t_off2lc $content($w) $off]}
          }
          return $out
        }
        nextrange - prevrange {
          if {![info exists tags($w,$tag)]} {return {}}
          set from [t_index $w [lindex $rest 2]]
          set found {}
          foreach {s e} $tags($w,$tag) {
            if {$what eq "nextrange" && $s >= $from} {set found [list $s $e]; break}
            if {$what eq "prevrange" && $s < $from} {set found [list $s $e]}
          }
          set out {}
          foreach off $found {lappend out [t_off2lc $content($w) $off]}
          return $out
        }
        names {
          set out {}
          set prefix [expr {[string length $w] + 1}]
          if {$tag eq ""} {
            foreach key [array names tags $w,*] {lappend out [string range $key $prefix end]}
            if {"sel" ni $out} {lappend out sel}
            return $out
          }
          set off [t_index $w $tag]
          foreach key [array names tags $w,*] {
            foreach {s e} $tags($key) {
              if {$off >= $s && $off < $e} {lappend out [string range $key $prefix end]; break}
            }
          }
          return $out
        }
        delete {
          foreach t [lrange $rest 1 end] {
            unset -nocomplain tags($w,$t) tagopt($w,$t)
          }
        }
        configure - config {
          if {![info exists tagopt($w,$tag)]} {set tagopt($w,$tag) {}}
          if {![info exists tags($w,$tag)]} {set tags($w,$tag) {}}
          foreach {o v} [lrange $rest 2 end] {dict set tagopt($w,$tag) $o $v}
        }
        cget {
          set o [lindex $rest 2]
          if {[info exists tagopt($w,$tag)] && [dict exists $tagopt($w,$tag) $o]} {
            return [dict get $tagopt($w,$tag) $o]
          }
          return {}
        }
        bind {
          if {[llength $rest] > 3} {::bind $w.tag.$tag [lindex $rest 2] [lindex $rest 3]}
          return {}
        }
        lower - raise {}
      }
      return {}
    }
    search {
      set regexp 0; set nocase 0; set backwards 0; set countvar {}; set nolinestop 0
      set i 0
      while {$i < [llength $rest]} {
        set a [lindex $rest $i]
        if {$a eq "--"} {incr i; break}
        if {![string match -* $a]} break
        switch -glob -- $a {
          -reg* {set regexp 1}
          -noc* {set nocase 1}
          -ba* {set backwards 1}
          -c* {incr i; set countvar [lindex $rest $i]}
          -nol* {set nolinestop 1}
        }
        incr i
      }
      lassign [lrange $rest $i end] pattern from stop
      set full "$content($w)\n"
      set start [t_index $w $from]
      set limit [expr {$stop eq "" ? -1 : [t_index $w $stop]}]
      set found -1
      set flen 0
      # all matches, then pick: simple and good enough for dialog-sized texts
      set matches {}
      if {$regexp} {
        set opts {-all -inline -indices}
        if {!$nolinestop} {lappend opts -line}
        if {$nocase} {lappend opts -nocase}
        if {[catch {set all [regexp {*}$opts -- $pattern $full]}]} {set all {}}
        # with sub-expressions every match yields several pairs: keep the whole match
        set groups 1
        catch {set groups [expr {[lindex [regexp -about -- $pattern] 0] + 1}]}
        set k 0
        foreach pair $all {
          if {$k % $groups == 0} {
            lassign $pair s e
            if {$e >= $s} {lappend matches [list $s [expr {$e - $s + 1}]]}
          }
          incr k
        }
      } else {
        set hay [expr {$nocase ? [string tolower $full] : $full}]
        set needle [expr {$nocase ? [string tolower $pattern] : $pattern}]
        set pos 0
        if {$needle ne ""} {
          while {[set p [string first $needle $hay $pos]] >= 0} {
            lappend matches [list $p [string length $needle]]
            set pos [expr {$p + 1}]
          }
        }
      }
      if {$backwards} {
        foreach m $matches {
          lassign $m s l
          if {$s < $start && ($limit < 0 || $s >= $limit)} {set found $s; set flen $l}
        }
        if {$found < 0 && $limit < 0 && [llength $matches]} {
          lassign [lindex $matches end] found flen
        }
      } else {
        foreach m $matches {
          lassign $m s l
          if {$s >= $start && ($limit < 0 || $s < $limit)} {set found $s; set flen $l; break}
        }
        if {$found < 0 && $limit < 0 && [llength $matches]} {
          lassign [lindex $matches 0] found flen
        }
      }
      if {$found < 0} {return {}}
      if {$countvar ne ""} {
        upvar 2 $countvar cv
        set cv $flen
      }
      return [t_off2lc $content($w) $found]
    }
    edit {
      switch -- [lindex $rest 0] {
        modified {
          if {[llength $rest] > 1} {
            set marks($w,modified) [expr {[lindex $rest 1] ? 1 : 0}]
            return {}
          }
          return [expr {[info exists marks($w,modified)] ? $marks($w,modified) : 0}]
        }
        canundo - canredo {return 0}
      }
      return {}
    }
    see - scan - window - image - peer - sync {return {}}
    xview - yview {
      if {[llength $rest]} {return {}}
      return {0.0 1.0}
    }
    bbox - dlineinfo - dump {return {}}
    default {todo "Text $sub"; return {}}
  }
}

# --- menu -------------------------------------------------------------------------------------------

proc faketk::m_index {m idx} {
  variable menu
  set n [llength $menu($m)]
  if {[string is integer -strict $idx]} {
    return [expr {$idx >= $n ? $n - 1 : $idx}]
  }
  switch -glob -- $idx {
    end - last {return [expr {$n - 1}]}
    active - none - {} {return -1}
    @* {return 0}
  }
  set i 0
  foreach e $menu($m) {
    if {[dict exists $e -label] && [dict get $e -label] eq $idx} {return $i}
    incr i
  }
  set i 0
  foreach e $menu($m) {
    if {[dict exists $e -label] && [string match $idx [dict get $e -label]]} {return $i}
    incr i
  }
  return -code error "bad menu entry index \"$idx\""
}

proc faketk::mtrace {args} {
  catch {::faketk::mark menu}
}

proc faketk::m_entry {type options} {
  variable mtraced
  set e [dict create type $type]
  foreach {o v} $options {
    set o [optname $o]
    dict set e $o $v
  }
  if {[dict exists $e -variable] && [dict get $e -variable] ne ""} {
    set n [gv [dict get $e -variable]]
    if {![info exists mtraced($n)]} {
      if {![catch {trace add variable $n write ::faketk::mtrace}]} {set mtraced($n) 1}
    }
  }
  return $e
}

# The variable and the on/off values of a check or radio entry.
proc faketk::m_var {e} {
  set label [expr {[dict exists $e -label] ? [dict get $e -label] : ""}]
  if {[dict get $e type] eq "checkbutton"} {
    set var [expr {[dict exists $e -variable] ? [dict get $e -variable] : $label}]
    set on [expr {[dict exists $e -onvalue] ? [dict get $e -onvalue] : 1}]
    set off [expr {[dict exists $e -offvalue] ? [dict get $e -offvalue] : 0}]
    return [list $var $on $off]
  }
  set var [expr {[dict exists $e -variable] ? [dict get $e -variable] : "selectedButton"}]
  set on [expr {[dict exists $e -value] ? [dict get $e -value] : $label}]
  return [list $var $on {}]
}

proc faketk::m_checked {e} {
  lassign [m_var $e] var on off
  if {$var eq "" || ![info exists [gv $var]]} {return 0}
  expr {[vget $var] eq $on}
}

proc faketk::w_menu {m sub rest} {
  variable menu; variable popup; variable popup_pending
  if {![info exists menu($m)]} {set menu($m) {}}
  switch -- $sub {
    add {
      lappend menu($m) [m_entry [lindex $rest 0] [lrange $rest 1 end]]
      mark menu
      return {}
    }
    insert {
      set i [m_index $m [lindex $rest 0]]
      if {$i < 0} {set i [llength $menu($m)]}
      set menu($m) [linsert $menu($m) $i [m_entry [lindex $rest 1] [lrange $rest 2 end]]]
      mark menu
      return {}
    }
    delete {
      if {![llength $menu($m)]} {return {}}
      set a [m_index $m [lindex $rest 0]]
      set b [expr {[llength $rest] > 1 ? [m_index $m [lindex $rest 1]] : $a}]
      if {$a < 0 || $b < $a} {return {}}
      set menu($m) [lreplace $menu($m) $a $b]
      mark menu
      return {}
    }
    entryconfigure - entryconfig {
      set i [m_index $m [lindex $rest 0]]
      if {$i < 0} {return {}}
      set e [lindex $menu($m) $i]
      if {[llength $rest] == 1} {
        set out {}
        dict for {o v} $e {if {$o ne "type"} {lappend out [list $o {} {} {} $v]}}
        return $out
      }
      if {[llength $rest] == 2} {
        set o [optname [lindex $rest 1]]
        return [list $o {} {} {} [expr {[dict exists $e $o] ? [dict get $e $o] : ""}]]
      }
      set new [m_entry [dict get $e type] [lrange $rest 1 end]]
      lset menu($m) $i [dict merge $e $new]
      mark menu
      return {}
    }
    entrycget {
      set i [m_index $m [lindex $rest 0]]
      if {$i < 0} {return {}}
      set e [lindex $menu($m) $i]
      set o [optname [lindex $rest 1]]
      if {[dict exists $e $o]} {return [dict get $e $o]}
      switch -- $o {
        -state {return normal}
        -onvalue {return 1}
        -offvalue {return 0}
      }
      return {}
    }
    type {
      set i [m_index $m [lindex $rest 0]]
      if {$i < 0} {return {}}
      return [dict get [lindex $menu($m) $i] type]
    }
    index {
      set i [m_index $m [lindex $rest 0]]
      return [expr {$i < 0 ? "none" : $i}]
    }
    invoke {
      set i [m_index $m [lindex $rest 0]]
      if {$i < 0} {return {}}
      set e [lindex $menu($m) $i]
      if {[dict exists $e -state] && [dict get $e -state] eq "disabled"} {return {}}
      switch -- [dict get $e type] {
        checkbutton {
          lassign [m_var $e] var on off
          if {$var ne ""} {vset $var [expr {[m_checked $e] ? $off : $on}]}
          mark menu
        }
        radiobutton {
          lassign [m_var $e] var on
          if {$var ne ""} {vset $var $on}
          mark menu
        }
        cascade - separator - tearoff {return {}}
      }
      if {[dict exists $e -command] && [dict get $e -command] ne ""} {
        return [uplevel #0 [dict get $e -command]]
      }
      return {}
    }
    post {
      set popup $m
      set popup_pending 1
      schedule
      return {}
    }
    unpost {
      if {$popup eq $m} {set popup {}}
      return {}
    }
    activate - postcascade - clone {return {}}
    yposition - xposition {return 0}
    default {todo "Menu $sub"; return {}}
  }
}

# ---------------------------------------------------------------------------------
# wm, winfo, focus, grab
# ---------------------------------------------------------------------------------

proc ::wm {sub w args} {
  variable ::faketk::wmi
  ::faketk::need $w
  set n [llength $args]
  switch -- $sub {
    title {
      if {!$n} {return [expr {[info exists wmi($w,title)] ? $wmi($w,title) : ""}]}
      set wmi($w,title) [lindex $args 0]
      if {$w eq "."} {::faketk::mark title} else {::faketk::mark win,$w}
      return {}
    }
    withdraw - iconify {
      set wmi($w,state) [expr {$sub eq "withdraw" ? "withdrawn" : "iconic"}]
      ::faketk::mark win,$w
      return {}
    }
    deiconify {
      set wmi($w,state) normal
      ::faketk::mark win,$w
      return {}
    }
    state {
      if {!$n} {return [expr {[info exists wmi($w,state)] ? $wmi($w,state) : "normal"}]}
      set wmi($w,state) [lindex $args 0]
      ::faketk::mark win,$w
      return {}
    }
    protocol {
      if {!$n} {
        set out {}
        foreach key [array names wmi $w,protocol,*] {lappend out [lindex [split $key ,] end]}
        return $out
      }
      set name [lindex $args 0]
      if {$n == 1} {
        return [expr {[info exists wmi($w,protocol,$name)] ? $wmi($w,protocol,$name) : ""}]
      }
      set wmi($w,protocol,$name) [lindex $args 1]
      return {}
    }
    geometry {
      if {!$n} {
        if {[info exists wmi($w,geometry)]} {return $wmi($w,geometry)}
        return "[::winfo width $w]x[::winfo height $w]+0+0"
      }
      set wmi($w,geometry) [lindex $args 0]
      ::faketk::mark win,$w
      return {}
    }
    overrideredirect {
      if {!$n} {return [expr {[info exists wmi($w,overrideredirect)] ? $wmi($w,overrideredirect) : 0}]}
      set wmi($w,overrideredirect) [expr {[lindex $args 0] ? 1 : 0}]
      ::faketk::mark win,$w
      return {}
    }
    attributes {
      if {$n == 1} {
        set key $w,attributes,[lindex $args 0]
        return [expr {[info exists wmi($key)] ? $wmi($key) : ""}]
      }
      foreach {o v} $args {set wmi($w,attributes,$o) $v}
      return {}
    }
    frame {return [::faketk::window_id $w]}
    stackorder {
      switch -- [lindex $args 0] {
        isabove {return 1}
        isbelow {return 0}
      }
      return [list $w]
    }
    default {
      # transient resizable minsize maxsize iconname iconphoto iconbitmap group
      # focusmodel positionfrom sizefrom client command aspect grid forget manage ...
      if {!$n} {return [expr {[info exists wmi($w,$sub)] ? $wmi($w,$sub) : ""}]}
      set wmi($w,$sub) $args
      return {}
    }
  }
}

# Whether a widget would be on screen.
proc faketk::mapped {w} {
  variable class; variable lay; variable wmi
  if {![info exists class($w)]} {return 0}
  if {$class($w) eq "Toplevel"} {
    return [expr {![info exists wmi($w,state)] || $wmi($w,state) eq "normal"}]
  }
  if {$class($w) eq "Menu"} {return 0}
  if {$lay($w) eq ""} {return 0}
  set m [expr {[dict exists $lay($w) in] ? [dict get $lay($w) in] : [parent $w]}]
  mapped $m
}

# Sizes are not computed: the drawing window is exact, the rest is an estimate.
proc faketk::size {w} {
  variable drawing; variable class
  if {$w eq ".drw" || $w eq "."} {
    # the C side knows the real size of the drawing window
    if {[info exists ::cdept::drw(width)]} {
      return [list $::cdept::drw(width) $::cdept::drw(height)]
    }
    return $drawing
  }
  set width [cget $w -width]
  set height [cget $w -height]
  switch -- $class($w) {
    Entry - Text - Listbox - Label - Button - Spinbox - TCombobox - Checkbutton -
    Radiobutton - Message {
      set width [expr {[string is integer -strict $width] && $width > 0 ? $width * 8 : 100}]
      set height [expr {[string is integer -strict $height] && $height > 0 ? $height * 16 : 20}]
    }
    default {
      if {![string is integer -strict $width] || $width <= 0} {set width 200}
      if {![string is integer -strict $height] || $height <= 0} {set height 100}
    }
  }
  list $width $height
}

proc faketk::drawing_size {width height} {
  variable drawing
  set drawing [list $width $height]
  return {}
}

proc faketk::pointer_now {} {
  variable pointer
  if {[info exists ::cdept::drw(pointerx)]} {
    return [list $::cdept::drw(pointerx) $::cdept::drw(pointery)]
  }
  return $pointer
}

proc faketk::pointer {x y} {
  variable pointer
  set pointer [list $x $y]
  return {}
}

# What `winfo id` returns. The C side may replace this proc with the ids it uses.
proc faketk::window_id {w} {
  variable wmi; variable serial
  if {![info exists wmi($w,id)]} {set wmi($w,id) [format 0x%x [expr {0x400000 + [incr serial]}]]}
  return $wmi($w,id)
}

proc faketk::pixels {value} {
  if {[regexp {^(-?[0-9.]+)([cmip]?)$} $value -> n unit]} {
    switch -- $unit {
      c {return [expr {int($n * 37.8)}]}
      m {return [expr {int($n * 3.78)}]}
      i {return [expr {int($n * 96)}]}
      p {return [expr {int($n * 96 / 72.0)}]}
      default {return [expr {int($n)}]}
    }
  }
  return 0
}

proc faketk::rgb {color} {
  if {[regexp -nocase {^#([0-9a-f]+)$} $color -> hex]} {
    set n [expr {[string length $hex] / 3}]
    set out {}
    for {set i 0} {$i < 3} {incr i} {
      set part [string range $hex [expr {$i * $n}] [expr {($i + 1) * $n - 1}]]
      scan $part %x v
      # scale to 16 bit
      switch -- $n {
        1 {set v [expr {$v * 0x1111}]}
        2 {set v [expr {$v * 0x101}]}
        3 {set v [expr {$v * 16 + $v / 256}]}
      }
      lappend out $v
    }
    return $out
  }
  # the C side knows every X colour name, when it is there (drawing mode)
  if {[llength [info commands ::cdept::colour]] && ![catch {::cdept::colour $color} hex]} {
    scan $hex #%2x%2x%2x r g b
    return [list [expr {$r * 257}] [expr {$g * 257}] [expr {$b * 257}]]
  }
  if {[regexp -nocase {^gr[ae]y([0-9]+)$} $color -> pct]} {
    set v [expr {int(65535 * $pct / 100.0)}]
    return [list $v $v $v]
  }
  switch -nocase -- $color {
    white {return {65535 65535 65535}}
    red {return {65535 0 0}}
    green {return {0 65535 0}}
    blue {return {0 0 65535}}
    yellow {return {65535 65535 0}}
    cyan {return {0 65535 65535}}
    magenta {return {65535 0 65535}}
    grey - gray {return {48830 48830 48830}}
    lightgrey - lightgray {return {54227 54227 54227}}
  }
  return {0 0 0}
}

proc ::winfo {sub args} {
  variable ::faketk::class
  set w [lindex $args 0]
  switch -- $sub {
    exists {return [info exists class($w)]}
    children {
      ::faketk::need $w
      return $::faketk::kids($w)
    }
    parent {::faketk::need $w; return [::faketk::parent $w]}
    toplevel {::faketk::need $w; return [::faketk::top $w]}
    class {
      ::faketk::need $w
      if {$w eq "."} {return Xschem}
      return $class($w)
    }
    name {
      ::faketk::need $w
      if {$w eq "."} {return xschem}
      return [string range $w [expr {[string last . $w] + 1}] end]
    }
    ismapped - viewable {
      # The main window is on screen once CDePT gave the drawing window a size.
      if {$w in {. .drw} && [info exists ::cdept::drw(mapped)] && !$::cdept::drw(mapped)} {
        return 0
      }
      return [::faketk::mapped $w]
    }
    width - reqwidth {::faketk::need $w; return [lindex [::faketk::size $w] 0]}
    height - reqheight {::faketk::need $w; return [lindex [::faketk::size $w] 1]}
    x - y - rootx - rooty - vrootx - vrooty {return 0}
    vrootwidth - screenwidth {return 1920}
    vrootheight - screenheight {return 1080}
    screenmmwidth {return 508}
    screenmmheight {return 286}
    pointerx {return [lindex [::faketk::pointer_now] 0]}
    pointery {return [lindex [::faketk::pointer_now] 1]}
    pointerxy {return [::faketk::pointer_now]}
    depth - screendepth {return 24}
    visual - screenvisual {return truecolor}
    visualid {return 0x21}
    visualsavailable {return {{truecolor 24}}}
    cells - screencells {return 256}
    colormapfull {return 0}
    id {::faketk::need $w; return [::faketk::window_id $w]}
    pathname {
      set id [lindex $args end]
      foreach key [array names ::faketk::wmi *,id] {
        if {$::faketk::wmi($key) == $id} {return [string range $key 0 end-3]}
      }
      return {}
    }
    containing {return {}}
    geometry {
      ::faketk::need $w
      lassign [::faketk::size $w] width height
      return ${width}x${height}+0+0
    }
    manager {
      ::faketk::need $w
      if {$class($w) eq "Toplevel"} {return wm}
      set l $::faketk::lay($w)
      if {$l eq ""} {return {}}
      return [dict get $l manager]
    }
    rgb {return [::faketk::rgb [lindex $args 1]]}
    pixels - fpixels {return [::faketk::pixels [lindex $args 1]]}
    server {return "X11R0 faketk"}
    screen {return :0.0}
    interps {return xschem}
    atom {return 1}
    atomname {return ATOM}
    default {
      ::faketk::todo "winfo $sub"
      return {}
    }
  }
}

proc ::focus {args} {
  variable ::faketk::focus
  if {![llength $args]} {return $focus}
  set w [lindex $args end]
  switch -- [lindex $args 0] {
    -displayof {return $focus}
    -lastfor {return $w}
  }
  if {$w eq ""} {return {}}
  if {[::faketk::exists $w] && $focus ne $w} {
    set old $focus
    set focus $w
    foreach x [list $old $w] {
      if {[::faketk::exists $x] && [::faketk::top $x] ne "."} {::faketk::mark win,[::faketk::top $x]}
    }
  }
  return {}
}

proc ::grab {args} {
  variable ::faketk::grabbed
  set sub [lindex $args 0]
  switch -- $sub {
    current {return $grabbed}
    status {return [expr {$grabbed eq [lindex $args 1] ? "local" : "none"}]}
    release {
      if {$grabbed eq [lindex $args 1]} {
        set grabbed {}
        if {[::faketk::exists [lindex $args 1]]} {::faketk::mark win,[::faketk::top [lindex $args 1]]}
      }
      return {}
    }
    set {set args [lrange $args 1 end]}
  }
  set w [lindex $args end]
  if {[::faketk::exists $w]} {
    set grabbed $w
    ::faketk::mark win,[::faketk::top $w]
  }
  return {}
}

foreach cmd {raise lower bell send tk_setPalette tk_bisque tk_focusFollowsMouse} {
  proc ::$cmd {args} {return {}}
}
proc ::tk_focusNext {w} {return $w}
proc ::tk_focusPrev {w} {return $w}

# ---------------------------------------------------------------------------------
# Waiting
# ---------------------------------------------------------------------------------

if {![llength [info commands ::faketk::tcl_vwait]]} {
  rename ::vwait ::faketk::tcl_vwait
  rename ::update ::faketk::tcl_update
}

# Reports go out before the program blocks, so that CDePT shows what is waited for.
proc ::vwait {name} {
  ::faketk::flush
  ::faketk::tcl_vwait $name
}

proc ::update {args} {
  ::faketk::flush
  ::faketk::tcl_update {*}$args
}

# The toplevel a `tkwait variable` most likely waits for: the one holding the grab,
# else the newest one that is shown.
proc faketk::waited_toplevel {} {
  variable grabbed; variable toporder; variable wmi
  if {$grabbed ne "" && [exists $grabbed] && [top $grabbed] ne "."} {return [top $grabbed]}
  foreach t [lreverse $toporder] {
    if {[exists $t] && [mapped $t] && ![string match *.balloon $t]} {return $t}
  }
  return {}
}

# Put the label texts of a window nobody can answer into the log, and destroy it.
proc faketk::starting_dismiss {t} {
  variable logtext
  if {$t eq "." || ![exists $t]} return
  set text {}
  set todo [list $t]
  while {[llength $todo]} {
    set todo [lassign $todo w]
    if {![catch {$w cget -text} label] && [string trim $label] ne "" \
        && [winfo class $w] in {Label Message}} {
      append text [string trim $label] \n
    }
    catch {lappend todo {*}[winfo children $w]}
  }
  if {$text ne ""} {
    puts stderr "xschem asked while starting (not answered): [string trimright $text]"
    append logtext $text
  }
  catch {destroy $t}
}

proc ::tkwait {what name} {
  variable ::faketk::waits; variable ::faketk::waitvars; variable ::faketk::serial
  switch -- $what {
    window {
      ::faketk::need $name
      if {[info exists ::cdept::starting] && $::cdept::starting} {
        # xschem is still starting: nobody is connected yet who could answer (see
        # faketk::special_wait). Log what the window says and close it unanswered;
        # xschem asks again when it matters (e.g. "execute scripts embedded in
        # schematics?" comes back with the first schematic that has one).
        ::faketk::starting_dismiss [::faketk::top $name]
        return {}
      }
      set var ::faketk::wv[incr serial]
      set $var 0
      lappend waitvars($name) $var
      set t [::faketk::top $name]
      if {$t ne "."} {
        if {![info exists waits($t)]} {set waits($t) 0}
        incr waits($t)
        ::faketk::mark win,$t
      }
      ::faketk::flush
      ::faketk::tcl_vwait $var
      unset -nocomplain $var
      if {$t ne "." && [info exists waits($t)]} {
        incr waits($t) -1
        ::faketk::mark win,$t
      }
    }
    variable {
      set t [::faketk::waited_toplevel]
      if {$t ne ""} {
        if {![info exists waits($t)]} {set waits($t) 0}
        incr waits($t)
        ::faketk::mark win,$t
      }
      ::faketk::flush
      ::faketk::tcl_vwait $name
      if {$t ne "" && [info exists waits($t)]} {
        incr waits($t) -1
        ::faketk::mark win,$t
      }
    }
    visibility {
      ::faketk::flush
    }
    default {
      return -code error "bad option \"$what\": must be variable, visibility, or window"
    }
  }
  return {}
}

# ---------------------------------------------------------------------------------
# Images
# ---------------------------------------------------------------------------------

proc ::image {sub args} {
  variable ::faketk::img; variable ::faketk::serial
  switch -- $sub {
    create {
      set type [lindex $args 0]
      set rest [lrange $args 1 end]
      if {[llength $rest] % 2} {
        set name [lindex $rest 0]
        set rest [lrange $rest 1 end]
      } else {
        set name image[incr serial]
      }
      set img($name) [dict create type $type data {} format {} file {}]
      interp alias {} ::$name {} ::faketk::image_cmd $name
      ::faketk::image_cmd $name configure {*}$rest
      return $name
    }
    delete {
      foreach name $args {
        unset -nocomplain img($name)
        catch {interp alias {} ::$name {}}
      }
      return {}
    }
    names {return [array names img]}
    type {return [dict get $img([lindex $args 0]) type]}
    width {return [lindex [::faketk::image_size [lindex $args 0]] 0]}
    height {return [lindex [::faketk::image_size [lindex $args 0]] 1]}
    inuse {return 0}
    types {return {photo bitmap}}
  }
  return {}
}

# Base64 as one line.
proc faketk::b64 {data} {
  regsub -all {\s+} $data {} data
  return $data
}

proc faketk::image_cmd {name sub args} {
  variable img
  if {![info exists img($name)]} {
    return -code error "image \"$name\" doesn't exist"
  }
  switch -- $sub {
    configure {
      foreach {o v} $args {
        switch -- $o {
          -data {dict set img($name) data [b64 $v]; dict set img($name) file {}}
          -file {dict set img($name) file $v; dict set img($name) data {}}
          -format {dict set img($name) format $v}
          default {dict set img($name) $o $v}
        }
      }
      mark images
    }
    cget {
      set o [lindex $args 0]
      switch -- $o {
        -data {return [dict get $img($name) data]}
        -file {return [dict get $img($name) file]}
        -format {return [dict get $img($name) format]}
      }
      if {[dict exists $img($name) $o]} {return [dict get $img($name) $o]}
      return {}
    }
    put {
      # a whole encoded picture; rows of colours (pixel drawing) are not kept
      set data [lindex $args 0]
      if {[regexp {^\s*[A-Za-z0-9+/=\s]+$} $data] && [string length $data] > 16} {
        dict set img($name) data [b64 $data]
        dict set img($name) file {}
        mark images
      } else {
        todo "image put with colour rows"
      }
    }
    copy {
      set src [lindex $args 0]
      if {[info exists img($src)]} {
        dict set img($name) data [dict get $img($src) data]
        dict set img($name) file [dict get $img($src) file]
        dict set img($name) format [dict get $img($src) format]
        mark images
      }
    }
    read {
      dict set img($name) file [lindex $args 0]
      dict set img($name) data {}
      mark images
    }
    blank {
      dict set img($name) data {}
      dict set img($name) file {}
    }
    width {return [lindex [image_size $name] 0]}
    height {return [lindex [image_size $name] 1]}
    data {return [image_data $name]}
    get {return {0 0 0}}
    write - redither - transparency {}
    default {todo "image $sub"}
  }
  return {}
}

# The encoded picture, base64.
proc faketk::image_data {name} {
  variable img
  if {![info exists img($name)]} {return {}}
  set data [dict get $img($name) data]
  if {$data ne ""} {return $data}
  set file [dict get $img($name) file]
  if {$file ne "" && ![catch {open $file rb} fd]} {
    set bytes [read $fd]
    close $fd
    return [binary encode base64 $bytes]
  }
  return {}
}

proc faketk::image_format {name data} {
  variable img
  switch -glob -- $data {
    R0lGOD* {return gif}
    iVBOR* {return png}
    /9j/* {return jpeg}
    Qk* {return bmp}
    PHN2Z* - PD94b* {return svg}
  }
  set f [string tolower [lindex [dict get $img($name) format] 0]]
  if {$f ne ""} {return $f}
  return [dict get $img($name) type]
}

proc faketk::image_size {name} {
  set data [image_data $name]
  if {$data eq ""} {return {0 0}}
  if {[catch {binary decode base64 [string range $data 0 31]} head]} {return {16 16}}
  if {[string match GIF8* $head]} {
    binary scan $head @6ss width height
    return [list [expr {$width & 0xffff}] [expr {$height & 0xffff}]]
  }
  if {[string match "\x89PNG*" $head]} {
    binary scan $head @16II width height
    return [list $width $height]
  }
  return {16 16}
}

# ---------------------------------------------------------------------------------
# Clipboard, selection, option database, fonts, tk
# ---------------------------------------------------------------------------------

proc ::clipboard {sub args} {
  variable ::faketk::clip
  switch -- $sub {
    clear {set clip {}}
    append {
      set i 0
      while {$i < [llength $args]} {
        set a [lindex $args $i]
        if {$a eq "--"} {incr i; break}
        if {$a in {-displayof -format -type}} {incr i 2; continue}
        break
      }
      append clip [lindex $args $i]
      ::faketk::mark clipboard
    }
    get {
      if {$clip eq ""} {
        return -code error "CLIPBOARD selection doesn't exist or form \"STRING\" not defined"
      }
      return $clip
    }
  }
  return {}
}

proc ::selection {sub args} {
  variable ::faketk::clip; variable ::faketk::primary; variable ::faketk::wmi
  set which PRIMARY
  set rest {}
  for {set i 0} {$i < [llength $args]} {incr i} {
    set a [lindex $args $i]
    switch -- $a {
      -selection {incr i; set which [lindex $args $i]}
      -displayof - -type - -format - -command {incr i; lappend rest $a [lindex $args $i]}
      default {lappend rest $a}
    }
  }
  switch -- $sub {
    get {
      if {$which eq "CLIPBOARD"} {return [::clipboard get]}
      # an owner with a handler provides the text itself
      if {[info exists wmi(selection,owner)] && [info exists wmi(selection,handler,$wmi(selection,owner))]} {
        if {![catch {uplevel #0 $wmi(selection,handler,$wmi(selection,owner)) 0 1000000} text]} {
          return $text
        }
      }
      if {$primary eq ""} {
        return -code error "PRIMARY selection doesn't exist or form \"STRING\" not defined"
      }
      return $primary
    }
    clear {
      if {$which eq "CLIPBOARD"} {set clip {}} else {set primary {}; unset -nocomplain wmi(selection,owner)}
    }
    own {
      set w [lindex $rest end]
      if {$w eq "" || [string match -* $w]} {
        return [expr {[info exists wmi(selection,owner)] ? $wmi(selection,owner) : ""}]
      }
      set wmi(selection,owner) $w
    }
    handle {
      # selection handle ?-type t? window command
      set positional {}
      foreach {a b} $rest {
        if {[string match -* $a]} continue
        lappend positional $a
        if {$b ne ""} {lappend positional $b}
      }
      lassign $positional w command
      if {$w ne ""} {set wmi(selection,handler,$w) $command}
    }
  }
  return {}
}

namespace eval ::tk {}
proc ::tk::GetSelection {w which} {
  if {$which eq "CLIPBOARD"} {return [::clipboard get]}
  return [::selection get]
}

proc ::option {sub args} {
  variable ::faketk::options
  switch -- $sub {
    add {set options([string tolower [lindex $args 0]]) [lindex $args 1]}
    get {
      lassign $args w name class
      foreach key [list *[string tolower $name] *[string tolower $class]] {
        if {$key ne "*" && [info exists options($key)]} {return $options($key)}
      }
      return {}
    }
    clear {array unset options}
    readfile {}
  }
  return {}
}

namespace eval faketk {
  variable fontdefault {-family sans-serif -size 10 -weight normal -slant roman -underline 0 -overstrike 0}
  foreach name {TkDefaultFont TkTextFont TkFixedFont TkMenuFont TkHeadingFont
                TkCaptionFont TkSmallCaptionFont TkIconFont TkTooltipFont fixed} {
    set fonts($name) $fontdefault
  }
  dict set fonts(TkFixedFont) -family monospace
  dict set fonts(fixed) -family monospace
  variable scaling 1.3333333333333333
}

# The options of a font given by name or as a description list.
proc faketk::font_options {font} {
  variable fonts; variable fontdefault
  if {[info exists fonts($font)]} {return [dict merge $fontdefault $fonts($font)]}
  set d $fontdefault
  if {[string match -* [lindex $font 0]]} {
    catch {set d [dict merge $d $font]}
  } else {
    if {[lindex $font 0] ne ""} {dict set d -family [lindex $font 0]}
    if {[string is integer -strict [lindex $font 1]]} {dict set d -size [lindex $font 1]}
    foreach style [lrange $font 2 end] {
      switch -- $style {
        bold {dict set d -weight bold}
        italic {dict set d -slant italic}
        underline {dict set d -underline 1}
        overstrike {dict set d -overstrike 1}
      }
    }
  }
  return $d
}

proc ::font {sub args} {
  variable ::faketk::fonts; variable ::faketk::serial
  switch -- $sub {
    create {
      if {[llength $args] % 2} {
        set name [lindex $args 0]
        set args [lrange $args 1 end]
      } else {
        set name font[incr serial]
      }
      if {[info exists fonts($name)]} {
        return -code error "named font \"$name\" already exists"
      }
      set fonts($name) $args
      return $name
    }
    configure {
      set name [lindex $args 0]
      if {![info exists fonts($name)]} {
        return -code error "named font \"$name\" doesn't exist"
      }
      set rest [lrange $args 1 end]
      if {![llength $rest]} {return [::faketk::font_options $name]}
      if {[llength $rest] == 1} {return [dict get [::faketk::font_options $name] [lindex $rest 0]]}
      set fonts($name) [dict merge $fonts($name) $rest]
      return {}
    }
    actual {
      set d [::faketk::font_options [lindex $args 0]]
      foreach a [lrange $args 1 end] {
        if {[dict exists $d $a]} {return [dict get $d $a]}
      }
      return $d
    }
    measure {
      set d [::faketk::font_options [lindex $args 0]]
      set size [expr {abs([dict get $d -size])}]
      if {$size == 0} {set size 10}
      return [expr {int(ceil([string length [lindex $args end]] * $size * 0.8))}]
    }
    metrics {
      set d [::faketk::font_options [lindex $args 0]]
      set size [expr {abs([dict get $d -size])}]
      if {$size == 0} {set size 10}
      set m [dict create -ascent [expr {int($size * 1.2)}] -descent [expr {int($size * 0.4)}] \
               -linespace [expr {int($size * 1.6)}] \
               -fixed [expr {[dict get $d -family] in {monospace fixed Courier courier}}]]
      foreach a [lrange $args 1 end] {
        if {[dict exists $m $a]} {return [dict get $m $a]}
      }
      return $m
    }
    names {return [array names fonts]}
    families {return {sans-serif serif monospace}}
    delete {foreach name $args {unset -nocomplain fonts($name)}}
  }
  return {}
}

proc ::tk {sub args} {
  switch -- $sub {
    scaling {
      set v [lindex $args end]
      if {[string is double -strict $v]} {set ::faketk::scaling $v; return {}}
      return $::faketk::scaling
    }
    windowingsystem {return x11}
    appname {return xschem}
    inactive {return 0}
    useinputmethods {return 0}
    default {return {}}
  }
}

# ---------------------------------------------------------------------------------
# Widget constructors
# ---------------------------------------------------------------------------------

foreach {cmd cls} {
  frame Frame labelframe Labelframe label Label message Message button Button
  checkbutton Checkbutton radiobutton Radiobutton entry Entry spinbox Spinbox
  listbox Listbox scale Scale scrollbar Scrollbar canvas Canvas menubutton Menubutton
  panedwindow Panedwindow
} {
  proc ::$cmd {w args} "return \[::faketk::create $cls \$w \$args\]"
}

proc ::toplevel {w args} {
  ::faketk::create Toplevel $w $args
  lappend ::faketk::toporder $w
  ::faketk::mark win,$w
  return $w
}

proc ::text {w args} {
  ::faketk::create Text $w $args
  set ::faketk::marks($w,insert) 0
  set ::faketk::marks($w,current) 0
  return $w
}

proc ::menu {w args} {
  ::faketk::create Menu $w $args
  set ::faketk::menu($w) {}
  if {[::faketk::cget $w -tearoff]} {
    lappend ::faketk::menu($w) [dict create type tearoff]
  }
  return $w
}

namespace eval ::ttk {}
foreach {cmd cls} {
  frame Frame labelframe Labelframe label Label button Button checkbutton Checkbutton
  radiobutton Radiobutton entry Entry spinbox Spinbox scale Scale scrollbar Scrollbar
  menubutton Menubutton panedwindow Panedwindow combobox TCombobox
  separator TSeparator sizegrip TSizegrip progressbar TProgressbar notebook TNotebook
  treeview Treeview
} {
  proc ::ttk::$cmd {w args} "return \[::faketk::create $cls \$w \$args\]"
}
proc ::ttk::style {sub args} {
  switch -- $sub {
    theme {
      if {[lindex $args 0] eq "names"} {return default}
      if {[llength $args] == 1} {return default}
    }
  }
  return {}
}

# ---------------------------------------------------------------------------------
# Tk's own dialogs
# ---------------------------------------------------------------------------------

# A toplevel whose root widget is reported with another class and extra options.
proc faketk::special {w rootclass title options} {
  variable wmi
  ::toplevel $w
  set wmi($w,title) $title
  set wmi($w,rootclass) $rootclass
  set wmi($w,rootopts) $options
  return $w
}

# Wait until the dialog has a result, and return it. A dialog destroyed from
# elsewhere gives the empty result.
proc faketk::special_wait {w} {
  variable spec; variable waits; variable grabbed
  if {[info exists ::cdept::starting] && $::cdept::starting} {
    # xschem is still starting (the C side says so): nobody is connected yet who
    # could answer, and waiting would stop the start for good. Put the message in the
    # log and go on with the default answer. xschem asks here when the start-up file
    # has an error.
    variable wmi; variable logtext
    set o $wmi($w,rootopts)
    set text {}
    foreach key {message detail} {
      if {[dict exists $o $key] && [dict get $o $key] ne ""} {
        append text [string trimright [dict get $o $key]] \n
      }
    }
    if {$text ne ""} {
      puts stderr [string trimright $text]
      append logtext $text
      mark log
    }
    set result [expr {[dict exists $o default] ? [dict get $o default] : ""}]
    ::destroy $w
    return $result
  }
  set waits($w) 1
  # as Tk's dialogs do: the C side drops input for the drawing window meanwhile
  set before $grabbed
  set grabbed $w
  set spec($w,result) {}
  set spec($w,done) 0
  mark win,$w
  flush
  tcl_vwait ::faketk::spec($w,done)
  set result $spec($w,result)
  unset -nocomplain spec($w,result) spec($w,done)
  ::destroy $w
  if {$before ne "" && [exists $before]} {set grabbed $before}
  return $result
}

proc faketk::special_done {w result} {
  variable spec
  if {![info exists spec($w,done)]} return
  set spec($w,result) $result
  set spec($w,done) 1
}

proc ::tk_messageBox {args} {
  set o [dict create -type ok -icon info -message {} -detail {} -title {} -default {} -parent .]
  foreach {k v} $args {dict set o $k $v}
  set w .faketk_msg[incr ::faketk::serial]
  set buttons [dict get {
    ok {ok OK}
    okcancel {ok OK cancel Cancel}
    yesno {yes Yes no No}
    yesnocancel {yes Yes no No cancel Cancel}
    retrycancel {retry Retry cancel Cancel}
    abortretryignore {abort Abort retry Retry ignore Ignore}
  } [dict get $o -type]]
  set default [dict get $o -default]
  if {$default eq ""} {set default [lindex $buttons 0]}
  ::faketk::special $w MessageBox [dict get $o -title] [dict create \
    icon [dict get $o -icon] type [dict get $o -type] message [dict get $o -message] \
    detail [dict get $o -detail] default $default]
  foreach {name label} $buttons {
    ::button $w.$name -text $label -command [list ::faketk::special_done $w $name]
    ::pack $w.$name -side left
  }
  # closing the window: cancel where there is one
  set closing [expr {[dict exists $buttons cancel] ? "cancel" : [lindex $buttons end-1]}]
  ::wm protocol $w WM_DELETE_WINDOW [list ::faketk::special_done $w $closing]
  ::bind $w <Return> [list ::faketk::special_done $w $default]
  ::bind $w <Escape> [list ::faketk::special_done $w $closing]
  ::focus $w.$default
  return [::faketk::special_wait $w]
}

proc faketk::file_dialog {mode arglist} {
  set o [dict create -title {} -initialdir {} -initialfile {} -filetypes {} \
           -defaultextension {} -multiple 0 -parent .]
  foreach {k v} $arglist {dict set o $k $v}
  set w .faketk_file[incr ::faketk::serial]
  set dir [dict get $o -initialdir]
  if {$dir eq ""} {set dir [pwd]}
  special $w FileDialog [dict get $o -title] [dict create \
    mode $mode initialdir $dir initialfile [dict get $o -initialfile] \
    filetypes [dict get $o -filetypes] defaultextension [dict get $o -defaultextension] \
    multiple [expr {[string is true -strict [dict get $o -multiple]] ? 1 : 0}]]
  ::wm protocol $w WM_DELETE_WINDOW [list ::faketk::special_done $w {}]
  set result [special_wait $w]
  if {$result ne "" && [string is true -strict [dict get $o -multiple]]} {
    return [list $result]
  }
  return $result
}

proc ::tk_getOpenFile {args} {::faketk::file_dialog open $args}
proc ::tk_getSaveFile {args} {::faketk::file_dialog save $args}
proc ::tk_chooseDirectory {args} {::faketk::file_dialog directory $args}

proc ::tk_chooseColor {args} {
  set o [dict create -initialcolor {} -title Color -parent .]
  foreach {k v} $args {dict set o $k $v}
  set w .faketk_color[incr ::faketk::serial]
  ::faketk::special $w ColorDialog [dict get $o -title] \
    [dict create initialcolor [dict get $o -initialcolor]]
  ::label $w.l -text "Color (#rrggbb or a name):"
  ::entry $w.e
  $w.e insert 0 [dict get $o -initialcolor]
  ::frame $w.b
  ::button $w.b.ok -text OK -command "::faketk::special_done $w \[$w.e get\]"
  ::button $w.b.cancel -text Cancel -command [list ::faketk::special_done $w {}]
  ::pack $w.l $w.e -side top -fill x
  ::pack $w.b -side top
  ::pack $w.b.ok $w.b.cancel -side left
  ::wm protocol $w WM_DELETE_WINDOW [list ::faketk::special_done $w {}]
  ::bind $w <Return> "$w.b.ok invoke"
  ::bind $w <Escape> "$w.b.cancel invoke"
  ::focus $w.e
  return [::faketk::special_wait $w]
}

proc ::tk_dialog {w title text bitmap default args} {
  catch {::destroy $w}
  ::faketk::special $w MessageBox $title [dict create icon $bitmap type custom \
    message $text detail {} default $default]
  set i 0
  foreach label $args {
    ::button $w.button$i -text $label -command [list ::faketk::special_done $w $i]
    ::pack $w.button$i -side left
    incr i
  }
  ::wm protocol $w WM_DELETE_WINDOW [list ::faketk::special_done $w -1]
  return [::faketk::special_wait $w]
}

proc ::tk_popup {m x y args} {
  ::faketk::need $m
  $m post $x $y
  return {}
}

# ---------------------------------------------------------------------------------
# What the user did (from CDePT)
# ---------------------------------------------------------------------------------

namespace eval faketk {
  # keysym numbers of the keys that have no character, for %N
  variable keynum
  array set keynum {
    Return 65293 Escape 65307 Tab 65289 BackSpace 65288 Delete 65535 Insert 65379
    Home 65360 End 65367 Prior 65365 Next 65366 Left 65361 Up 65362 Right 65363
    Down 65364 space 32 KP_Enter 65421 ISO_Left_Tab 65056
  }
  variable keychar
  array set keychar [list Return \r Escape \x1b Tab \t BackSpace \b Delete \x7f space " " KP_Enter \r]
  variable keynames
  array set keynames {
    " " space - minus + plus , comma . period / slash \\ backslash ; semicolon
    : colon ' apostrophe \" quotedbl ! exclam ? question = equal < less > greater
    ( parenleft ) parenright [ bracketleft ] bracketright \{ braceleft \} braceright
    _ underscore * asterisk & ampersand % percent $ dollar # numbersign @ at
    ^ asciicircum ~ asciitilde | bar ` grave
  }
}

# The user changed something in a window CDePT shows (typed, selected, moved the
# focus): CDePT has that state already. Take it as reported, so that only what
# xschem's code changes in response is sent back.
proc faketk::absorb {w} {
  variable shown; variable last
  if {![exists $w]} return
  set t [top $w]
  if {$t ne "." && [info exists shown($t)] && $shown($t) eq "window"} {
    set last(win,$t) [json_window $t]
  }
}

# The buttons a popup made of a toplevel shows, in order.
proc faketk::popup_buttons {t} {
  variable class; variable slaves
  set out {}
  if {[info exists slaves($t)]} {
    foreach s $slaves($t) {
      if {[info exists class($s)] && $class($s) eq "Button"} {lappend out $s}
    }
  }
  return $out
}

proc faketk::event {kind args} {
  set code [catch {event_do $kind $args} result]
  if {$code == 1} {problem $result}
  flush
  return {}
}

proc faketk::event_do {kind arglist} {
  variable class; variable wmi; variable popup; variable sel; variable marks
  variable content; variable focus
  switch -- $kind {
    menu {
      lassign $arglist m index
      if {![exists $m]} return
      if {$class($m) eq "Menu"} {
        if {$popup eq $m} {set popup {}}
        $m invoke $index
      } else {
        set b [lindex [popup_buttons $m] $index]
        if {$b ne ""} {$b invoke}
      }
    }
    popup_closed {
      set p $popup
      set popup {}
      if {$p ne "" && [exists $p] && $class($p) eq "Toplevel"} {::destroy $p}
    }
    invoke {
      lassign $arglist w
      if {![exists $w]} return
      switch -- $class($w) {
        Button - Checkbutton - Radiobutton - Menubutton {$w invoke}
        Spinbox {$w invoke buttonup}
        default {
          # a click on something else: its button bindings
          dispatch $w <ButtonPress-1> {b 1}
          dispatch $w <ButtonRelease-1> {b 1}
        }
      }
    }
    text {
      lassign $arglist w value
      if {![exists $w]} return
      if {[info exists wmi($w,rootclass)]} {
        # answer of a file dialog (or of any special window): its result
        special_done $w $value
        return
      }
      switch -- $class($w) {
        Text {
          set content($w) $value
          set len [string length $value]
          foreach key [array names marks $w,*] {
            if {$key ne "$w,modified" && $marks($key) > $len} {set marks($key) $len}
          }
          set marks($w,insert) $len
          array unset ::faketk::tags $w,sel
          set marks($w,modified) 1
          touch $w
          absorb $w
          fire $w {<<Modified>>} {}
        }
        Scale {$w set $value}
        default {
          set_text $w $value
          set sel($w) {}
          set marks($w,insert) [string length $value]
          absorb $w
          if {$class($w) eq "TCombobox" && $value in [cget $w -values]} {
            fire $w {<<ComboboxSelected>>} {}
          }
          if {$class($w) eq "Spinbox"} {
            set command [cget $w -command]
            if {$command ne ""} {run [string map [list %s [list $value] %W $w %d up] $command]}
          }
        }
      }
      # code that follows typing (filters, live checks) hangs on the key release
      if {[exists $w] && [binding $w <KeyRelease>] ne ""} {
        run [subst_event [binding $w <KeyRelease>] [dict create W $w T 3]]
      }
    }
    select {
      lassign $arglist w indices
      if {![exists $w]} return
      set sel($w) $indices
      if {[llength $indices]} {
        set marks($w,active) [lindex $indices 0]
        set marks($w,anchor) [lindex $indices 0]
      }
      touch $w
      absorb $w
      fire $w {<<ListboxSelect>>} {}
      if {[exists $w]} {
        dispatch $w <ButtonPress-1> {b 1}
        if {[exists $w]} {dispatch $w <ButtonRelease-1> {b 1}}
      }
    }
    activate {
      # The second click of a double click. The first one was reported with `select`
      # and ran the click bindings; here only the selection is made sure of, and the
      # double click binding runs.
      lassign $arglist w index
      if {![exists $w]} return
      if {$sel($w) ne [list $index]} {
        set sel($w) [list $index]
        touch $w
        fire $w {<<ListboxSelect>>} {}
      }
      if {![exists $w]} return
      set marks($w,active) $index
      set marks($w,anchor) $index
      dispatch $w <Double-ButtonPress-1> {b 1}
    }
    key {
      lassign $arglist w spec
      if {![exists $w]} return
      key $w $spec
      # xschem takes over a status bar entry (SNAP, GRID) when the pointer leaves it;
      # here Return ends the edit.
      if {$spec eq "Return" && [exists $w] && $class($w) eq "Entry"
          && [string match *.statusbar.* $w]} {
        dispatch $w <Leave> {}
      }
    }
    focus {
      lassign $arglist w
      if {![exists $w] || $focus eq $w} return
      set old $focus
      ::focus $w
      absorb $w
      if {[exists $old]} {
        set script [binding $old <FocusOut>]
        if {$script ne ""} {run [subst_event $script [dict create W $old T 10]]}
      }
      if {[exists $w]} {
        set script [binding $w <FocusIn>]
        if {$script ne ""} {run [subst_event $script [dict create W $w T 9]]}
      }
    }
    close {
      lassign $arglist w
      if {![exists $w]} return
      if {[info exists wmi($w,protocol,WM_DELETE_WINDOW)] && $wmi($w,protocol,WM_DELETE_WINDOW) ne ""} {
        run $wmi($w,protocol,WM_DELETE_WINDOW)
      } else {
        ::destroy $w
      }
    }
    default {
      return -code error "unknown event kind \"$kind\""
    }
  }
}

# A key pressed (and released) in a widget. `spec` is a Tk keysym name, with the
# modifiers that were down in front: Return, Control-Return, Shift-Tab, a, A, ...
proc faketk::key {w spec} {
  variable keynum; variable keychar; variable keynames
  set mods {}
  set state 0
  while {[regexp {^(Control|Shift|Alt|Meta|Mod1|Mod4|Lock)-(.+)$} $spec -> mod spec]} {
    switch -- $mod {
      Shift {incr state 1; lappend mods Shift}
      Lock {incr state 2}
      Control {incr state 4; lappend mods Control}
      Alt - Meta - Mod1 {incr state 8; lappend mods Alt}
      Mod4 {incr state 64; lappend mods Mod4}
    }
  }
  set keysym $spec
  if {[string length $spec] == 1} {
    set char $spec
    if {[info exists keynames($spec)]} {set keysym $keynames($spec)}
    scan $spec %c number
  } else {
    set char [expr {[info exists keychar($spec)] ? $keychar($spec) : ""}]
    set number [expr {[info exists keynum($spec)] ? $keynum($spec) : 0}]
    if {[regexp {^F([0-9]+)$} $spec -> n]} {set number [expr {65469 + $n}]}
  }
  set fields [dict create W $w K $keysym A $char N $number k $number s $state]
  set mods [lsort -unique $mods]
  foreach {type num} {KeyPress 2 KeyRelease 3} {
    if {![exists $w]} return
    set candidates {}
    if {[llength $mods]} {lappend candidates "<[join [concat $mods [list $type $keysym]] -]>"}
    # as in Tk, a binding that names no modifier also matches when one is down
    lappend candidates "<$type-$keysym>"
    if {[llength $mods]} {lappend candidates "<[join [concat $mods [list $type]] -]>"}
    lappend candidates "<$type>"
    dict set fields T $num
    fire $w $candidates $fields
  }
}

# ---------------------------------------------------------------------------------
# Reports to CDePT
# ---------------------------------------------------------------------------------

namespace eval faketk {
  variable toporder {}
  variable popup_pending 0
  variable imgneed {}
  # options passed on in widget snapshots, without the dash
  variable reported {
    width height wrap image justify show from to increment resolution orient anchor
    compound state relief background foreground font selectmode underline label
    digits length padx pady borderwidth values tabs
  }
}

# Remember that a report uses an image, and return its name.
proc faketk::use_image {name} {
  variable img; variable imgneed
  if {$name ne "" && [info exists img($name)] && $name ni $imgneed} {lappend imgneed $name}
  return $name
}

# The help text xschem attached to a widget with `balloon`.
proc faketk::tooltip {w} {
  set script [binding $w <Enter>]
  if {[string first balloon_show $script] < 0} {return {}}
  # the script is: after <delay> balloon_show %W {<help as a one-item list>} <pos>
  if {[catch {lindex [lindex $script 4] 0} help]} {return {}}
  return $help
}

# The colour xschem gave a menu entry to show a state (Netlist, Simulate and Waves go
# red, green or orange): its -background when that is not the ordinary one, which
# xschem keeps in `simulate_bg`.
proc faketk::entry_color {e} {
  if {![dict exists $e -background]} {return {}}
  set color [dict get $e -background]
  if {[info exists ::simulate_bg] && $color eq $::simulate_bg} {return {}}
  return $color
}

proc faketk::menu_items {m {depth 0}} {
  variable menu; variable class
  set out {}
  if {![info exists menu($m)] || $depth > 8} {return $out}
  set i -1
  foreach e $menu($m) {
    incr i
    set type [dict get $e type]
    if {$type eq "tearoff"} continue
    set pairs {}
    switch -- $type {
      command {lappend pairs kind [jstr command]}
      checkbutton {lappend pairs kind [jstr check]}
      radiobutton {lappend pairs kind [jstr radio]}
      separator {lappend pairs kind [jstr separator]}
      cascade {lappend pairs kind [jstr cascade]}
      default {continue}
    }
    if {[dict exists $e -label]} {lappend pairs label [jstr [dict get $e -label]]}
    if {[dict exists $e -accelerator] && [dict get $e -accelerator] ne ""} {
      lappend pairs accelerator [jstr [dict get $e -accelerator]]
    }
    lappend pairs menu [jstr $m] index $i
    if {[dict exists $e -state] && [dict get $e -state] eq "disabled"} {
      lappend pairs enabled false
    }
    if {$type in {checkbutton radiobutton} && [m_checked $e]} {
      lappend pairs checked true
    }
    if {[dict exists $e -image]} {
      set image [use_image [dict get $e -image]]
      if {$image ne ""} {lappend pairs image [jstr $image]}
    }
    set color [entry_color $e]
    if {$color ne ""} {lappend pairs color [jstr $color]}
    if {$type eq "cascade" && [dict exists $e -menu]} {
      set sub [dict get $e -menu]
      if {[info exists class($sub)] && $class($sub) eq "Menu"} {
        set post [cget $sub -postcommand]
        if {$post ne ""} {run $post}
        lappend pairs items [jarr [menu_items $sub [expr {$depth + 1}]]]
      }
    }
    lappend out [jobj $pairs]
  }
  return $out
}

proc faketk::json_menu {} {
  variable class
  set m [cget . -menu]
  set items {}
  if {$m ne "" && [info exists class($m)]} {set items [menu_items $m]}
  jobj [list kind [jstr menu] items [jarr $items]]
}

proc faketk::json_toolbar {} {
  variable class; variable slaves; variable lay
  set left {}
  set right {}
  if {[info exists class(.toolbar)] && $lay(.toolbar) ne "" && [info exists slaves(.toolbar)]} {
    foreach s $slaves(.toolbar) {
      if {![info exists class($s)]} continue
      set pairs {}
      switch -- $class($s) {
        Button - Checkbutton - Radiobutton {
          set label [cget $s -text]
          if {$label eq ""} {
            regsub {^\.toolbar\.b?} $s {} label
          }
          lappend pairs path [jstr $s] label [jstr $label]
          set tip [tooltip $s]
          if {$tip ne ""} {lappend pairs tooltip [jstr $tip]}
          set image [use_image [cget $s -image]]
          if {$image ne ""} {lappend pairs image [jstr $image]}
          if {[cget $s -state] eq "disabled"} {lappend pairs enabled false}
          if {$class($s) ne "Button" && [is_on $s]} {lappend pairs checked true}
        }
        default {lappend pairs separator true}
      }
      set side [expr {[dict exists $lay($s) side] ? [dict get $lay($s) side] : "top"}]
      if {$side in {right bottom}} {
        set right [linsert $right 0 [jobj $pairs]]
      } else {
        lappend left [jobj $pairs]
      }
    }
  }
  # xschem's own tabs (several schematics in one xschem), when there is more than
  # one: text buttons after the tool bar, the current one shown pressed.
  set tabs {}
  if {[info exists slaves(.tabs)]} {
    foreach s $slaves(.tabs) {
      if {![info exists class($s)] || ![string match .tabs.x* $s]} continue
      set pairs [list path [jstr $s] label [jstr [cget $s -text]]]
      set tip [tooltip $s]
      if {$tip ne ""} {lappend pairs tooltip [jstr $tip]}
      if {[cget $s -background] in {Palegreen DarkGreen}} {lappend pairs checked true}
      lappend tabs [jobj $pairs]
    }
  }
  if {[llength $tabs] > 1} {
    if {[llength $left]} {lappend left [jobj {separator true}]}
    set left [concat $left $tabs]
  }
  jobj [list kind [jstr toolbar] items [jarr [concat $left $right]]]
}

proc faketk::json_status {} {
  variable class; variable slaves; variable lay
  set fields {}
  if {[info exists class(.statusbar)] && [info exists slaves(.statusbar)]} {
    foreach s $slaves(.statusbar) {
      if {![info exists class($s)]} continue
      if {$class($s) ni {Label Entry Button}} continue
      set pairs [list name [jstr $s] text [jstr [text_of $s]]]
      # SNAP and GRID are entries: the user can type a new value
      if {$class($s) eq "Entry" && [cget $s -state] eq "normal"} {lappend pairs editable true}
      lappend fields [jobj $pairs]
    }
  }
  jobj [list kind [jstr status] fields [jarr $fields]]
}

proc faketk::json_cursor {} {
  variable class
  set name {}
  if {[info exists class(.drw)]} {set name [cget .drw -cursor]}
  if {$name eq ""} {set name [cget . -cursor]}
  if {$name eq ""} {set name arrow}
  jobj [list kind [jstr cursor] name [jstr $name]]
}

# The children of a widget as they are stacked: those its geometry manager holds, in
# order, then the ones not placed anywhere.
proc faketk::shown_children {w} {
  variable kids; variable slaves; variable lay; variable class
  set out {}
  if {[info exists slaves($w)]} {
    foreach s $slaves($w) {
      if {[info exists class($s)]} {lappend out $s}
    }
  }
  foreach k $kids($w) {
    if {$k in $out || $class($k) in {Toplevel Menu}} continue
    if {$lay($k) ne ""} continue  ;# managed by another master (-in)
    lappend out $k
  }
  return $out
}

proc faketk::json_widget {w {depth 0}} {
  variable class; variable opt; variable lay; variable focus; variable sel
  variable wmi; variable reported
  set cls $class($w)
  set options [dict create]
  if {[info exists wmi($w,rootclass)]} {
    set cls $wmi($w,rootclass)
    set options $wmi($w,rootopts)
  }
  set pairs [list path [jstr $w] class [jstr $cls]]
  set text {}
  switch -- $cls {
    Frame - Toplevel - Scrollbar - Canvas - Panedwindow - Listbox {}
    Labelframe {set text [cget $w -text]}
    default {
      if {![info exists wmi($w,rootclass)]} {set text [text_of $w]}
    }
  }
  if {$text ne ""} {lappend pairs text [jstr $text]}
  if {$cls in {Checkbutton Radiobutton} && [is_on $w]} {lappend pairs checked true}
  if {[cget $w -state] eq "disabled"} {lappend pairs enabled false}
  if {$focus eq $w} {lappend pairs focus true}
  switch -- $cls {
    Listbox {
      set items {}
      foreach item [lb_items $w] {lappend items [jstr $item]}
      lappend pairs items [jarr $items]
      set n [llength $items]
      set selection {}
      foreach s [lsort -integer -unique $sel($w)] {
        if {$s >= 0 && $s < $n} {lappend selection $s}
      }
      if {[llength $selection]} {lappend pairs selection [jarr $selection]}
    }
    TCombobox - Spinbox {
      set items {}
      foreach item [cget $w -values] {lappend items [jstr $item]}
      if {[llength $items]} {lappend pairs items [jarr $items]}
    }
  }
  dict for {o v} $opt($w) {
    set name [string range $o 1 end]
    if {$name in $reported || [string match grid* $name]} {
      if {$name eq "values"} continue
      if {$name eq "image"} {use_image $v}
      dict set options $name $v
    }
  }
  set tip [tooltip $w]
  if {$tip ne ""} {dict set options tooltip $tip}
  if {[dict size $options]} {lappend pairs options [jmapobj $options]}
  if {$lay($w) ne ""} {lappend pairs layout [jmapobj $lay($w)]}
  if {$depth < 40} {
    set children {}
    foreach k [shown_children $w] {
      if {[string match *.balloon $k]} continue
      lappend children [json_widget $k [expr {$depth + 1}]]
    }
    if {[llength $children]} {lappend pairs children [jarr $children]}
  }
  jobj $pairs
}

proc faketk::is_modal {t} {
  variable waits; variable grabbed
  if {[info exists waits($t)] && $waits($t) > 0} {return 1}
  expr {$grabbed ne "" && [exists $grabbed] && [top $grabbed] eq $t}
}

proc faketk::json_window {t} {
  variable wmi
  set title [expr {[info exists wmi($t,title)] ? $wmi($t,title) : ""}]
  set window [jobj [list path [jstr $t] title [jstr $title] modal [jbool [is_modal $t]] \
                        root [json_widget $t]]]
  jobj [list kind [jstr window] window $window]
}

# A toplevel without window frame holding only buttons: a menu at the pointer.
proc faketk::is_popup {t} {
  variable wmi; variable kids; variable class
  if {![info exists wmi($t,overrideredirect)] || !$wmi($t,overrideredirect)} {return 0}
  set buttons [popup_buttons $t]
  if {![llength $buttons]} {return 0}
  foreach k $kids($t) {
    if {$class($k) ni {Button Frame}} {return 0}
  }
  return 1
}

proc faketk::json_popup {p} {
  variable class
  set items {}
  if {$class($p) eq "Menu"} {
    set post [cget $p -postcommand]
    if {$post ne ""} {run $post}
    set items [menu_items $p]
  } else {
    set i 0
    foreach b [popup_buttons $p] {
      set pairs [list kind [jstr command] label [jstr [cget $b -text]] menu [jstr $p] index $i]
      if {[cget $b -state] eq "disabled"} {lappend pairs enabled false}
      set image [use_image [cget $b -image]]
      if {$image ne ""} {lappend pairs image [jstr $image]}
      lappend items [jobj $pairs]
      incr i
    }
  }
  jobj [list kind [jstr popup] items [jarr $items]]
}

proc faketk::json_images {names} {
  variable img; variable imgsent
  set images {}
  foreach name $names {
    if {![info exists img($name)]} continue
    set data [image_data $name]
    if {$data eq ""} continue
    if {[info exists imgsent($name)] && $imgsent($name) eq $data} continue
    set imgsent($name) $data
    lappend images [jobj [list name [jstr $name] format [jstr [image_format $name $data]] \
                              data [jstr $data]]]
  }
  if {![llength $images]} {return {}}
  jobj [list kind [jstr images] images [jarr $images]]
}

# Send what changed. Safe to call at any time; does nothing while already running.
proc faketk::flush {} {
  variable pending; variable flushing
  if {$pending} {
    after cancel ::faketk::flush
    set pending 0
  }
  if {$flushing} return
  set flushing 1
  set code [catch {flush_do} err]
  set flushing 0
  if {$code == 1} {puts stderr "faketk: flush: $err"}
  return {}
}

proc faketk::flush_do {} {
  variable dirty; variable last; variable shown; variable class; variable wmi
  variable popup; variable popup_pending; variable imgneed; variable clip
  set todo [array names dirty]
  if {![llength $todo] && !$popup_pending} return
  array unset dirty
  set out {}
  set imgneed {}

  # one report each for the parts of the main window; the same report is not repeated
  foreach {what proc} {menu json_menu toolbar json_toolbar status json_status cursor json_cursor} {
    if {$what ni $todo && !($what eq "toolbar" && "images" in $todo)} continue
    set json [$proc]
    if {![info exists last($what)]} {
      # nothing to say about a part that does not exist yet
      if {$what eq "menu" && [cget . -menu] eq ""} continue
      if {$what eq "toolbar" && ![info exists class(.toolbar)]} continue
      if {$what eq "status" && ![info exists class(.statusbar)]} continue
    }
    if {![info exists last($what)] || $last($what) ne $json} {
      set last($what) $json
      lappend out $json
    }
  }
  if {"title" in $todo && [info exists wmi(.,title)]} {
    set json [jobj [list kind [jstr title] text [jstr $wmi(.,title)]]]
    if {![info exists last(title)] || $last(title) ne $json} {
      set last(title) $json
      lappend out $json
    }
  }

  foreach what [lsort $todo] {
    if {![string match win,* $what]} continue
    set t [string range $what 4 end]
    if {$t eq "." || [string match *.balloon $t]} continue
    set visible [expr {[info exists class($t)] && $class($t) eq "Toplevel" && [mapped $t]}]
    if {!$visible} {
      if {[info exists shown($t)]} {
        if {$shown($t) eq "window"} {
          lappend out [jobj [list kind [jstr window_closed] path [jstr $t]]]
        }
        unset shown($t)
        unset -nocomplain last(win,$t)
      }
      continue
    }
    if {[is_popup $t]} {
      set json [json_popup $t]
      set popup $t
      set shown($t) popup
    } else {
      set json [json_window $t]
      set shown($t) window
    }
    if {![info exists last(win,$t)] || $last(win,$t) ne $json} {
      set last(win,$t) $json
      lappend out $json
    }
  }

  if {$popup_pending} {
    set popup_pending 0
    if {$popup ne "" && [info exists class($popup)] && $class($popup) eq "Menu"} {
      lappend out [json_popup $popup]
    }
  }
  if {"clipboard" in $todo} {
    lappend out [jobj [list kind [jstr clipboard] text [jstr $clip]]]
  }
  if {"log" in $todo} {
    # the info window: what was added to it since the last report
    variable infotext; variable logtext; variable content
    if {[info exists content(.infotext.f1.text)]} {
      set now $content(.infotext.f1.text)
      if {$now ne $infotext} {
        set n [string length $infotext]
        if {$n > 0 && [string equal -length $n $infotext $now]} {
          append logtext [string range $now $n end]
        } else {
          append logtext $now
        }
        set infotext $now
      }
    }
    if {[string trim $logtext] ne ""} {
      lappend out [jobj [list kind [jstr log] text [jstr $logtext]]]
    }
    set logtext {}
  }

  set images [json_images $imgneed]
  if {$images ne ""} {emit $images}
  foreach json $out {emit $json}
}

# ---------------------------------------------------------------------------------
# Start: the main window exists from the beginning
# ---------------------------------------------------------------------------------

namespace eval faketk {
  if {![info exists class(.)]} {
    set class(.) Toplevel
    set kids(.) {}
    set opt(.) {}
    set lay(.) {}
    set content(.) {}
    set items(.) {}
    set sel(.) {}
    set wmi(.,title) xschem
    set wmi(.,state) normal
    interp alias {} . {} ::faketk::w .
  }
}

set ::tk_version 8.6
set ::tk_patchLevel 8.6.14
set ::tk_library {}
set ::tk_strictMotif 0
package provide Tk 8.6.14
# xschem.tcl only builds its windows when the environment names a display.
if {![info exists ::env(DISPLAY)] || $::env(DISPLAY) eq ""} {
  set ::env(DISPLAY) :cdept
}
