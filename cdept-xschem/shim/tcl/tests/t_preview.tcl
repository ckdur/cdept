# Picture panes: dialogs where xschem's C code draws a schematic or symbol into a Tk
# frame (xschem's `preview_window`), and the desktop's clipboard reaching Tcl.
# The panes need the stand-in X of the C side: run with CDX_DISPLAY=900x600 tests/run.sh

# --- clipboard and primary selection from CDePT (no drawing window needed) ------------
clear_updates
faketk::event clipboard CLIPBOARD "from the desktop"
check_eq clipboard-get [clipboard get] "from the desktop"
check_eq clipboard-selection-get [selection get -selection CLIPBOARD] "from the desktop"
check_eq clipboard-tk-get [::tk::GetSelection . CLIPBOARD] "from the desktop"
check_eq clipboard-not-echoed [updates clipboard] {}
faketk::event clipboard PRIMARY "selected elsewhere"
check_eq primary-get [selection get] "selected elsewhere"
check_eq primary-keeps-clipboard [clipboard get] "from the desktop"
# xschem's own copy still goes out, and wins until the desktop says otherwise
clipboard clear
clipboard append "from xschem"
faketk::flush
check_eq clipboard-own [clipboard get] "from xschem"
check clipboard-own-reported {[string first {"text":"from xschem"} [lindex [updates clipboard] end]] >= 0}

if {![info exists ::cdept::drw(width)]} {
  puts "PASS picture pane tests skipped: no drawing window (set CDX_DISPLAY)"
} else {

set example [file normalize [file join $::faketk_dir .. .. .. examples buffer]]
append XSCHEM_LIBRARY_PATH :$example/libs
set buf_sym $example/libs/logic/buf/symbol/buf.sym
proc pane {path} {
  foreach p [cdept::previews] {
    if {[lindex $p 0] eq $path} {return [lrange $p 1 2]}
  }
  return {}
}

# --- the key cheat sheet: a top-level window that is a picture ----------------------
show_bindkeys
check bindkeys-pane {[pane .bk] ne {}}
check bindkeys-drawn {[cdept::colours .bk] > 3}
# CDePT shows it at another size: the window follows and xschem draws again
faketk::event configure .bk 420 300
check_eq bindkeys-resized [pane .bk] {420 300}
check_eq bindkeys-winfo [list [winfo width .bk] [winfo height .bk]] {420 300}
check bindkeys-redrawn {[cdept::colours .bk] > 3}
# the same size again changes nothing
cdept::damage
faketk::event configure .bk 420 300
check_eq bindkeys-same-size [pane .bk] {420 300}
# a click on it closes it, as in xschem
faketk::event invoke .bk
check_eq bindkeys-closed [winfo exists .bk] 0
check_eq bindkeys-pane-gone [pane .bk] {}

# --- xschem's own file browser (CDePT's library picker replaces it by default) -------
set cdept::native_file_dialogs 0
set ::seen {}
set ::colours 0
set r [with_answers {
  {set ::seen [pane .load.l.paneright.draw]}
  {faketk::event text .load.buttons_bot.entry $::buf_sym}
  {file_dialog_display_preview $::buf_sym}
  {set ::colours [cdept::colours .load.l.paneright.draw]}
  {faketk::event configure .load.l.paneright.draw 330 210}
  {set ::resized [pane .load.l.paneright.draw]}
  {set ::colours2 [cdept::colours .load.l.paneright.draw]}
  {press .load Cancel}
} {load_file_dialog {Choose symbol} {*.sym} INITIALINSTDIR}]
check load-pane-made {$::seen ne {}}
# a frame -height 3.8c, as Tk measures it
check_eq load-pane-height [lindex $::seen 1] 143
check load-pane-drawn {$::colours > 3}
check_eq load-pane-resized $::resized {330 210}
check load-pane-redrawn {$::colours2 > 3}
check_eq load-pane-gone [pane .load.l.paneright.draw] {}
set cdept::native_file_dialogs 1

}
