# Simulation and waveform graphs: the jobs xschem starts (`execute`, read through Tcl
# file events), the simulator configuration, the list of running jobs, raw files and
# the graph dialogs. No simulator is needed: the jobs are shell commands, and the raw
# file is written here. The graph part needs the drawing window (CDX_DISPLAY).

# Run the event loop until `condition` holds, for at most `ms` milliseconds. Returns
# whether it holds.
proc wait_for {condition {ms 5000}} {
  set end [expr {[clock milliseconds] + $ms}]
  while {![uplevel #0 [list expr $condition]] && [clock milliseconds] < $end} {
    after 20 {set ::wait_tick 1}
    vwait ::wait_tick
  }
  uplevel #0 [list expr $condition]
}
proc job_done {id} {expr {![info exists ::execute(pipe,$id)]}}
# The texts of the `log` updates sent since the log was cleared.
proc logs {} {
  set out {}
  foreach json [updates log] {
    if {[regexp {"text":"(.*)"\}$} $json -> text]} {append out $text \n}
  }
  return $out
}
# Shown windows whose title contains `pattern`.
proc windows_titled {pattern} {
  set out {}
  foreach w [faketk_open_windows] {
    if {[string match *$pattern* [wm title $w]]} {lappend out $w}
  }
  return $out
}
# Close the message boxes that open during the next `ms` milliseconds, keeping their
# messages in ::boxes. For questions asked from places a test cannot wait at.
proc close_boxes {{ms 1500}} {
  set ::boxes_until [expr {[clock milliseconds] + $ms}]
  after 20 close_boxes_tick
}
proc close_boxes_tick {} {
  foreach w [faketk_open_windows] {
    if {[winfo class $w] eq "Toplevel" && [info exists ::faketk::wmi($w,rootclass)]
        && $::faketk::wmi($w,rootclass) eq "MessageBox"} {
      set o $::faketk::wmi($w,rootopts)
      lappend ::boxes [list [wm title $w] [dict get $o message] \
        [expr {[dict exists $o detail] ? [dict get $o detail] : ""}]]
      faketk::event close $w
    }
  }
  if {[clock milliseconds] < $::boxes_until} {after 20 close_boxes_tick}
}

set ::boxes {}
cd [pwd]
set_sim_defaults

# --- a job that ends well ---------------------------------------------------------------
clear_updates
set id [execute 0 sh -c {echo out; echo err >&2}]
check job-started {$id >= 0}
check job-ended [wait_for "\[job_done $id\]"]
check_eq job-exit-status $execute(exitcode,$id) 0
check_eq job-data [string trim $execute(data,last)] out
check_eq job-errors [string trim $execute(error,last)] err
check job-logged {[string first "Job $id ended: sh -c" [logs]] >= 0}
update
check_eq job-no-window [faketk_open_windows] {}

# --- a job that fails without being asked to report: shown, not silent -------------------
clear_updates
set ::boxes {}
close_boxes
set id [execute 0 sh -c {echo oops >&2; exit 3}]
check failed-job-ended [wait_for "\[job_done $id\]"]
check_eq failed-job-exit-status $execute(exitcode,$id) 3
check failed-job-logged {[string first "Job $id ended with exit status 3: sh -c" [logs]] >= 0}
check failed-job-errors-logged {[string first oops [logs]] >= 0}
wait_for {[llength $::boxes] > 0} 1000
check_eq failed-job-box-title [lindex $::boxes 0 0] {Job failed}
check failed-job-box-message {[string match "*exit status 3*sh -c*" [lindex $::boxes 0 1]]}
check_eq failed-job-box-detail [lindex $::boxes 0 2] oops

# without error output (a terminal closed with a status, a job stopped): logged only
clear_updates
set ::boxes {}
set id [execute 0 sh -c {exit 4}]
check silent-failure-ended [wait_for "\[job_done $id\]"]
close_boxes 300
wait_for 0 400
check_eq silent-failure-no-box $::boxes {}
check silent-failure-logged {[string first "Job $id ended with exit status 4: sh -c" [logs]] >= 0}
check silent-failure-no-tcl-message {[string first abnormally [logs]] < 0}

# --- a program that is not there: xschem's own message ---------------------------------
set ::boxes {}
close_boxes
check_eq missing-program [execute 0 /nonexistent/program] -1
check missing-program-box {[string match "*Can not execute*" [lindex $::boxes 0 1]]}

# --- a job asked to report (status 1): xschem's "Completed" window ----------------------
set id [execute 1 sh -c {echo reported}]
check reported-job-ended [wait_for "\[job_done $id\] && \[llength \[windows_titled {View data}\]\]"]
set w [lindex [windows_titled {View data}] 0]
set t [find_widget $w Text *]
check reported-job-window {$t ne "" && [string match "Completed: sh -c*reported*" [$t get 1.0 end]]}
press $w Dismiss
update
check_eq reported-job-closed [faketk_open_windows] {}

# --- the list of running jobs; stopping one is not a failure ---------------------------
set ::boxes {}
set id [execute 0 sleep 30]
list_running_cmds
set lb .processlist.f2.lb
check running-listed {[lsearch -glob [$lb get 0 end] "sleep 30"] >= 0}
faketk::event select $lb [list [lsearch -glob [$lb get 0 end] "sleep 30"]]
press .processlist {Terminate selected}
check stopped-job-ended [wait_for "\[job_done $id\]"]
close_boxes 300
wait_for 0 400
check_eq stopped-job-no-box $::boxes {}
check stopped-job-unlisted [wait_for "\[lsearch -glob \[$lb get 0 end\] {sleep 30}\] < 0" 2000]
press .processlist Dismiss
update
check_eq running-list-closed [winfo exists .processlist] 0

# --- Simulation > Configure simulators and tools ---------------------------------------
set row .sim.topf.f.scrl.center.spice.r.2
set before [list $sim(spice,default) $sim(spice,2,cmd)]
simconf
check simconf-open {[winfo ismapped .sim] && [winfo exists $row.cmd] && [winfo exists $row.radio]}
check_eq simconf-command-shown [$row.cmd get 1.0 {end - 1 chars}] $sim(spice,2,cmd)
faketk::event text $row.cmd {ngspice -b "$N"}
faketk::event invoke $row.radio
check_eq simconf-radio $sim(spice,default) 2
press .sim.bottom Help
set help [lindex [windows_titled {View data}] 0]
check simconf-help {$help ne ""}
catch {press $help Dismiss}
# "Reset to default" asks first; "cancel" leaves everything
with_answers {{press [lindex [faketk_open_windows] end] Cancel}} {press .sim.bottom {Reset to default}}
check_eq simconf-reset-cancelled $sim(spice,default) 2
press .sim.bottom {Accept, no Save and Close}
check_eq simconf-closed [winfo exists .sim] 0
check_eq simconf-accepted [list $sim(spice,default) $sim(spice,2,cmd)] {2 {ngspice -b "$N"}}
check_eq simconf-semaphore [xschem get semaphore] 0
check_eq simconf-not-saved [file exists $USER_CONF_DIR/simrc] 0
simconf
press .sim.bottom {Accept, Save and Close}
check_eq simconf-saved [file exists $USER_CONF_DIR/simrc] 1
lassign $before sim(spice,default) sim(spice,2,cmd)

# --- with the drawing window: simulate, load the raw file, graphs ----------------------
if {![info exists ::cdept::drw(width)]} {
  puts "PASS graph tests skipped: no drawing window (set CDX_DISPLAY)"
} else {

set example [file normalize [file join $::faketk_dir .. .. .. examples buffer]]
append XSCHEM_LIBRARY_PATH :$example/libs
xschem load $example/libs/bench/tb_buf/schematic/tb_buf.sch
update
set netlist_dir [file normalize [pwd]/simulations]
file mkdir $netlist_dir

# A raw file as ngspice writes it in ascii: a pulse on `in`, its inverse on `out`.
set raw [file normalize [pwd]/source.raw]
set f [open $raw w]
puts $f "Title: test\nDate: today\nPlotname: Transient Analysis\nFlags: real"
puts $f "No. Variables: 3\nNo. Points: 101\nVariables:"
puts $f "\t0\ttime\ttime\n\t1\tv(in)\tvoltage\n\t2\tv(out)\tvoltage\nValues:"
for {set i 0} {$i <= 100} {incr i} {
  set v [expr {($i / 20) % 2 ? 1.8 : 0.0}]
  puts $f " $i\t[expr {$i * 1e-10}]\n\t$v\n\t[expr {1.8 - $v}]\n"
}
close $f

# The "simulator" copies the raw file to where a simulation would write it.
set sim(spice,0,cmd) "cp {$raw} \"\$n.raw\""
set sim(spice,0,fg) 0
set sim(spice,0,st) 0
set sim(spice,default) 0
clear_updates
xschem netlist
check netlist-written [file exists $netlist_dir/tb_buf.spice]
set id [simulate]
check simulate-started {$id >= 0}
check_eq simulate-button-busy [.menubar entrycget Simulate -background] orange
check simulate-ended [wait_for "\[job_done $id\]"]
check simulate-wrote-raw [file exists $netlist_dir/tb_buf.raw]
check simulate-button-done {[.menubar entrycget Simulate -background] ne "orange"}
check simulate-logged {[string first "Job $id ended: cp" [logs]] >= 0}

# Waves > Tran: xschem asks for the file (CDePT's file dialog), then reads it.
proc answer_file {file {tries 100}} {
  foreach w [faketk_open_windows] {
    if {[info exists ::faketk::wmi($w,rootclass)] && $::faketk::wmi($w,rootclass) eq "FileDialog"} {
      faketk::event text $w $file
      return
    }
  }
  if {$tries > 0} {after 10 [list answer_file $file [expr {$tries - 1}]]}
}
with_answers [list [list answer_file $netlist_dir/tb_buf.raw]] {
  faketk::event menu .menubar.waves [.menubar.waves index Tran]
}
check_eq waves-loaded [xschem raw_query loaded] 0
check waves-signals {[lsearch [xschem raw_query list] v(out)] >= 0}

# Simulation > Graphs > Add waveform graph: placed where the pointer is, like a move.
set n [xschem get rects 2]
faketk::event menu .menubar.simulation.graph [.menubar.simulation.graph index {Add waveform graph}]
cdept::event 6 450 450 0 0 0
cdept::event 4 450 450 0 1 0
cdept::event 5 450 450 0 1 256
update
check_eq graph-added [xschem get rects 2] [expr {$n + 1}]
check_eq graph-flags [xschem getprop rect 2 $n flags] graph
xschem unselect_all

# The graph dialog (a double click on a graph opens it).
graph_edit_properties $n
update
set d .graphdialog
check graph-dialog-open {[winfo ismapped $d]}
set list $d.center.left.list1
check graph-dialog-signals {[lsearch [$list get 0 end] out] >= 0 && [lsearch [$list get 0 end] in] >= 0}
faketk::event select $list [list [lsearch [$list get 0 end] in] [lsearch [$list get 0 end] out]]
press $d.center.left Add
check_eq graph-dialog-added [lsort [split [string trim [$d.center.right.text1 get 1.0 end]] \n]] {in out}
check_eq graph-dialog-live [lsort [split [xschem getprop rect 2 $n node] \n]] {in out}
faketk::event text $d.top3.xmax 1e-8
check_eq graph-dialog-xmax [xschem getprop rect 2 $n x2] 1e-8
faketk::event text $d.center.right.list tran
check_eq graph-dialog-sim-type [xschem getprop rect 2 $n sim_type] tran
faketk::event invoke $d.top4.r7
check_eq graph-dialog-colour $graph_sel_color 7
faketk::event invoke $d.top3.logy
check_eq graph-dialog-logy [xschem getprop rect 2 $n logy] 1
faketk::event invoke $d.top3.logy
# typing the list of signals
faketk::event text $d.center.right.text1 "out\n"
check_eq graph-dialog-typed [xschem getprop rect 2 $n node] out
faketk::event text $d.center.right.text1 "in\nout\n"
press $d.bottom OK
update
check_eq graph-dialog-closed [winfo exists $d] 0
check_eq graph-dialog-ok [split [xschem getprop rect 2 $n node] \n] {in out}
# the raw file named in the graph, read when it is drawn
graph_edit_properties $n
faketk::event invoke $d.center.right.autoload
faketk::event text $d.center.right.rawentry $netlist_dir/tb_buf.raw
check_eq graph-dialog-rawfile [xschem getprop rect 2 $n rawfile] $netlist_dir/tb_buf.raw
check_eq graph-dialog-autoload [xschem getprop rect 2 $n autoload] 1
press $d.bottom Cancel
check_eq graph-dialog-cancelled [winfo exists $d] 0

# The colour of one wave (a double click on its name in the graph); waits for OK.
with_answers {{faketk::event invoke .graphdialog.f.r5} {press .graphdialog OK}} {
  graph_edit_wave $n 1
}
check_eq graph-wave-colour [lindex [xschem getprop rect 2 $n color] 1] 5
check_eq graph-wave-dialog-closed [winfo exists .graphdialog] 0

# Cursors (keys a and b with the pointer in the graph), then a full zoom (f).
set before [xschem get cursor1_x]
foreach {key x} {97 420 98 480} {
  cdept::event 6 $x 450 0 0 0
  cdept::event 2 $x 450 $key 0 0
  cdept::event 3 $x 450 $key 0 0
}
update
check graph-cursors {([xschem getprop rect 2 $n flags] eq "graph") && [xschem get cursor1_x] != [xschem get cursor2_x]}
cdept::event 2 450 450 102 0 0
cdept::event 3 450 450 102 0 0
update
check graph-full-zoom {[xschem getprop rect 2 $n x2] > 9e-9}

# Send highlighted nets to the viewer (Ctrl+Shift+X). The default viewer, gaw, is not
# there: xschem says so and does not wait for it.
xschem select instance l7
xschem hilight
set ::boxes {}
close_boxes 3000
set t0 [clock milliseconds]
xschem create_plot_cmd
check send-to-viewer-returns {[clock milliseconds] - $t0 < 12000}
wait_for {[llength $::boxes] > 0} 1000
check send-to-viewer-said {[string match *gaw* [lindex $::boxes 0 1]]}
xschem unhilight_all

# Waves > Clear, then the graph draws without data (and does not read the file again).
xschem setprop -fast rect 2 $n autoload 0
xschem raw_clear
check_eq waves-cleared [xschem raw_query loaded] -1
xschem draw_graph $n
xschem set_modify 0
}

update
check_eq no-windows-left [faketk_open_windows] {}
