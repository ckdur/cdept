# Sweep: call every xschem.tcl procedure that builds a window, and every command of
# the menu bar that is safe to run, close whatever opens, and report Tcl errors.
# This finds Tk features the stand-in lacks. Run with CDEPT_FAKETK_DEBUG=1 to also
# get "not implemented" lines on stderr.

set ::sweep_seen {}
# Close the windows that are open. While the swept command still runs, only the ones
# it waits for: closing a window that is still being built would only prove that
# xschem's code does not expect that.
proc sweep_close {{all 1}} {
  for {set round 0} {$round < 10} {incr round} {
    set open {}
    foreach w [faketk_open_windows] {
      if {$all || [faketk::is_modal $w]} {lappend open $w}
    }
    if {![llength $open]} break
    foreach w [lreverse $open] {
      if {![winfo exists $w]} continue
      lappend ::sweep_seen $w
      faketk::flush
      catch {faketk::event close $w}
      if {[winfo exists $w] && [winfo ismapped $w]} {catch {destroy $w}}
    }
  }
  if {[winfo exists .ctxmenu]} {catch {faketk::event popup_closed}}
}
proc sweep_tick {} {
  sweep_close 0
  set ::sweep_timer [after 60 sweep_tick]
}

proc sweep {name script} {
  set ::sweep_seen {}
  set ::sweep_timer [after 60 sweep_tick]
  set code [catch {uplevel #0 $script} err]
  # what the command left for the event loop (a dialog that opens from an idle
  # handler and waits) runs here, still watched by the timer
  catch {update}
  after cancel $::sweep_timer
  sweep_close
  if {$code == 1} {
    puts "FAIL sweep $name: [lindex [split $err \n] 0]"
  } else {
    puts "PASS sweep $name [lsort -unique $::sweep_seen]"
  }
}

if {[info exists ::cdept::drw(width)]} {
  set example [file normalize [file join $::faketk_dir .. .. .. examples buffer]]
  append XSCHEM_LIBRARY_PATH :$example/libs
  xschem load $example/libs/bench/tb_buf/schematic/tb_buf.sch
  update
}

# --- procedures that build windows ---------------------------------------------
sweep about about
sweep show_bindkeys show_bindkeys
sweep textwindow {textwindow $XSCHEM_SHAREDIR/keys.help ro}
sweep tclcmd tclcmd
sweep select_layers select_layers
sweep color_dim color_dim
sweep property_search property_search
sweep attach_labels_to_inst attach_labels_to_inst
sweep simconf simconf
sweep cellview cellview
sweep traversal traversal
sweep list_running_cmds list_running_cmds
sweep file_chooser file_chooser
sweep file_chooser_edit_paths file_chooser_edit_paths
sweep load_file_dialog-insert {load_file_dialog {Insert symbol} *.sym INITIALINSTDIR 2}
sweep load_file_dialog-load {load_file_dialog {Load file} *.\{sch,sym,tcl\} INITIALLOADDIR}
sweep load_file_dialog-save {save_file_dialog {Save file} *.\{sch,sym,tcl\} INITIALLOADDIR}
sweep context_menu context_menu
sweep infowindow {infowindow; wm deiconify .infotext}
sweep viewdata {viewdata "some\ndata"}
sweep editdata {editdata "some\ndata"}
sweep inutile_line {inutile_line "Line:"}
sweep ask_save ask_save
sweep alert_ {alert_ "An alert"}
sweep input_line {input_line "A number:"}
sweep text_line {text_line "Global property:" 0 normal}
sweep text_line-header {text_line "Header:" 0 header}
sweep enter_text {set props {}; set tctx::hsize 0.4; set tctx::vsize 0.4; set tctx::retval x; enter_text "Text:"}
sweep tk_messageBox {tk_messageBox -message hello}
sweep tk_getOpenFile {tk_getOpenFile}
sweep tk_chooseColor {tk_chooseColor}

# --- every command of the menu bar that neither quits, nor starts programs, nor
# --- writes files ----------------------------------------------------------------------
set ::deny {
  Quit|Close|Exit|process|Simulat|Netlist|Waves|Delete files|Save|Reload|Merge
  |Export|Print|editor|Edit file|terminal|Open in new|new window|window/tab|Open last
  |Open most|Clear|Push|Pop|browser|Component|Utile|utile|Shell|Configure simulators
  |Stop|Kill|Run|Launch|View Data|Annotate|backannot|Gaw|Bespice|Graph|graph|raw
  |Make|Create|Descend|Go back|LVS|Check|netlist|Tabbed|Fullscreen|Help|Keys|undo|Undo
  |Redo|Paste|paste|Insert|symbol from|pins|Attach|Join|Break|Trim|Align|Horizontal
  |Vertical|Flip|Rotate|Move|Duplicate|Copy|Cut|Delete|Select all|Wire|Line|Rect|Polygon
  |Arc|Circle|Text|Edit Header|Change order|Hide|Unhide|Lock|Edit vim|^Edit$
}
regsub -all {\s+} $::deny {} ::deny
proc sweep_menu {m path} {
  set n [expr {[$m index end] eq "none" ? -1 : [$m index end]}]
  for {set i 0} {$i <= $n} {incr i} {
    set type [$m type $i]
    set label [$m entrycget $i -label]
    switch -- $type {
      cascade {
        set sub [$m entrycget $i -menu]
        if {[winfo exists $sub]} {sweep_menu $sub "$path > $label"}
      }
      command - checkbutton - radiobutton {
        if {[regexp -- $::deny $label]} continue
        if {[$m entrycget $i -state] eq "disabled"} continue
        sweep "menu $path > $label" [list faketk::event menu $m $i]
        # check and radio entries are put back the way they were
        if {$type eq "checkbutton"} {catch {faketk::event menu $m $i}}
      }
    }
  }
}
# Only with the drawing window: the commands end in xschem's C code, which must have
# its display.
if {[info exists ::cdept::drw(width)]} {
  set saved_netlist_type $::netlist_type
  sweep_menu .menubar {}
  catch {
    if {$::netlist_type ne $saved_netlist_type} {
      set ::netlist_type $saved_netlist_type
      xschem set netlist_type $saved_netlist_type
    }
  }
  catch {xschem set_modify 0}
} else {
  puts "PASS sweep of the menu bar skipped: no drawing window (set CDX_DISPLAY)"
}
check_eq sweep-no-windows-left [faketk_open_windows] {}
