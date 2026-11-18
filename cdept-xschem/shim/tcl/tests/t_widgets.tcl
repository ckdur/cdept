# The widget commands on their own: no xschem code involved.

# --- text widget --------------------------------------------------------------
toplevel .t
text .t.x
.t.x insert 1.0 "first line\nsecond line\nthird"
check_eq text-get-all [.t.x get 1.0 end] "first line\nsecond line\nthird\n"
check_eq text-get-end-1c [.t.x get 1.0 {end - 1 chars}] "first line\nsecond line\nthird"
check_eq text-index-end [.t.x index end] 4.0
check_eq text-index-end-1c [.t.x index "end - 1 chars"] 3.5
check_eq text-lineend [.t.x index "2.0 lineend"] 2.11
check_eq text-linestart [.t.x index "2.5 linestart"] 2.0
check_eq text-plus-lines [.t.x index "1.3 + 1 lines"] 2.3
check_eq text-plus-chars [.t.x get 1.0 "1.0 + 5 chars"] first
check_eq text-plusc [.t.x get "1.0+6c" "1.0 lineend"] line
check_eq text-wordend [.t.x index "1.0 wordend"] 1.5
.t.x insert end "\nfourth"
check_eq text-insert-end [.t.x get 4.0 4.end] fourth
.t.x delete 1.0 2.0
check_eq text-delete-line [.t.x get 1.0 1.end] "second line"
.t.x mark set insert 1.6
.t.x insert insert "!!"
check_eq text-insert-mark [.t.x get 1.0 1.end] "second!! line"
check_eq text-mark-moves [.t.x index insert] 1.8
.t.x tag add sel 1.0 1.6
check_eq text-tag-ranges [.t.x tag ranges sel] {1.0 1.6}
check_eq text-sel-first [.t.x get sel.first sel.last] second
.t.x delete sel.first sel.last
check_eq text-delete-sel [.t.x get 1.0 1.end] "!! line"
check_eq text-tag-gone [.t.x tag ranges sel] {}
check text-sel-error {[catch {.t.x index sel.first}]}
.t.x delete 1.0 end
.t.x insert 1.0 "name=R1 value=10k\nmodel=res"
set idx [.t.x search -regexp -count n {value=[^ \n]+} 1.0]
check_eq text-search-regexp $idx 1.8
check_eq text-search-count $n 9
check_eq text-search-exact [.t.x search model 1.0] 2.0
check_eq text-search-nocase [.t.x search -nocase MODEL 1.0] 2.0
check_eq text-search-none [.t.x search zzz 1.0] {}
check_eq text-search-nolinestop [.t.x search -regexp -nolinestop {10k.model} 1.0] 1.14
check_eq text-compare [.t.x compare 1.0 < 2.0] 1
check_eq text-count [.t.x count 1.0 1.4] 4
.t.x replace 1.0 1.4 NAME
check_eq text-replace [.t.x get 1.0 1.7] NAME=R1
.t.x edit modified 0
check_eq text-modified-reset [.t.x edit modified] 0
.t.x insert end x
check_eq text-modified-set [.t.x edit modified] 1
check_eq text-tag-names-at [lsort [.t.x tag names 1.0]] {}
.t.x tag add hot 1.0 1.4
check_eq text-tag-names-at2 [.t.x tag names 1.2] hot

# --- entry -------------------------------------------------------------------
entry .t.e
.t.e insert 0 hello
.t.e insert end " world"
check_eq entry-get [.t.e get] "hello world"
.t.e delete 0 6
check_eq entry-delete [.t.e get] world
.t.e selection range 0 end
check_eq entry-sel-present [.t.e selection present] 1
.t.e delete sel.first sel.last
check_eq entry-delete-sel [.t.e get] {}
set ::tv start
entry .t.e2 -textvariable tv
check_eq entry-textvariable [.t.e2 get] start
.t.e2 insert end !
check_eq entry-writes-variable $::tv start!
set ::tv other
check_eq entry-follows-variable [.t.e2 get] other
entry .t.e3 -textvariable not_yet_set
check_eq entry-creates-variable [info exists ::not_yet_set] 1
check_eq entry-index-end [.t.e2 index end] 5

# --- listbox ------------------------------------------------------------------
listbox .t.l
.t.l insert end a b c d
check_eq lb-size [.t.l size] 4
check_eq lb-get [.t.l get 1 2] {b c}
.t.l selection set 1
.t.l selection set 3
check_eq lb-cursel [.t.l curselection] {1 3}
.t.l delete 0
check_eq lb-sel-shifts [.t.l curselection] {0 2}
check_eq lb-includes [.t.l selection includes 2] 1
.t.l selection clear 0 end
check_eq lb-clear [.t.l curselection] {}
set ::lv {x y z}
listbox .t.l2 -listvariable lv
check_eq lb-listvariable [.t.l2 get 0 end] {x y z}
.t.l2 insert end w
check_eq lb-writes-listvariable $::lv {x y z w}
check_eq lb-index-end [.t.l2 index end] 4

# --- buttons -------------------------------------------------------------------
set ::cb 0
checkbutton .t.c -variable cb -command {incr ::cb_calls}
set ::cb_calls 0
.t.c invoke
check_eq check-invoke "$::cb $::cb_calls" {1 1}
.t.c toggle
check_eq check-toggle $::cb 0
checkbutton .t.c2 -variable mode -onvalue disk -offvalue memory
set ::mode memory
.t.c2 invoke
check_eq check-onvalue $::mode disk
set ::rb {}
radiobutton .t.r1 -variable rb -value one
radiobutton .t.r2 -variable rb -value two
.t.r2 invoke
check_eq radio-invoke $::rb two
button .t.b -text Go -command {set ::went 1} -state disabled
.t.b invoke
check button-disabled {![info exists ::went]}
.t.b configure -state normal
.t.b invoke
check_eq button-invoke $::went 1
check_eq cget-default [.t.b cget -state] normal
check_eq cget-alias [.t.b cget -bg] [.t.b cget -background]
check_eq configure-one [lindex [.t.b configure -text] 4] Go

# --- combobox, spinbox, scale ------------------------------------------------
ttk::combobox .t.cb -values {red green blue} -textvariable colour
.t.cb set green
check_eq combo-set $::colour green
check_eq combo-current [.t.cb current] 1
.t.cb current 2
check_eq combo-current-set [.t.cb get] blue
spinbox .t.s -values {f p n u} -textvariable unit
.t.s set n
check_eq spinbox-set $::unit n
scale .t.sc -from 0 -to 10 -variable level -command {set ::scaled}
.t.sc set 4
check_eq scale-set "$::level $::scaled" {4 4}

# --- bindings ----------------------------------------------------------------------
bind .t <Return> {set ::key return}
bind .t <Control-Key-a> {set ::key ctrl-a}
bind .t <a> {set ::key a}
bind .t.e <KeyPress> {set ::key any-%K-%A}
check_eq bind-normalised [bind .t <Key-Return>] {set ::key return}
check_eq bind-button-alias [faketk::normseq <1>] [faketk::normseq <Button-1>]
check_eq bind-modifier-order [faketk::normseq <Shift-Control-x>] [faketk::normseq <Control-Shift-Key-x>]
faketk::event key .t.b Return
check_eq key-on-toplevel $::key return
faketk::event key .t.b Control-a
check_eq key-with-modifier $::key ctrl-a
faketk::event key .t.b a
check_eq key-plain $::key a
faketk::event key .t.e a
check_eq key-widget-then-toplevel $::key a
bind .t.e <KeyPress> {set ::key any-%K-%A; break}
faketk::event key .t.e x
check_eq key-break-stops $::key any-x-x
bind .t.b <Button-1> {set ::click %b-%x}
faketk::dispatch .t.b <ButtonPress-1> {b 1 x 12}
check_eq dispatch-button $::click 1-12
bind .t.b <ButtonPress> {set ::click any-%b}
faketk::dispatch .t.b <ButtonPress-3> {b 3}
check_eq dispatch-falls-back $::click any-3
bind .t.b <Button-1> "+append ::click +more"
faketk::dispatch .t.b <ButtonPress-1> {b 1 x 5}
check_eq bind-append $::click 1-5+more
bind .t.b <Button-1> {}
check_eq bind-remove [bind .t.b <Button-1>] {}

# --- geometry -------------------------------------------------------------------
frame .t.f
label .t.f.a -text A; label .t.f.b -text B; label .t.f.c -text C
pack .t.f.a .t.f.b -side left
pack .t.f.c -side left -before .t.f.a
check_eq pack-order [pack slaves .t.f] {.t.f.c .t.f.a .t.f.b}
check pack-info {[dict get [pack info .t.f.a] -side] eq "left"}
pack forget .t.f.b
check_eq pack-forget [pack slaves .t.f] {.t.f.c .t.f.a}
check_eq winfo-manager [winfo manager .t.f.a] pack
check_eq winfo-ismapped-unpacked [winfo ismapped .t.f.a] 0
pack .t.f
check_eq winfo-ismapped-packed [winfo ismapped .t.f.a] 1
frame .t.g
label .t.g.a; label .t.g.b; label .t.g.c
grid .t.g.a - .t.g.b -sticky ew
grid .t.g.c -row 1 -column 2
check_eq grid-columnspan [dict get [grid info .t.g.a] -columnspan] 2
check_eq grid-next-column [dict get [grid info .t.g.b] -column] 2
check_eq grid-explicit [dict get [grid info .t.g.c] -row] 1

# --- winfo, wm, destroy -----------------------------------------------------
check_eq winfo-toplevel [winfo toplevel .t.f.a] .t
check_eq winfo-parent [winfo parent .t.f.a] .t.f
check_eq winfo-class [winfo class .t.x] Text
check winfo-children {".t.f" in [winfo children .t]}
wm title .t "A title"
check_eq wm-title [wm title .t] "A title"
wm protocol .t WM_DELETE_WINDOW {set ::closed 1}
faketk::event close .t
check_eq close-runs-protocol $::closed 1
check_eq close-keeps-window [winfo exists .t] 1
check dup-create-fails {[catch {frame .t.f}]}
check orphan-create-fails {[catch {frame .nope.f}]}
after 10 {destroy .t}
tkwait window .t
check_eq destroy-recursive [winfo exists .t.f.a] 0
check destroy-removes-command {[catch {.t.x get 1.0 end}]}
check_eq image-size [list [image width imgFileOpen] [image height imgFileOpen]] {24 24}
