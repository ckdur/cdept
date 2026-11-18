# Whole flows through xschem with the drawing window on: xschem's C code opens the
# dialogs, they are answered with events, and the design changes.
# Needs the stand-in X of the C side: run with CDX_DISPLAY=900x600 tests/run.sh

if {![info exists ::cdept::drw(width)]} {
  puts "PASS flow tests skipped: no drawing window (set CDX_DISPLAY)"
} else {

set example [file normalize [file join $::faketk_dir .. .. .. examples buffer]]
append XSCHEM_LIBRARY_PATH :$example/libs
proc idx {w name} {lsearch -exact [$w get 0 end] $name}
proc popup_index {label} {
  set i 0
  foreach b [faketk::popup_buttons .ctxmenu] {
    if {[$b cget -text] eq $label} {return $i}
    incr i
  }
  return -1
}

# --- load ----------------------------------------------------------------------
clear_updates
xschem load $example/libs/logic/inv/schematic/inv.sch
update
check_eq load-title [lindex [updates title] end] {{"kind":"title","text":"xschem - logic/inv/schematic/inv.sch"}}
check load-mapped {[winfo ismapped .]}

# --- property editor, opened by xschem's C code -----------------------------------
xschem select instance M1
clear_updates
with_answers {
  {faketk::event text .dialog.symprop "name=M1 model=nmos w=3u l=0.35u del=0 m=1"}
  {press .dialog OK}
} {xschem edit_prop}
check_eq edit_prop-changes [xschem getprop instance M1 w] 3u
set w [join [updates window]]
check edit_prop-text {[string first {"path":".dialog.symprop","class":"Text","text":"name=M1 model=nmos w=1u l=0.35u del=0 m=1"} $w] >= 0}
check edit_prop-symbol-entry {[string first {"class":"Entry","text":"nmos4.sym"} $w] >= 0}
check edit_prop-modal {[string first {"path":".dialog","title":"Edit Properties","modal":true} $w] >= 0}
faketk::flush
check_eq edit_prop-title-modified [lindex [updates title] end] {{"kind":"title","text":"xschem - logic/inv/schematic/inv.sch*"}}
with_answers {
  {faketk::event text .dialog.symprop "name=M1 model=nmos w=9u l=0.35u del=0 m=1"}
  {press .dialog Cancel}
} {xschem edit_prop}
check_eq edit_prop-cancel [xschem getprop instance M1 w] 3u
# the same through the menu bar
with_answers {
  {faketk::event text .dialog.symprop "name=M1 model=nmos w=4u l=0.35u del=0 m=1"}
  {faketk::event key .dialog.symprop Escape}
  {press .dialog OK}
} {faketk::event menu .menubar.prop [.menubar.prop index Edit]}
check_eq edit_prop-from-menu [xschem getprop instance M1 w] 4u

# --- context menu: a right click in the drawing window -----------------------------
clear_updates
after 100 {
  set ::ctx_buttons [llength [faketk::popup_buttons .ctxmenu]]
  faketk::event popup_closed
}
cdept::event 4 450 300 0 3 0
cdept::event 5 450 300 0 3 1024
update
check ctxmenu-popup {[string first {"kind":"popup","items":[{"kind":"command","label":"Abort command","menu":".ctxmenu","index":0,"image":"CtxmenuAbort"}} [lindex [updates popup] end]] >= 0}
check ctxmenu-images-first {[string first {"name":"CtxmenuAbort","format":"} [join [updates images]]] >= 0}
check ctxmenu-many-entries {$::ctx_buttons > 5}
check_eq ctxmenu-dismissed [winfo exists .ctxmenu] 0
check ctxmenu-not-a-window {[string first .ctxmenu [join [updates window]]] < 0}
# choosing an entry: "Edit attributes" opens the property editor
xschem unselect_all
xschem select instance M1
after 100 {
  set i [popup_index {Edit attributes}]
  after 100 {press .dialog Cancel}
  faketk::event menu .ctxmenu $i
}
cdept::event 4 450 300 0 3 0
cdept::event 5 450 300 0 3 1024
update
check_eq ctxmenu-entry-ran [list [winfo exists .ctxmenu] [winfo exists .dialog]] {0 0}

# --- File > Open: xschem's own file dialog, and the question about unsaved changes ---
# (CDePT replaces this dialog with its library picker, see t_cdept.tcl; here the
# original is switched back on.)
set cdept::native_file_dialogs 0
set L .load.l.paneleft.list
set R .load.l.paneright.f.list
clear_updates
with_answers [list \
  {press .dialog No} \
  [list faketk::event select $L [list [lsearch -exact $::pathlist $example/libs]]] \
  {faketk::event select $R [idx $R logic]} \
  {faketk::event select $R [idx $R buf]} \
  {faketk::event select $R [idx $R schematic]} \
  {faketk::event select $R [idx $R buf.sch]} \
  {set ::entry_shows [.load.buttons_bot.entry get]} \
  {faketk::event activate $R [idx $R buf.sch]} \
] {faketk::event menu .menubar.file [.menubar.file index Open]}
check_eq open-entry-follows-selection $::entry_shows buf.sch
check_eq open-loaded [xschem get schname] $example/libs/logic/buf/schematic/buf.sch
set w [join [updates window]]
check open-asked-to-save {[string first {"title":"Ask Save","modal":true} $w] >= 0}
check open-dialog-lists {[regexp {"path":"\.load\.l\.paneright\.f\.list","class":"Listbox","items":\["\.","\.\.","buf","inv"\]} $w]}
check_eq open-title [lindex [updates title] end] {{"kind":"title","text":"xschem - logic/buf/schematic/buf.sch"}}
set cdept::native_file_dialogs 1

# --- a check entry of the menu changes what xschem draws with --------------------------
set before [xschem get draw_window]
faketk::event menu .menubar.view [.menubar.view index {Draw grid}]
check menu-check-reaches-c {$::draw_grid in {0 1}}
faketk::event menu .menubar.view [.menubar.view index {Draw grid}]

# --- insert text: dialog, then a click places it ----------------------------------------
with_answers {
  {faketk::event text .dialog.f2.txt "Hello CDePT"}
  {press .dialog OK}
} {xschem place_text}
cdept::event 6 300 200 0 0 0
cdept::event 4 300 200 0 1 0
cdept::event 5 300 200 0 1 256
update
check_eq place_text-added [xschem getprop text 0 txt_ptr] {Hello CDePT}

# --- quitting with changes asks, and Cancel keeps xschem running -------------------------
check_eq modified-before-exit [xschem get modified] 1
clear_updates
with_answers {{press [lindex [faketk_open_windows] end] Cancel}} {xschem exit closewindow}
check exit-asks {[string first {UNSAVED data: want to exit?} [join [updates window]]] >= 0}
check_eq exit-cancelled [xschem get schname] $example/libs/logic/buf/schematic/buf.sch
xschem set_modify 0

# --- xschem's own tabs: text buttons at the end of the tool bar -----------------------
faketk::flush
check tabs-hidden-when-single {[string first .tabs.x0 $::faketk::last(toolbar)] < 0}
clear_updates
# (xschem's own command: CDePT's `xschem new_schematic create` opens a document)
cdept::xschem_c new_schematic create
update
set toolbar [lindex [updates toolbar] end]
check tabs-two-buttons {[regexp {"path":"\.tabs\.x0","label":"[^"]*".*"path":"\.tabs\.x1"} $toolbar]}
check tabs-current-checked {[regexp {"path":"\.tabs\.x1","label":"[^"]*"(,"tooltip":"[^"]*")?,"checked":true} $toolbar]}
clear_updates
faketk::event invoke .tabs.x0
update
check_eq tabs-switch [xschem get current_win_path] .drw
check tabs-first-checked {[regexp {"path":"\.tabs\.x0","label":"[^"]*"(,"tooltip":"[^"]*")?,"checked":true} [lindex [updates toolbar] end]]}
xschem new_schematic switch .x1.drw
xschem set_modify 0
xschem exit closewindow
update
check_eq tabs-closed [winfo exists .tabs.x1] 0

check_eq no-windows-left [faketk_open_windows] {}
}
