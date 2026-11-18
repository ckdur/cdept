# boot.tcl: bring xschem's user interface up on the Tk stand-in, for the tests.
#
# When the helper runs without drawing (cdx-tcl without --display), xschem.tcl skipped
# building its windows. This does what the end of xschem.tcl and xschem's C start-up
# do when there is a display, so that menus, tool bar, status bar and dialogs exist.
# With --display (the stand-in is loaded by the C side) nothing is left to do.

# The runner (run.sh) sets ::faketk_dir to the folder holding faketk.tcl.
if {![llength [info commands ::faketk::flush]]} {
  source [file join $::faketk_dir faketk.tcl]
}
if {![info exists has_x] || ![winfo exists .menubar]} {
  set has_x 1
  # variables xschem's C start-up sets when it has a display
  set tctx::max_new_windows 20
  foreach {name value} {Mod4Mask 64 Mod1Mask 8 ShiftMask 1 ControlMask 4 has_cairo 1 fullscreen 0} {
    set $name $value
  }
  # the part of xschem.tcl's main program that depends on has_x
  set tk_scaling [tk scaling]
  setup_toolbar
  catch {eval font create Underline-Font [font actual TkDefaultFont]}
  font configure Underline-Font -underline true -size 24
  . configure -cursor left_ptr
  set_old_tk_fonts
  set infowindow_text {}
  infowindow
  build_widgets {}
  set_replace_key_binding
  # what xinit.c does once its windows exist
  pack_widgets {}
  set_bindings .drw
  update
}
