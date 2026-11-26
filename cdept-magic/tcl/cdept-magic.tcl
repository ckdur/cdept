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
# shows, as Magic's own GUI wrapper does: without Magic's caption, scroll bars and
# border (CDePT draws its own), so that the window is all drawing area.
proc cdept::magic::initialized {args} {
  if {[magic::display] eq "NULL"} return
  magic::windowcaption off
  magic::windowscrollbars off
  magic::windowborder off
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
  catch {set cell [cellname list self]}
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

# The selection: cell instances {use cell} and the box {llx lly urx ury} in microns.
proc cdept::magic::selection {} {
  set uses {}
  catch {
    foreach u [what -list] {}
  }
  set ids {}
  catch {set ids [instance list selected]}
  foreach id $ids {
    set c {}
    catch {set c [instance list celldef $id]}
    lappend uses [list $id $c]
  }
  set box {}
  catch {set box [box values]}
  list $uses $box
}
