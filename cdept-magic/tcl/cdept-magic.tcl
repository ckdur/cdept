# cdept-magic.tcl: CDePT's side of Magic inside the helper (docs/MAGIC.md, section
# "Embedded Magic"). Read before Magic's own magic.tcl; nothing of Magic is changed,
# these are additions CDePT calls, and the report of Magic's text output.
#
# Part of cdept-magic, GPL-2.0-or-later.

namespace eval cdept {}
namespace eval cdept::magic {
  variable logbuf {}
  variable logpending 0
}

# ---- Magic's text output -------------------------------------------------------
# Magic prints through Tcl's `puts` (its Tcl_printf builds "puts -nonewline stdout ..."):
# what goes to stdout or stderr is also reported to CDePT as `log` lines.

rename ::puts ::cdept::magic::tcl_puts
proc ::puts {args} {
  set chan stdout
  set nonl 0
  set a $args
  if {[lindex $a 0] eq "-nonewline"} {set nonl 1; set a [lrange $a 1 end]}
  if {[llength $a] == 2} {set chan [lindex $a 0]; set text [lindex $a 1]} \
  elseif {[llength $a] == 1} {set text [lindex $a 0]} \
  else {return [uplevel 1 [list ::cdept::magic::tcl_puts {*}$args]]}
  if {$chan in {stdout stderr}} {
    ::cdept::magic::log [expr {$nonl ? $text : "$text\n"}]
  }
  uplevel 1 [list ::cdept::magic::tcl_puts {*}$args]
}

proc cdept::magic::log {text} {
  variable logbuf; variable logpending
  append logbuf $text
  if {!$logpending} {
    set logpending 1
    after idle ::cdept::magic::flushlog
  }
}

proc cdept::magic::flushlog {} {
  variable logbuf; variable logpending
  set logpending 0
  set text [string trimright $logbuf "\n"]
  set logbuf {}
  if {$text eq "" || ![llength [info commands ::cdept::ui]]} return
  if {[llength [info commands ::faketk::jstr]]} {
    set j [::faketk::jstr $text]
  } else {
    set j "\"[string map {\\ \\\\ \" \\\" \n \\n \t \\t \r \\r} $text]\""
  }
  ::cdept::ui "{\"kind\":\"log\",\"text\":$j}"
}

# ---- Start-up -------------------------------------------------------------------

# Source Magic's magic.tcl. It ends the process with `exit 1` when magic::initialize
# fails; here that becomes an error with Magic's message.
proc cdept::magic::start {script} {
  rename ::exit ::cdept::magic::tcl_exit
  proc ::exit {{code 0}} {
    error "Magic stopped during start-up (exit $code); see its messages above"
  }
  # Once magic::initialize has made the layout window, and before magic::startup
  # reads the start-up files, Magic's current point must be inside that window:
  # Magic sends a command to the window under it.
  trace add execution ::load leave ::cdept::magic::loaded
  set failed [catch {uplevel #0 [list source $script]} message options]
  catch {trace remove execution ::load leave ::cdept::magic::loaded}
  rename ::exit {}
  rename ::cdept::magic::tcl_exit ::exit
  flushlog
  # magic.tcl turns the background DRC off while it starts and on again only when
  # `drc status` said 1 before, which it does not (it prints and returns nothing).
  # Without it the areas a loaded file marks for checking (<< checkpaint >>) stay
  # drawn as "not checked yet" (black) for ever.
  if {!$failed} {catch {magic::drc on}}
  if {$failed} {
    if {[info exists ::env(CDEPT_MAGIC_DEBUG)]} {
      ::cdept::magic::tcl_puts stderr [dict get $options -errorinfo]
    }
    return -options $options $message
  }
  # After start-up, Magic's output goes on being reported.
  return {}
}

proc cdept::magic::loaded {args} {
  if {[llength [info commands ::magic::initialize]]
      && ![llength [trace info execution ::magic::initialize]]} {
    trace add execution ::magic::initialize leave ::cdept::magic::initialized
  }
}

# Magic was started with -nowindow (see engine.rs). Open the one layout window CDePT
# shows, as Magic's own GUI wrapper does: without Magic's caption and scroll bars
# (CDePT has its own). The caption must go: its height comes from a text measured
# before any window exists, which gives an arbitrary number. The thin border stays:
# without it a top-level window draws nothing (seen with 8.3.623).
proc cdept::magic::initialized {args} {
  if {[magic::display] eq "NULL"} return
  # See fix_caption() in shim/cdm.c: Magic's caption height is unset at this point.
  if {![cdept::fixcaption 18]} {
    puts stderr "cdept-magic: could not set Magic's caption height: small windows may be drawn cut off"
  }
  magic::windowcaption off
  magic::windowscrollbars off
  magic::openwindow
  ::cdept::setpoint
}

proc cdept::magic::version {} {
  if {[catch {magic::version} v]} {return unknown}
  return [lindex $v 0]
}

# ---- Questions CDePT asks --------------------------------------------------------

# The edit cell and whether it has unsaved changes: {cell modified}
proc cdept::magic::state {} {
  set cell {}
  catch {set cell [lindex [magic::cellname list window] 0]}
  set modified 0
  if {$cell ne {} && ![catch {cellname list modified} mods]} {
    set modified [expr {[llength $mods] > 0}]
  }
  list $cell $modified
}

# Cells with unsaved changes.
proc cdept::magic::modified {} {
  if {[catch {cellname list modified} mods]} {return {}}
  return $mods
}

# Write every modified cell that belongs to `dir` (the edited layout view's folder)
# or that has no file yet: cells of other views are not written from here unless
# `all` is set. Returns the cells written.
proc cdept::magic::save {{all 0}} {
  set written {}
  foreach c [modified] {
    if {[string match *UNNAMED* $c]} continue
    set f {}
    catch {set f [file dirname [magic::cellname list filepath $c]]}
    if {$all || $f eq {} || [string match *UNNAMED* $f] || [is_own $f]} {
      lappend written $c
    }
  }
  # `save <name>` would write the edit cell under that name: `writeall force <cells>`
  # writes the cells named, each to its own file.
  if {[llength $written]} {magic::writeall force {*}$written}
  return $written
}

variable cdept::magic::own_dir {}
proc cdept::magic::is_own {dir} {
  variable own_dir
  if {$own_dir eq {}} {return 1}
  return [expr {[file normalize $dir] eq [file normalize $own_dir]}]
}

# The selection: the instances selected, as {use cell} pairs (Magic's `what -list`:
# paint, labels, cells).
proc cdept::magic::selection {} {
  set out {}
  if {![catch {magic::what -list} w]} {
    foreach item [lindex $w 2] {
      lappend out [list [lindex $item 0] [lindex $item 1]]
    }
  }
  return $out
}

# Select these instances (cross-probing): the others are deselected. Names that are
# not instances of the edit cell are left out. Returns the ones selected.
proc cdept::magic::select_instances {names} {
  magic::select clear
  set done {}
  foreach n $names {
    if {[catch {magic::select more cell $n}] == 0} {lappend done $n}
  }
  return $done
}

# DRC of the edit cell as it is now (unsaved edits included): {style {{why {llx lly
# urx ury}} ...}} with the boxes in microns. At most `max` boxes.
proc cdept::magic::drc {{max 5000}} {
  set old [lindex [magic::units] 0]
  set box {}
  catch {set box [magic::box values]}
  # Check the whole edit cell again (areas painted while DRC was off are not queued).
  magic::units microns
  catch {magic::select top cell}
  magic::drc check
  magic::drc catchup
  catch {magic::select clear}
  set out {}
  set n 0
  foreach {why boxes} [magic::drc listall why] {
    foreach b $boxes {
      if {[incr n] > $max} break
      lappend out [list $why $b]
    }
  }
  set style {}
  catch {set style [magic::drc list style]}
  catch {magic::units internal; magic::box values {*}$box}
  catch {magic::units $old}
  return [list $style $out]
}

# Show the area {llx lly urx ury} (microns): the box on it, zoomed to it.
proc cdept::magic::show_area {area} {
  set old [lindex [magic::units] 0]
  magic::units microns
  magic::box values {*}$area
  magic::findbox zoom
  magic::zoom 2
  catch {magic::units $old}
}

# The layer names of the technology (the first name of each).
proc cdept::magic::layers {} {
  if {[catch {magic::tech layers *} l]} {return {}}
  return $l
}

# Reload the edit cell and what it places from disk (after CDePT changed the files).
proc cdept::magic::reload {file} {
  set cell [file rootname [file tail $file]]
  catch {magic::flush $cell}
  magic::load [file rootname $file]
  catch {magic::select clear}
  catch {magic::view}
}
