# The main window as xschem.tcl builds it: menu bar, tool bar, status bar, title.

faketk::flush
# sent while the windows were built, before this test could listen
set menu $::faketk::last(menu)
check menu-sent {$menu ne ""}
check menu-file-cascade {[string first {"kind":"cascade","label":"File","menu":".menubar","index":0,"items":} $menu] >= 0}
check menu-accelerator {[string first {"label":"Open","accelerator":"Ctrl+O","menu":".menubar.file"} $menu] >= 0}
check menu-separator {[string first {"kind":"separator"} $menu] >= 0}
check menu-top-level-command {[regexp {"kind":"command","label":"Netlist","menu":"\.menubar","index":[0-9]+} $menu]}

# A check entry: choosing it flips the variable and the menu is sent again.
set idx [.menubar.view index {Draw grid}]
check menu-index-by-label {[string is integer -strict $idx]}
check_eq menu-entry-type [.menubar.view type $idx] checkbutton
set before $::draw_grid
check menu-checked-before {[string first "\"label\":\"Draw grid\",\"accelerator\":\"%\",\"menu\":\".menubar.view\",\"index\":$idx,\"checked\":true" $menu] >= 0}
clear_updates
faketk::event menu .menubar.view $idx
check_eq menu-check-toggles $::draw_grid [expr {!$before}]
set menu [lindex [updates menu] end]
check menu-resent {$menu ne ""}
check menu-unchecked-after {[string first "\"label\":\"Draw grid\",\"accelerator\":\"%\",\"menu\":\".menubar.view\",\"index\":$idx\}" $menu] >= 0}
faketk::event menu .menubar.view $idx
check_eq menu-check-toggles-back $::draw_grid $before

# Setting the variable from elsewhere (a key binding in xschem) also re-sends the menu.
clear_updates
set ::draw_grid [expr {!$::draw_grid}]
faketk::flush
check menu-follows-variable {[llength [updates menu]] == 1}
set ::draw_grid $before

# A radio group.
set idx [.menubar.option.netlist index {Verilog netlist}]
faketk::event menu .menubar.option.netlist $idx
check_eq menu-radio-sets $::netlist_type verilog
faketk::event menu .menubar.option.netlist [.menubar.option.netlist index {Spice netlist}]
check_eq menu-radio-back $::netlist_type spice

# entryconfigure from xschem's C code.
clear_updates
.menubar entryconfigure Netlist -background red
.menubar.view entryconfigure {Tabbed interface} -state disabled
faketk::flush
set menu [lindex [updates menu] end]
check menu-disabled {[regexp {"label":"Tabbed interface"[^\}]*"enabled":false} $menu]}

# Tool bar.
faketk::flush
set toolbar [lindex [updates toolbar] end]
if {$toolbar eq ""} {set toolbar $::faketk::last(toolbar)}
check toolbar-button {[string first {{"path":".toolbar.bFileOpen","label":"FileOpen","tooltip":"Open File","image":"imgFileOpen"}} $toolbar] >= 0}
check toolbar-separator {[string first {{"separator":true}} $toolbar] >= 0}
check toolbar-right-side-last {[regexp {"\.toolbar\.bNetlist".*"\.toolbar\.bSimulate".*"\.toolbar\.bWaves"[^\]]*\]\}$} $toolbar]}
set images $::faketk::imgsent(imgFileOpen)
check image-is-gif {[string match R0lGOD* $images]}
set before $::dark_colorscheme
faketk::event invoke .toolbar.bViewToggleColors
check toolbar-invoke {$::dark_colorscheme != $before}
faketk::event invoke .toolbar.bViewToggleColors
clear_updates
toolbar_hide {}
faketk::flush
check toolbar-hidden {[string first {"items":[]} [lindex [updates toolbar] end]] >= 0}
toolbar_show {}
faketk::flush
check toolbar-shown-again {[string first bFileOpen [lindex [updates toolbar] end]] >= 0}

# Status bar, title, cursor: what xschem's C code does.
clear_updates
.statusbar.1 configure -text {mouse = 10 20 - selected: 0}
.statusbar.10 configure -state active -text {DRAW WIRE! }
.statusbar.3 delete 0 end
.statusbar.3 insert 0 10
faketk::flush
set status [lindex [updates status] end]
check status-text {[string first {{"name":".statusbar.1","text":"mouse = 10 20 - selected: 0"}} $status] >= 0}
check status-entry {[string first {{"name":".statusbar.3","text":"10","editable":true}} $status] >= 0}
check status-label-not-editable {[string first {{"name":".statusbar.2","text":"SNAP:"}} $status] >= 0}
check status-mode {[string first {DRAW WIRE! } $status] >= 0}
clear_updates
.statusbar.1 configure -text {mouse = 10 20 - selected: 0}
faketk::flush
check status-not-repeated {[llength [updates status]] == 0}
# An editable field: the new value, then Return, as CDePT sends them. xschem takes
# the value over (its own binding does that when the pointer leaves the entry).
clear_updates
set cadsnap 10
faketk::event text .statusbar.3 20
check_eq status-edit-not-yet-applied $cadsnap 10
faketk::event key .statusbar.3 Return
check_eq status-edit-applied $cadsnap 20
faketk::event text .statusbar.5 40
faketk::event key .statusbar.5 Return
check_eq status-edit-grid $cadgrid 40
check_eq status-edit-focus-back [focus] .drw
faketk::event text .statusbar.3 10
faketk::event key .statusbar.3 Return
faketk::event text .statusbar.5 20
faketk::event key .statusbar.5 Return
check_eq status-edit-back [list $cadsnap $cadgrid] {10 20}

# A menu entry xschem colours to show a state, and one with a picture.
.menubar entryconfigure Netlist -background $simulate_bg
clear_updates
.menubar entryconfigure Simulate -background Green
faketk::flush
set menu [lindex [updates menu] end]
check menu-colour {[regexp {"label":"Simulate","menu":"\.menubar","index":[0-9]+,"color":"Green"} $menu]}
check menu-colour-not-on-plain-entries {![regexp {"label":"Open"[^\}]*"color":} $menu]}
clear_updates
.menubar entryconfigure Simulate -background $simulate_bg
faketk::flush
check menu-colour-gone {[regexp {"label":"Simulate","menu":"\.menubar","index":[0-9]+\}} [lindex [updates menu] end]]}
clear_updates
.menubar.file entryconfigure Open -image imgFileOpen -compound left
faketk::flush
check menu-image {[regexp {"label":"Open","accelerator":"Ctrl\+O","menu":"\.menubar\.file","index":[0-9]+,"image":"imgFileOpen"} [lindex [updates menu] end]]}
.menubar.file entryconfigure Open -image {}
faketk::flush
check menu-image-gone {[string first {"image":} [lindex [updates menu] end]] < 0}
wm title . "xschem - test.sch*"
.drw configure -cursor tcross
faketk::flush
check_eq title-update [lindex [updates title] end] {{"kind":"title","text":"xschem - test.sch*"}}
check_eq cursor-update [lindex [updates cursor] end] {{"kind":"cursor","name":"tcross"}}
clear_updates
.drw configure -cursor {}
faketk::flush
check_eq cursor-default [lindex [updates cursor] end] {{"kind":"cursor","name":"left_ptr"}}

# Text with characters that need escaping survives as JSON.
clear_updates
.statusbar.1 configure -text "quote \" backslash \\ tab \t newline \n bell \a é €"
faketk::flush
check status-escaped {[string first {quote \" backslash \\ tab \t newline \n bell \u0007 é €} [lindex [updates status] end]] >= 0}

# The drawing window answers with its real size.
if {[info exists ::cdept::drw(width)]} {
  # with the drawing window on, the C side says how big it is
  check_eq drawing-size [list [winfo width .drw] [winfo height .drw]] \
    [list $::cdept::drw(width) $::cdept::drw(height)]
} else {
  faketk::drawing_size 1234 567
  check_eq drawing-size [list [winfo width .drw] [winfo height .drw]] {1234 567}
}
check_eq main-exists [winfo exists .drw] 1
check drw-bindings {[string first "xschem callback" [bind .drw <ButtonPress>]] >= 0}
check_eq clipboard-roundtrip [list [clipboard clear] [clipboard append abc] [clipboard get]] {{} {} abc}
faketk::flush
check_eq clipboard-update [lindex [updates clipboard] end] {{"kind":"clipboard","text":"abc"}}

# The info window: what xschem adds to it is reported as log text.
clear_updates
xschem set infowindow_text "first message\n"
infowindow
faketk::flush
check_eq log-first [lindex [updates log] end] {{"kind":"log","text":"first message\n"}}
clear_updates
xschem set infowindow_text "first message\nsecond message\n"
infowindow
faketk::flush
check_eq log-only-the-new-part [lindex [updates log] end] {{"kind":"log","text":"second message\n"}}
check log-window-stays-hidden {[string first .infotext [join [updates window]]] < 0}

# An error in a callback is reported, not raised.
toplevel .e
button .e.b -text Boom -command {error "it broke"}
pack .e.b
clear_updates
faketk::event invoke .e.b
check log-callback-error {[string first {Error: it broke} [lindex [updates log] end]] >= 0}
destroy .e
