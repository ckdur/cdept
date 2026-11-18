# cdept.tcl: CDePT's adjustments to xschem's Tcl side.
#
# The helper reads this file after xschem.tcl and the start-up file
# (cdept-xschem/src/start.rs). Parts 1 to 3 need the Tk stand-in (faketk.tcl) and
# change nothing with the minimal stub (tkmin.tcl); part 4 works with both.
#
# 1. Library picker. xschem's file browsers become one window whose root widget has
#    class `LibraryPicker`, which CDePT shows as its library / cell / view picker:
#      options  mode         open | save | symbol | merge
#               title        the title xschem gives the dialog
#               extensions   space separated, e.g. `sch sym tcl`
#               initialdir   folder xschem would start in
#               initialfile  file name proposed (save)
#      answer   faketk::event text <window path> <absolute file, or {} for cancel>
#    Covered: `load_file_dialog` (open, choose or insert a symbol, merge, save as; and
#    through it `save_file_dialog`, which only prepares its arguments) and
#    `file_chooser` (xschem's alternate browser). The originals stay available as
#    cdept::orig::load_file_dialog and cdept::orig::file_chooser, and are used when
#    `cdept::native_file_dialogs` is 0.
#    Destination files that are no design files (PNG, SVG, PDF exports) are asked for
#    with the ordinary file dialog (`FileDialog`, see faketk.tcl) instead.
#
# 2. One drawing area. A CDePT tab is one xschem process, so everything that would
#    open another schematic window or tab inside this process reports
#      {"kind":"open","path":"<absolute file>"}
#    instead, and CDePT opens that file as another document. The current design is
#    left as it is. See `cdept::open_document` and the list at "New windows and tabs".
#
# 3. For CDePT to call:
#      cdept::modified      1 when the design has unsaved changes, else 0
#      cdept::close         close the document: asks xschem's "save?" question when
#                           there are unsaved changes, then ends the process (exit
#                           status 0). Returns 0 if the user cancelled.
#      cdept::selection     what is selected, see there
#
# 4. Symbols made from schematics go into the cell's `symbol` view and name their
#    schematic, in libraries with view folders. See "Symbols made from schematics".
#      cdept::symbol_file <schematic>   where the symbol of that schematic is written
#
# Part of cdept-xschem, GPL-2.0-or-later.

namespace eval cdept {
  # 1: file dialogs are CDePT's library picker. 0: xschem's own browsers.
  variable native_file_dialogs
  if {![info exists native_file_dialogs]} {set native_file_dialogs 1}
  # the picker window that is open, if any
  variable picker {}
  namespace eval orig {}
}

# True when the stand-in that can show special windows is loaded.
proc cdept::have_faketk {} {
  expr {[llength [info commands ::faketk::special]] > 0}
}

# ---------------------------------------------------------------------------------
# Library picker
# ---------------------------------------------------------------------------------

# The extensions in one of xschem's file patterns: `*.\{sch,sym,tcl\}` gives
# `sch sym tcl`, `*.sym` gives `sym`, `*` gives nothing (any file).
proc cdept::extensions {pattern} {
  set pattern [string map [list \\ {}] $pattern]
  if {![regexp {^\*\.(.+)$} $pattern -> rest]} {return {}}
  set rest [string trim $rest "{}"]
  set out {}
  foreach ext [split $rest ,] {
    set ext [string trim $ext]
    if {$ext ne "" && $ext ne "*"} {lappend out $ext}
  }
  return $out
}

# Show the picker and wait for the answer: the chosen file, or {}.
proc cdept::pick {mode title extensions initialdir initialfile} {
  variable picker
  # one picker at a time, as xschem allows one file dialog: a new one replaces it
  if {$picker ne "" && [winfo exists $picker]} {::faketk::special_done $picker {}}
  set w .cdept_pick[incr ::faketk::serial]
  ::faketk::special $w LibraryPicker $title [dict create mode $mode title $title \
    extensions $extensions initialdir $initialdir initialfile $initialfile]
  wm protocol $w WM_DELETE_WINDOW [list ::faketk::special_done $w {}]
  set picker $w
  set result [::faketk::special_wait $w]
  if {$picker eq $w} {set picker {}}
  return $result
}

# What xschem's own dialog does with the name it got: the same questions (overwrite?
# not an xschem file?) and the same result. `dirvar` is the name of the global that
# remembers the folder (INITIALLOADDIR or INITIALINSTDIR).
proc cdept::accept {file loadfile confirm_overwrt ext dirvar} {
  global file_dialog_retval file_dialog_dir1 file_dialog_ext
  if {$file eq ""} {
    set file_dialog_retval {}
    return {}
  }
  set file_dialog_retval $file
  set file_dialog_dir1 [file dirname $file]
  if {$ext ne ""} {set file_dialog_ext $ext}
  set result [file_dialog_getresult $loadfile $confirm_overwrt]
  if {$result ne ""} {
    upvar #0 $dirvar dir
    set dir $file_dialog_dir1
  }
  return $result
}

# Insert a symbol, the way xschem's browser does it when it is left open next to the
# drawing (loadfile 2): ask, then start placing. Runs from the event loop, not from
# inside the key or menu handler that asked for it.
proc cdept::insert_symbol {title ext dirvar} {
  upvar #0 $dirvar dir
  set initialdir [expr {[info exists dir] ? $dir : ""}]
  set file [pick symbol $title [extensions $ext] $initialdir {}]
  set symbol [accept $file 2 0 $ext $dirvar]
  if {$symbol eq ""} return
  if {[xschem get ui_state] & 8192} {xschem abort_operation}
  xschem place_symbol $symbol
}

if {[cdept::have_faketk]} {

if {![llength [info commands ::cdept::orig::load_file_dialog]]} {
  rename ::load_file_dialog ::cdept::orig::load_file_dialog
  rename ::file_chooser ::cdept::orig::file_chooser
}

# Arguments as xschem's own (see xschem.tcl): loadfile is 1 to choose a file to read,
# 0 to choose a file to write, 2 for the symbol browser that does not wait.
proc load_file_dialog {{msg {}} {ext {}} {global_initdir {INITIALINSTDIR}}
     {loadfile {1}} {confirm_overwrt {1}} {initialf {}}} {
  if {!$::cdept::native_file_dialogs} {
    return [::cdept::orig::load_file_dialog $msg $ext $global_initdir $loadfile \
              $confirm_overwrt $initialf]
  }
  upvar #0 $global_initdir initdir
  if {![info exists initdir]} {set initdir {}}
  if {$loadfile == 2} {
    # xschem expects to get control back at once
    after idle [list ::cdept::insert_symbol $msg $ext $global_initdir]
    return {}
  }
  set extensions [::cdept::extensions $ext]
  xschem set semaphore [expr {[xschem get semaphore] + 1}]
  if {$loadfile == 0 && [llength $extensions]
      && [lsearch -regexp $extensions {^(sch|sym|tcl)$}] < 0} {
    # an export (png, svg, pdf...): any folder will do
    set file [tk_getSaveFile -title $msg -initialdir $initdir -initialfile $initialf]
  } else {
    if {$loadfile == 0} {
      set mode save
    } elseif {$global_initdir eq "INITIALINSTDIR"} {
      set mode symbol
    } elseif {[string match -nocase merge* $msg]} {
      set mode merge
    } else {
      set mode open
    }
    set file [::cdept::pick $mode $msg $extensions $initdir $initialf]
  }
  xschem set semaphore [expr {[xschem get semaphore] - 1}]
  return [::cdept::accept $file $loadfile $confirm_overwrt $ext $global_initdir]
}

# xschem's alternate browser (the `new_file_browser` setting) opens schematics and
# places symbols from one window. Here: one question, then what a double click does
# there: a schematic is opened, a symbol is placed.
proc file_chooser {} {
  if {!$::cdept::native_file_dialogs} {return [::cdept::orig::file_chooser]}
  after idle ::cdept::browse
  return {}
}

}

proc cdept::browse {} {
  global INITIALLOADDIR
  set initialdir [expr {[info exists INITIALLOADDIR] ? $INITIALLOADDIR : ""}]
  set file [pick open {File Browser} {sch sym tcl} $initialdir {}]
  if {$file eq "" || [xschem get semaphore] > 0} return
  set type [is_xschem_file $file]
  if {$type eq {0}} return
  if {[xschem get ui_state] & 8192} {xschem abort_operation}
  set INITIALLOADDIR [file dirname $file]
  if {$type eq "SCHEMATIC"} {
    if {$::open_in_new_window} {
      xschem load_new_window $file
    } else {
      xschem load -gui $file
    }
  } else {
    xschem place_symbol $file
  }
}

# ---------------------------------------------------------------------------------
# New windows and tabs
# ---------------------------------------------------------------------------------
#
# How each way of getting a second schematic in the same xschem is handled:
#   xschem new_schematic create ?arg? ?file?    report `open` (no file: refused)
#   xschem load_new_window ?file|-lastopened|-lastclosed?
#                                               report `open` (no file: ask first)
#   xschem schematic_in_new_window ...          report `open` for the schematic of
#                                               the selected instance, or the
#                                               current one
#   xschem symbol_in_new_window ...             the same for the symbol
#   xschem new_process ?file?                   report `open` (no file: refused)
#   open_sub_schematic (Alt+E, menu)            report `open`
#   keys handled in xschem's C code (Alt+I, Alt+Shift+I, Alt+Shift+E, Alt+O, Ctrl+T)
#                                               taken before xschem sees them
#                                               (cdept::key_filter, called by the C
#                                               side) and mapped to the commands above
#   the tabbed_interface setting                kept at 1: whatever still creates a
#                                               schematic inside this process makes
#                                               an xschem tab, which shares the one
#                                               drawing area, never a second window
# The menu entries, the tab bar's "+" button and the file dialog buttons all end in
# one of these commands.

# Tell CDePT to open `file` as a document of its own. Returns 1 if reported.
proc cdept::open_document {file} {
  if {$file eq ""} {return 0}
  if {![regexp {^https?://} $file]} {
    set file [file normalize [abs_sym_path $file]]
    if {![file isfile $file]} {
      cdept::say "$file does not exist."
      return 0
    }
  }
  cdept::ui [::faketk::jobj [list kind [::faketk::jstr open] path [::faketk::jstr $file]]]
  return 1
}

# A line for the user (CDePT's console).
proc cdept::say {text} {
  cdept::ui [::faketk::jobj [list kind [::faketk::jstr log] text [::faketk::jstr $text]]]
}

proc cdept::no_empty_window {} {
  cdept::say "A new schematic is created in the Library Manager, not as an empty window."
}

# The view of the same cell with the other extension: next to the file as xschem
# expects it, or in the neighbouring view folder of CDePT's `cells` layout
# (<cell>/schematic/<cell>.sch and <cell>/symbol/<cell>.sym).
proc cdept::counterpart {file ext view} {
  set beside [file rootname $file]$ext
  if {[file isfile $beside]} {return $beside}
  set other [file join [file dirname [file dirname $file]] $view [file tail $beside]]
  if {[file isfile $other]} {return $other}
  return $beside
}

# The one selected instance, or {} when the selection is anything else.
proc cdept::selected_instance {} {
  if {[xschem get lastsel] != 1} {return {}}
  return [lindex [xschem selected_set] 0]
}

proc cdept::schematic_in_new_window {} {
  if {[xschem get lastsel] == 0} {
    return [open_document [counterpart [xschem get schname] .sch schematic]]
  }
  set inst [selected_instance]
  if {$inst eq ""} {return 0}
  set file [xschem get_sch_from_sym $inst]
  if {$file eq "" || ![file isfile $file]} {return 0}
  return [open_document $file]
}

proc cdept::symbol_in_new_window {} {
  set inst [selected_instance]
  if {$inst eq ""} {
    return [open_document [counterpart [xschem get schname] .sym symbol]]
  }
  return [open_document [abs_sym_path [xschem getprop instance $inst cell::name]]]
}

proc cdept::load_new_window {files} {
  if {![llength $files]} {
    set file [load_file_dialog {Load file} {*.\{sch,sym,tcl\}} INITIALLOADDIR]
    if {$file ne ""} {open_document $file}
    return {}
  }
  foreach file $files {
    switch -- $file {
      -lastclosed {set file [get_lastclosed]}
      -lastopened {set file [get_lastopened]}
    }
    if {$file eq ""} {
      no_empty_window
    } else {
      open_document [cleanup_path $file]
    }
  }
  return {}
}

# `xschem new_schematic create ?noconfirm? ?file? ?draw?`
proc cdept::new_schematic_create {arglist} {
  set file [lindex $arglist 1]
  if {$file eq ""} {
    no_empty_window
  } else {
    open_document $file
  }
  # as xschem: the number of additional windows, which stays what it was
  return [xschem new_schematic ntabs]
}

# Keys that xschem's C code turns into a new window or tab without passing through
# Tcl. Called by the C side for every key press in the drawing window, before xschem
# gets it; returns 1 when the key was dealt with here.
proc cdept::key_filter {keysym state} {
  if {![have_faketk]} {return 0}
  # without Shift, Caps Lock, Num Lock and the mouse buttons, as xschem compares
  set mods [expr {$state & (4 | 8 | 64)}]
  set alt [expr {$mods == 8 || $mods == 64}]
  switch -- $keysym {
    105 - 73 {
      # i, I: symbol in a new window (I: in a new xschem)
      if {$alt} {symbol_in_new_window; return 1}
    }
    69 {
      # E: schematic in a new xschem. (Alt+e runs open_sub_schematic, see below.)
      if {$alt} {schematic_in_new_window; return 1}
    }
    111 {
      # o: load in a new tab or window
      if {$alt} {load_new_window {}; return 1}
    }
    116 {
      # Ctrl+t: a new, empty tab
      if {$mods & 4} {no_empty_window; return 1}
    }
  }
  return 0
}

if {[cdept::have_faketk]} {

# The `xschem` command, with the subcommands that create windows taken out.
if {![llength [info commands ::cdept::xschem_c]]} {
  rename ::xschem ::cdept::xschem_c
}
proc xschem {args} {
  switch -- [lindex $args 0] {
    new_schematic {
      if {[lindex $args 1] eq "create"} {
        return [::cdept::new_schematic_create [lrange $args 2 end]]
      }
    }
    load_new_window {return [::cdept::load_new_window [lrange $args 1 end]]}
    schematic_in_new_window {return [::cdept::schematic_in_new_window]}
    symbol_in_new_window {return [::cdept::symbol_in_new_window]}
    new_process {
      if {[llength $args] > 1} {
        ::cdept::open_document [lindex $args 1]
      } else {
        ::cdept::no_empty_window
      }
      return {}
    }
  }
  tailcall ::cdept::xschem_c {*}$args
}

if {![llength [info commands ::cdept::orig::open_sub_schematic]]} {
  rename ::open_sub_schematic ::cdept::orig::open_sub_schematic
}
# The schematic of an instance (the given one, or the selected one), or the current
# schematic when nothing is selected, in a window of its own. Returns 1 if opened.
proc open_sub_schematic {{inst {}} {inst_number 0}} {
  if {$inst eq "" && [xschem get lastsel] == 0} {
    return [::cdept::open_document [::cdept::counterpart [xschem get schname] .sch schematic]]
  }
  if {$inst eq ""} {
    set inst [::cdept::selected_instance]
    if {$inst eq ""} {return 0}
  } else {
    set names {}
    foreach {name symbol type} [xschem instance_list] {lappend names $name}
    if {$inst ni $names} {return 0}
  }
  set file [xschem get_sch_from_sym $inst]
  if {$file eq "" || ![file isfile $file]} {return 0}
  return [::cdept::open_document $file]
}

# Extra windows would each have a drawing area of their own: stay with tabs.
set tabbed_interface 1
trace add variable tabbed_interface write {apply {{args} {
  if {$::tabbed_interface != 1} {set ::tabbed_interface 1}
}}}
catch {.menubar.view entryconfigure {Tabbed interface} -state disabled}

}

# ---------------------------------------------------------------------------------
# For CDePT
# ---------------------------------------------------------------------------------

proc cdept::modified {} {
  expr {[xschem get modified] ? 1 : 0}
}

# Close the document. With unsaved changes xschem asks its own question first (the
# mirrored "Ask Save" dialog: Yes saves, No discards, Cancel keeps the document
# open). Ends the process with status 0, which CDePT takes as "closed"; returns 0
# when the document stays open.
proc cdept::close {} {
  if {[cdept::modified]} {
    switch -- [ask_save "Save changes to [file tail [xschem get schname]]?"] {
      yes {
        xschem save
        # still modified: the save was cancelled or failed
        if {[cdept::modified]} {return 0}
      }
      no {}
      default {return 0}
    }
  }
  exit 0
}

# What is selected, for CDePT's Inspector, as a list of keys and values:
#   count      number of selected objects of any kind
#   instances  one {name symbol attributes} per selected instance: the instance name,
#              the symbol as the schematic refers to it, the attribute string
#   wires      number of selected wires
#   texts      number of selected texts
proc cdept::selection {} {
  set instances {}
  foreach name [xschem selected_set] {
    lappend instances [list $name [xschem getprop instance $name cell::name] \
                         [xschem getprop instance $name]]
  }
  set texts 0
  foreach line [split [xschem selected_set text] \n] {
    if {[string trim $line] ne ""} {incr texts}
  }
  list count [xschem get lastsel] instances $instances \
       wires [llength [xschem selected_wire]] texts $texts
}

# ---------------------------------------------------------------------------------
# Symbols made from schematics
# ---------------------------------------------------------------------------------
#
# xschem writes the symbol it makes from a schematic next to that schematic: x.sch
# gives x.sym in the same folder. In a library with view folders (`cells` layout) the
# schematic is <library>/<cell>/<view>/<name>.sch and its symbol belongs in
#   <library>/<cell>/symbol/<name>.sym
# with a `schematic=` attribute naming the schematic (docs/XSCHEM.md section 7, and
# libcdept/src/xschem/symbol.rs, whose rules for the value are repeated here).
#
# xschem's C code leaves the writing to two Tcl procedures, which are replaced:
#   make_symbol      Symbol > Make symbol from schematic, key A, `xschem make_symbol`
#   make_symbol_lcc  the symbol of "make schematic and symbol from selected components"
# Both run an awk script of xschem that derives the name of the symbol from the name
# of the schematic. Here it runs on a copy of the schematic in a scratch folder, and
# the result is moved to its place. Schematics anywhere else (flat libraries, files
# outside the project) are handled by xschem's own procedures, as before.
#
# The start-up file lists the libraries with view folders in cdept_cells_libraries.

if {![info exists ::cdept_cells_libraries]} {set ::cdept_cells_libraries {}}
namespace eval cdept {
  variable scratch 0
}

# For a schematic in a view folder of one of the libraries: where its symbol belongs
# and the value of its `schematic=` attribute, as a list of two. Else {}.
proc cdept::symbol_place {schematic} {
  set schematic [file normalize $schematic]
  if {[file extension $schematic] ne ".sch"} {return {}}
  foreach library $::cdept_cells_libraries {
    set library [file normalize $library]
    set length [string length $library]
    if {[string range $schematic 0 $length] ne "$library/"} continue
    set parts [file split [string range $schematic [expr {$length + 1}] end]]
    if {[llength $parts] != 3} continue
    lassign $parts cell view name
    set stem [file rootname $name]
    set folder [file tail $library]
    if {$stem eq $cell} {
      # xschem puts the name of the symbol file in place of @symname: the value stays
      # right when the cell is renamed
      set value $folder/@symname/$view/@symname.sch
    } else {
      set value $folder/$cell/$view/$name
    }
    return [list [file join $library $cell symbol $stem.sym] $value]
  }
  return {}
}

# Where the symbol made from `schematic` is written.
proc cdept::symbol_file {schematic} {
  set schematic [abs_sym_path $schematic]
  set place [symbol_place $schematic]
  if {$place ne {}} {return [lindex $place 0]}
  return [abs_sym_path $schematic .sym]
}

# The first block `tag {...}` of a design file: the index of its first character
# after the brace and the index of the closing brace (the first brace without a
# backslash), or {} when there is none.
proc cdept::block {text tag} {
  if {![regexp -indices -line "^$tag \\\{" $text start]} {return {}}
  set from [expr {[lindex $start 1] + 1}]
  set end $from
  set length [string length $text]
  while {$end < $length} {
    set c [string index $text $end]
    if {$c eq "\\"} {incr end 2; continue}
    if {$c eq "\}"} {return [list $from $end]}
    incr end
  }
  return {}
}

# `text` (a symbol file) with the attribute schematic=`value` among the symbol's
# attributes: the K block, or the G block in old files where K is missing or empty.
proc cdept::set_schematic_attribute {text value} {
  if {[regexp {["\\]} $value]} {
    error "cannot write \"$value\" as an xschem attribute value"
  }
  if {$value eq "" || [regexp {\s} $value]} {set value \"$value\"}
  # inside the block, braces are written with a backslash
  set value [string map [list \{ \\\{ \} \\\}] $value]
  set k [block $text K]
  set g [block $text G]
  set blank {{text range} {
    expr {[string trim [string range $text [lindex $range 0] [lindex $range 1]-1]] eq ""}
  }}
  if {$k ne {} && $g ne {} && [apply $blank $text $k] && ![apply $blank $text $g]} {
    set range $g
  } elseif {$k ne {}} {
    set range $k
  } elseif {$g ne {}} {
    set range $g
  } else {
    error "the symbol has no attribute block (K {...})"
  }
  lassign $range from end
  set block [string range $text $from [expr {$end - 1}]]
  set item {(^|[ \t\n])schematic=("[^"]*"|[^ \t\n]*)}
  if {[regexp -indices $item $block -> before old]} {
    set first [expr {[lindex $before 1] + 1}]
    set block [string replace $block $first [lindex $old 1] schematic=$value]
  } else {
    set block [string trimright $block]
    if {$block ne ""} {append block \n}
    append block schematic=$value\n
  }
  return [string range $text 0 [expr {$from - 1}]]$block[string range $text $end end]
}

# Run one of xschem's symbol generators (`script`, with the arguments `arguments`
# before the file name) for `schematic` and put the result at `symbol`, with the
# attribute schematic=`value`.
proc cdept::generate_symbol {script arguments schematic symbol value} {
  global XSCHEM_SHAREDIR XSCHEM_TMP_DIR
  variable scratch
  set tmp [expr {[info exists XSCHEM_TMP_DIR] ? $XSCHEM_TMP_DIR : "/tmp"}]
  set dir [file join $tmp cdept-symbol-[pid]-[incr scratch]]
  file mkdir $dir
  set code [catch {
    set copy [file join $dir [file tail $schematic]]
    set made [file rootname $copy].sym
    file copy -force $schematic $copy
    # the generators look at the symbol that exists
    if {[file exists $symbol]} {file copy -force $symbol $made}
    exec awk -f [file join $XSCHEM_SHAREDIR $script] {*}$arguments $copy
    set fd [open $made r]
    fconfigure $fd -translation binary
    set text [read $fd]
    ::close $fd
    set text [set_schematic_attribute $text $value]
    file mkdir [file dirname $symbol]
    set fd [open $symbol w]
    fconfigure $fd -translation binary
    puts -nonewline $fd $text
    ::close $fd
  } message options]
  file delete -force $dir
  if {$code} {return -options $options $message}
  if {[have_faketk]} {
    cdept::ui [::faketk::jobj [list kind [::faketk::jstr written] path [::faketk::jstr $symbol]]]
    say "Symbol written to $symbol"
  }
  return {}
}

if {![llength [info commands ::cdept::orig::make_symbol]]} {
  rename ::make_symbol ::cdept::orig::make_symbol
  rename ::make_symbol_lcc ::cdept::orig::make_symbol_lcc
}

# Arguments as xschem's own: with `ask` at its default an existing symbol is only
# replaced after a question.
proc make_symbol {name {ask {no}}} {
  global symbol_width
  set name [abs_sym_path $name]
  set place [::cdept::symbol_place $name]
  if {$place eq {}} {return [::cdept::orig::make_symbol $name $ask]}
  lassign $place symbol value
  if {$ask eq {no} && [file exists $symbol]} {
    set answer [tk_messageBox -message "Warning: symbol $symbol already exists. Overwrite?" \
        -icon warning -parent [xschem get topwindow] -type okcancel]
    if {$answer ne {ok}} {return {}}
  }
  ::cdept::generate_symbol make_sym.awk [list $symbol_width] $name $symbol $value
  return {}
}

proc make_symbol_lcc {name} {
  set name [abs_sym_path $name]
  set place [::cdept::symbol_place $name]
  if {$place eq {}} {return [::cdept::orig::make_symbol_lcc $name]}
  lassign $place symbol value
  ::cdept::generate_symbol make_sym_lcc.awk {} $name $symbol $value
  return {}
}
