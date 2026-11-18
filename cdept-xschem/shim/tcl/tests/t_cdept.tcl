# CDePT's adjustments to xschem (cdept.tcl): the library picker in place of xschem's
# file browsers, new windows and tabs reported as documents to open, closing, and the
# selection. Needs the drawing window: run with CDX_DISPLAY=900x600 tests/run.sh

if {![info exists ::cdept::drw(width)]} {
  puts "PASS cdept.tcl tests skipped: no drawing window (set CDX_DISPLAY)"
} else {

# A copy of the example project to write into (the runner starts each test in a
# scratch folder).
set source [file normalize [file join $::faketk_dir .. .. .. examples buffer]]
file copy $source [pwd]/buffer
set example [file normalize [pwd]/buffer]
append XSCHEM_LIBRARY_PATH :$example/libs
set tb $example/libs/bench/tb_buf/schematic/tb_buf.sch
set buf $example/libs/logic/buf/schematic/buf.sch
set buf_sym $example/libs/logic/buf/symbol/buf.sym
set inv $example/libs/logic/inv/schematic/inv.sch
set inv_sym $example/libs/logic/inv/symbol/inv.sym

# The answer CDePT gives to the picker that is open. A picker that opens from the
# event loop may not be there yet: then try again shortly.
proc answer {file {tries 100}} {
  if {$::cdept::picker eq "" || ![winfo exists $::cdept::picker]} {
    if {$tries > 0} {after 10 [list answer $file [expr {$tries - 1}]]}
    return
  }
  faketk::event text $::cdept::picker $file
}
# The files reported with `open` since the log was cleared.
proc opened {} {
  set out {}
  foreach json [updates open] {
    if {[regexp {^\{"kind":"open","path":"(.*)"\}$} $json -> path]} {lappend out $path}
  }
  return $out
}
# Nothing else is shown in this xschem: no further window, no tab.
proc single {} {
  expr {[xschem new_schematic ntabs] == 0 && ![winfo exists .x1] && ![winfo exists .tabs.x1]
        && [faketk_open_windows] eq {}}
}
proc instances_of {symbol} {
  set out {}
  foreach {name sym type} [xschem instance_list] {
    if {$sym eq $symbol} {lappend out $name}
  }
  return $out
}

check_eq loaded-after-xschem.tcl [llength [info commands ::cdept::orig::load_file_dialog]] 1
xschem load $tb
update

# --- File > Open through the library picker ------------------------------------------
clear_updates
with_answers [list [list answer $buf]] {
  faketk::event menu .menubar.file [.menubar.file index Open]
}
check_eq open-loaded [xschem get schname] $buf
set w [join [updates window]]
check open-picker-window {[regexp {"title":"Load file","modal":true,"root":\{"path":"\.cdept_pick[0-9]+","class":"LibraryPicker"} $w]}
check open-picker-options {[regexp {"options":\{"mode":"open","title":"Load file","extensions":"sch sym tcl","initialdir":"} $w]}
check open-picker-closed {[llength [updates window_closed]] == 1}
check_eq open-remembers-folder $INITIALLOADDIR [file dirname $buf]
check_eq open-no-xschem-dialog [winfo exists .load] 0

# cancel: nothing changes
clear_updates
with_answers {{answer {}}} {xschem load}
check_eq open-cancel [xschem get schname] $buf
check_eq open-cancel-folder $INITIALLOADDIR [file dirname $buf]
# closing the picker window is a cancel too
with_answers {{faketk::event close $::cdept::picker}} {xschem load}
check_eq open-closed [xschem get schname] $buf
# the semaphore is back where it was
check_eq open-semaphore [xschem get semaphore] 0

# --- Save As through the picker --------------------------------------------------------
xschem load $inv
set copy $example/libs/logic/inv/schematic/inv_copy.sch
clear_updates
with_answers [list [list answer $copy]] {xschem saveas}
check save-written {[file isfile $copy]}
check_eq save-name [xschem get schname] $copy
set w [join [updates window]]
check save-picker-options {[regexp {"class":"LibraryPicker","options":\{"mode":"save","title":"Save file","extensions":"","initialdir":"[^"]*/inv/schematic","initialfile":"inv.sch"\}} $w]}
# an existing file: xschem asks before overwriting; No keeps the name
with_answers [list [list answer $inv] {press .alert No}] {xschem saveas}
check_eq save-overwrite-refused [xschem get schname] $copy
check save-overwrite-asked {[string first {Overwrite } [join [updates window]]] >= 0}
# an export is asked for with the ordinary file dialog
clear_updates
set r [with_answers {{faketk::event text [lindex [faketk_open_windows] end] /tmp/cdept_test_plot.png}} {
  save_file_dialog {Select destination file} *.png INITIALLOADDIR $example/plot.png
}]
check_eq export-result $r /tmp/cdept_test_plot.png
check export-file-dialog {[regexp {"class":"FileDialog","options":\{"mode":"save"} [join [updates window]]]}

# --- insert a symbol through the picker, and place it ----------------------------------
xschem load $buf
update
check_eq insert-before [llength [instances_of logic/inv/symbol/inv.sym]] 2
# Shift+Insert: xschem's browser that does not wait
clear_updates
cdept::event 6 300 200 0 0 0
cdept::event 2 300 200 65379 0 1
check_eq insert-returns-at-once [faketk_open_windows] {}
with_answers [list [list answer $inv_sym]] {update}
check insert-picker-mode {[regexp {"options":\{"mode":"symbol","title":"Insert symbol","extensions":"sym tcl"} [join [updates window]]]}
check insert-placing {[xschem get ui_state] != 0}
cdept::event 6 320 220 0 0 0
cdept::event 4 320 220 0 1 0
cdept::event 5 320 220 0 1 256
cdept::event 2 320 220 65307 0 0
check_eq insert-placed [llength [instances_of logic/inv/symbol/inv.sym]] 3
# Insert: xschem's C code asks for the symbol and waits
with_answers [list [list answer $buf_sym]] {cdept::event 2 300 200 65379 0 0}
cdept::event 6 400 300 0 0 0
cdept::event 4 400 300 0 1 0
cdept::event 5 400 300 0 1 256
cdept::event 2 400 300 65307 0 0
check_eq insert-waiting-placed [llength [instances_of logic/buf/symbol/buf.sym]] 1
check_eq insert-remembers-folder $INITIALINSTDIR [file dirname $buf_sym]
# cancel: nothing is placed
set before [llength [xschem instance_list]]
with_answers {{answer {}}} {cdept::event 2 300 200 65379 0 0}
check_eq insert-cancel [llength [xschem instance_list]] $before
check_eq insert-cancel-idle [xschem get ui_state] 0
# the menu entry
with_answers {{answer {}}} {
  faketk::event menu .menubar.file [.menubar.file index {Component browser}]
  update
}
check_eq insert-menu-cancel [faketk_open_windows] {}
xschem set_modify 0

# --- xschem's alternate browser (new_file_browser) -------------------------------------
xschem load $tb
set new_file_browser 1
with_answers [list [list answer $buf]] {file_chooser; update}
check_eq chooser-opens-schematic [xschem get schname] $buf
with_answers [list [list answer $inv_sym]] {cdept::event 2 300 200 65379 0 1; update}
check chooser-places-symbol {[xschem get ui_state] != 0}
cdept::event 2 300 200 65307 0 0
set new_file_browser 0
xschem set_modify 0

# --- the original dialogs are still there ----------------------------------------------
set cdept::native_file_dialogs 0
with_answers {{.load.buttons_bot.cancel invoke}} {
  set r [load_file_dialog {Load file} *.\{sch,sym,tcl\} INITIALLOADDIR]
}
check_eq original-dialog $r {}
set cdept::native_file_dialogs 1

# --- new windows and tabs become documents to open ----------------------------------------
xschem load $tb
update
proc reports {name script expected} {
  clear_updates
  uplevel #0 $script
  update
  check_eq $name-reports [opened] $expected
  check_eq $name-design-kept [xschem get schname] $::tb
  check $name-single {[single]}
}
reports new_schematic-file [list xschem new_schematic create {} $buf] [list $buf]
reports new_schematic-relative {xschem new_schematic create {} logic/inv/symbol/inv.sym} [list $inv_sym]
reports new_schematic-empty {xschem new_schematic create} {}
check new_schematic-empty-explained {[string first {Library Manager} [join [updates log]]] >= 0}
reports new_schematic-missing {xschem new_schematic create {} /nonexistent/x.sch} {}
reports load_new_window-file [list xschem load_new_window $buf] [list $buf]
reports load_new_window-two [list xschem load_new_window $buf $inv] [list $buf $inv]
reports load_new_window-ask [list with_answers [list [list answer $inv]] {xschem load_new_window}] [list $inv]
reports load_new_window-cancel {with_answers {{answer {}}} {xschem load_new_window}} {}
reports menu-open-in-new-window [list with_answers [list [list answer $buf]] {
  faketk::event menu .menubar.file [.menubar.file index {Open in new window}]
}] [list $buf]
reports menu-create-new-window {
  faketk::event menu .menubar.file [.menubar.file index {Create new window/tab}]
} {}
reports new_process-file [list xschem new_process $buf] [list $buf]
reports new_process-empty {xschem new_process} {}
# nothing selected: the current schematic, and its symbol (none here)
reports schematic_in_new_window-current {xschem schematic_in_new_window} [list $tb]
reports symbol_in_new_window-none {xschem symbol_in_new_window} {}
# the buffer instance selected
xschem select instance x1
reports schematic_in_new_window-instance {xschem schematic_in_new_window} [list $buf]
reports schematic_in_new_window-force {xschem schematic_in_new_window force} [list $buf]
reports symbol_in_new_window-instance {xschem symbol_in_new_window} [list $buf_sym]
reports open_sub_schematic {open_sub_schematic} [list $buf]
reports open_sub_schematic-named {open_sub_schematic x1} [list $buf]
reports menu-sub-schematic {
  set m .menubar.file
  for {set i 0} {$i <= [$m index end]} {incr i} {
    if {[$m type $i] eq "command" && [$m entrycget $i -command] eq "open_sub_schematic"} break
  }
  faketk::event menu $m $i
} [list $buf]
# keys that xschem's C code handles itself
xschem unselect_all
xschem select instance x1
reports key-alt-e {cdept::event 2 300 200 101 0 8} [list $buf]
xschem select instance x1
reports key-alt-shift-e {cdept::event 2 300 200 69 0 9} [list $buf]
xschem select instance x1
reports key-alt-i {cdept::event 2 300 200 105 0 8} [list $buf_sym]
xschem select instance x1
reports key-alt-shift-i {cdept::event 2 300 200 73 0 9} [list $buf_sym]
xschem select instance x1
reports key-super-i {cdept::event 2 300 200 105 0 64} [list $buf_sym]
reports key-alt-o [list with_answers [list [list answer $inv]] {cdept::event 2 300 200 111 0 8}] [list $inv]
reports key-ctrl-t {cdept::event 2 300 200 116 0 4} {}
# a primitive has no schematic to open
xschem unselect_all
xschem select instance C1
reports sub-schematic-of-primitive {open_sub_schematic} {}
xschem unselect_all
# keys that are not taken still reach xschem: `f` zooms, `i` descends into a symbol
check_eq key-not-taken [cdept::key_filter 102 0] 0
check_eq key-plain-i-not-taken [cdept::key_filter 105 0] 0
check_eq key-ctrl-o-not-taken [cdept::key_filter 111 4] 0
# the symbol of a schematic in the cells layout is in the neighbouring view folder
xschem load $buf
update
clear_updates
xschem symbol_in_new_window
check_eq counterpart-symbol [opened] [list $buf_sym]
xschem load $buf_sym
update
clear_updates
open_sub_schematic
check_eq counterpart-schematic [opened] [list $buf]
xschem load $tb
update
# extra windows are never used
set tabbed_interface 0
check_eq tabbed-interface-stays $tabbed_interface 1
check_eq tabbed-entry-disabled [.menubar.view entrycget {Tabbed interface} -state] disabled

# --- symbols made from schematics go into the symbol view ---------------------------------
lappend ::cdept_cells_libraries $example/libs/logic $example/libs/bench
check_eq symbol-place [cdept::symbol_place $buf] \
  [list $buf_sym logic/@symname/schematic/@symname.sch]
check_eq symbol-place-other-name [cdept::symbol_place $example/libs/logic/buf/schematic/b2.sch] \
  [list $example/libs/logic/buf/symbol/b2.sym logic/buf/schematic/b2.sch]
check_eq symbol-place-cell-folder [cdept::symbol_place $example/libs/logic/buf/buf.sch] {}
check_eq symbol-place-outside [cdept::symbol_place [pwd]/flat/x.sch] {}
check_eq symbol-place-not-a-schematic [cdept::symbol_place $buf_sym] {}
check_eq symbol-file [cdept::symbol_file $buf] $buf_sym
check_eq symbol-file-outside [cdept::symbol_file [pwd]/flat/x.sch] [pwd]/flat/x.sym
# the attribute
set k "v {x}\nK {type=subcircuit\ntemplate=\"name=x1\"\n}\nT {a} 0 0 0 0 1 1 {}\n"
check_eq attribute-added [cdept::set_schematic_attribute $k l/@symname/s/@symname.sch] \
  "v {x}\nK {type=subcircuit\ntemplate=\"name=x1\"\nschematic=l/@symname/s/@symname.sch\n}\nT {a} 0 0 0 0 1 1 {}\n"
check_eq attribute-replaced \
  [cdept::set_schematic_attribute "K {type=subcircuit schematic=old.sch\nformat=x}\n" new.sch] \
  "K {type=subcircuit schematic=new.sch\nformat=x}\n"
check_eq attribute-quoted [cdept::set_schematic_attribute "K {}\n" {my lib/a.sch}] \
  "K {schematic=\"my lib/a.sch\"\n}\n"
check_eq attribute-old-files [cdept::set_schematic_attribute "G {type=x}\nK {}\n" a.sch] \
  "G {type=x\nschematic=a.sch\n}\nK {}\n"
check attribute-no-block {[catch {cdept::set_schematic_attribute "v {x}\n" a.sch}]}

# a new cell with only a schematic: Symbol > Make symbol from schematic
proc slurp {file} {set fd [open $file]; set text [read $fd]; close $fd; return $text}
set inv3 $example/libs/logic/inv3/schematic/inv3.sch
set inv3_sym $example/libs/logic/inv3/symbol/inv3.sym
file mkdir [file dirname $inv3]
file copy $inv $inv3
xschem load $inv3
update
clear_updates
set make [.menubar.sym index {Make symbol from schematic}]
with_answers {{press [lindex [faketk_open_windows] end] OK}} {
  faketk::event menu .menubar.sym $make
}
check symbol-in-view {[file isfile $inv3_sym]}
check symbol-not-beside-schematic {![file exists [file rootname $inv3].sym]}
set text [slurp $inv3_sym]
check symbol-names-schematic \
  {[string first "\nschematic=logic/@symname/schematic/@symname.sch\n\}" $text] > 0}
check symbol-pins {[regexp {name=A dir=in} $text] && [regexp {name=Y dir=out} $text]}
check_eq symbol-reported [updates written] \
  [list "{\"kind\":\"written\",\"path\":\"$inv3_sym\"}"]
check_eq symbol-no-scratch-left [glob -nocomplain [file join $XSCHEM_TMP_DIR cdept-symbol-[pid]-*]] {}
# an existing symbol: xschem asks a second time; Cancel keeps it
set fd [open $inv3_sym a]; puts $fd "T {mine} 0 0 0 0 0.2 0.2 {}"; close $fd
with_answers {{press [lindex [faketk_open_windows] end] OK}
              {press [lindex [faketk_open_windows] end] Cancel}} {
  faketk::event menu .menubar.sym $make
}
check symbol-kept {[string first {T {mine}} [slurp $inv3_sym]] > 0}
with_answers {{press [lindex [faketk_open_windows] end] OK}
              {press [lindex [faketk_open_windows] end] OK}} {
  faketk::event menu .menubar.sym $make
}
check symbol-replaced {[string first {T {mine}} [slurp $inv3_sym]] < 0}
check symbol-replaced-names-schematic {[string first schematic=logic/@symname/ [slurp $inv3_sym]] > 0}
# the symbol of "make schematic and symbol from selected components"
set lcc $example/libs/logic/lcc/schematic/lcc.sch
file mkdir [file dirname $lcc]
file copy $inv $lcc
make_symbol_lcc $lcc
check lcc-symbol-in-view {[file isfile $example/libs/logic/lcc/symbol/lcc.sym]}
check lcc-symbol-not-beside-schematic {![file exists [file rootname $lcc].sym]}
check lcc-symbol-names-schematic \
  {[string first schematic=logic/@symname/schematic/@symname.sch \
     [slurp $example/libs/logic/lcc/symbol/lcc.sym]] > 0}
# anywhere else xschem's own procedure writes next to the schematic, as before
file mkdir [pwd]/flat
file copy $inv [pwd]/flat/inv4.sch
make_symbol [pwd]/flat/inv4.sch yes
check flat-symbol-beside-schematic {[file isfile [pwd]/flat/inv4.sym]}
check flat-symbol-no-attribute {[string first schematic= [slurp [pwd]/flat/inv4.sym]] < 0}
xschem load $tb
update

# --- colour names --------------------------------------------------------------------------
check_eq colour-name [cdept::colour {Deep Pink}] #ff1493
check_eq colour-name-case [cdept::colour lightSeaGreen] #20b2aa
check_eq colour-grey [cdept::colour grey50] #7f7f7f
check_eq colour-hex [cdept::colour #f00] #ff0000
check colour-unknown {[catch {cdept::colour nosuchcolour}]}
check_eq colour-winfo-rgb [winfo rgb . {Deep Pink}] {65535 5140 37779}

# --- the selection, for the Inspector --------------------------------------------------
check_eq selection-empty [cdept::selection] {count 0 instances {} wires 0 texts 0}
xschem select instance x1
check_eq selection-instance [cdept::selection] \
  {count 1 instances {{x1 logic/buf/symbol/buf.sym name=x1}} wires 0 texts 0}
xschem select instance C1
set s [cdept::selection]
check_eq selection-two [dict get $s count] 2
check_eq selection-two-names [lsort [lmap i [dict get $s instances] {lindex $i 0}]] {C1 x1}
check selection-attributes {[string first value=20f [dict get $s instances]] >= 0}
xschem select_all
set s [cdept::selection]
check selection-wires {[dict get $s wires] == 2}
check selection-count {[dict get $s count] > 10}
xschem unselect_all

# --- closing ---------------------------------------------------------------------------
# `exit` is watched instead of obeyed, except for the last case.
rename exit real_exit
proc exit {{code 0}} {set ::exited $code}
xschem load $copy
update
check_eq modified-clean [cdept::modified] 0
xschem select instance M1
xschem setprop instance M1 w 7u
xschem set_modify 1
check_eq modified-changed [cdept::modified] 1
# Cancel: the document stays
set ::exited none
clear_updates
set r [with_answers {{press .dialog Cancel}} {cdept::close}]
check_eq close-cancel-returns $r 0
check_eq close-cancel-stays $::exited none
check close-asks {[string first {"title":"Ask Save","modal":true} [join [updates window]]] >= 0}
check close-names-file {[string first {Save changes to inv_copy.sch?} [join [updates window]]] >= 0}
# Yes: saved, then closed
set ::exited none
with_answers {{press .dialog Yes}} {cdept::close}
check_eq close-yes-exits $::exited 0
check_eq close-yes-saved [cdept::modified] 0
set fd [open $copy]; set text [read $fd]; close $fd
check close-yes-on-disk {[string first w=7u $text] >= 0}
# not modified: no question
set ::exited none
clear_updates
cdept::close
check_eq close-clean-exits $::exited 0
check_eq close-clean-no-question [updates window] {}
# No: changes dropped, and the process really ends with status 0
xschem setprop instance M1 w 9u
xschem set_modify 1
rename exit {}
rename real_exit exit
check_eq windows-left [faketk_open_windows] {}
after 20 {press .dialog No}
cdept::close
puts "FAIL close-no: the process is still running"

}
