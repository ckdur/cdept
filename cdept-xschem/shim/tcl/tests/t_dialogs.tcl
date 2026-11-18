# xschem's dialogs, answered the way CDePT answers them: with events that arrive while
# xschem waits.

# --- ask_save ----------------------------------------------------------------
clear_updates
set r [with_answers {{press .dialog Yes}} {ask_save "Save it?"}]
check_eq ask_save-yes $r yes
set window [lindex [updates window] 0]
check ask_save-window {[string first {"path":".dialog","title":"Ask Save","modal":true} [lindex [updates window] end]] >= 0}
check ask_save-label {[string first {"path":".dialog.l1","class":"Label","text":"Save it?"} [join [updates window]]] >= 0}
check ask_save-buttons {[regexp {"path":"\.dialog\.f1\.b1","class":"Button","text":"Yes".*"text":"Cancel".*"text":"No"} [lindex [updates window] end]]}
check_eq ask_save-closed [lindex [updates window_closed] end] {{"kind":"window_closed","path":".dialog"}}
check_eq ask_save-no [with_answers {{press .dialog No}} {ask_save "Save it?"}] no
check_eq ask_save-cancel [with_answers {{press .dialog Cancel}} {ask_save "Save it?"}] {}
check_eq ask_save-escape [with_answers {{faketk::event key .dialog Escape}} {ask_save "Save it?"}] {}
check_eq ask_save-return [with_answers {{faketk::event key .dialog.f1.b1 Return}} {ask_save "Save it?"}] yes
check_eq ask_save-key-n [with_answers {{faketk::event key .dialog n}} {ask_save "Save it?"}] no
check_eq ask_save-no-cancel [with_answers {{press .dialog No}} {ask_save "Save it?" 0}] no

# --- alert_ --------------------------------------------------------------------
check_eq alert-ok [with_answers {{press .alert OK}} {alert_ "Something happened"}] 1
check_eq alert-close [with_answers {{faketk::event close .alert}} {alert_ "Something happened"}] 0
check_eq alert-yesno-no [with_answers {{press .alert No}} {alert_ "Sure?" {} 0 1}] 0
check_eq alert-yesno-yes [with_answers {{press .alert Yes}} {alert_ "Sure?" {} 0 1}] 1
check_eq alert-copy [with_answers {{press .alert Copy} {press .alert OK}} {alert_ "Copy me"}] 1
check_eq alert-copied [clipboard get] {Copy me}

# --- input_line ----------------------------------------------------------------
clear_updates
set r [with_answers {
  {faketk::event text .dialog.f1.e 42}
  {press .dialog OK}
} {input_line "Value:" {} 7}]
check_eq input_line-ok $r 42
check input_line-preset {[string first {"path":".dialog.f1.e","class":"Entry","text":"7"} [join [updates window]]] >= 0}
check input_line-focus {[regexp {"path":"\.dialog\.f1\.e","class":"Entry","text":"7","focus":true} [join [updates window]]]}
check_eq input_line-return [with_answers {
  {faketk::event text .dialog.f1.e 3.5}
  {faketk::event key .dialog.f1.e Return}
} {input_line "Value:"}] 3.5
check_eq input_line-cancel [with_answers {{press .dialog Cancel}} {input_line "Value:" {} 9}] {}
set ::got {}
with_answers {{faketk::event text .dialog.f1.e 11} {press .dialog OK}} {input_line "Value:" {set ::got}}
check_eq input_line-command $::got 11

# --- Tk's own dialogs --------------------------------------------------------------
clear_updates
set r [with_answers {{press [lindex [faketk_open_windows] end] Cancel}} {
  tk_messageBox -type okcancel -icon warning -title Careful -message "Really?" -detail "No way back"
}]
check_eq messagebox-cancel $r cancel
set w [lindex [updates window] end]
check messagebox-root {[regexp {"title":"Careful","modal":true,"root":\{"path":"\.faketk_msg[0-9]+","class":"MessageBox"} $w]}
check messagebox-options {[string first {"options":{"icon":"warning","type":"okcancel","message":"Really?","detail":"No way back","default":"ok"}} $w] >= 0}
check messagebox-buttons {[regexp {"class":"Button","text":"OK".*"class":"Button","text":"Cancel"} $w]}
check_eq messagebox-ok [with_answers {{press [lindex [faketk_open_windows] end] OK}} {tk_messageBox -message hi}] ok
check_eq messagebox-yes [with_answers {{press [lindex [faketk_open_windows] end] Yes}} {tk_messageBox -type yesno -message hi}] yes
check_eq messagebox-close [with_answers {{faketk::event close [lindex [faketk_open_windows] end]}} {tk_messageBox -type yesnocancel -message hi}] cancel
check_eq messagebox-return [with_answers {{faketk::event key [lindex [faketk_open_windows] end] Return}} {tk_messageBox -type yesno -default no -message hi}] no

clear_updates
set r [with_answers {{faketk::event text [lindex [faketk_open_windows] end] /tmp/chosen.sch}} {
  tk_getOpenFile -title "Pick one" -initialdir /tmp -initialfile a.sch -filetypes {{{Schematics} {.sch}} {{All} *}}
}]
check_eq openfile-result $r /tmp/chosen.sch
set w [lindex [updates window] end]
check openfile-root {[regexp {"title":"Pick one","modal":true,"root":\{"path":"\.faketk_file[0-9]+","class":"FileDialog"} $w]}
check openfile-options {[string first {"mode":"open","initialdir":"/tmp","initialfile":"a.sch","filetypes":"{{Schematics} {.sch}} {{All} *}"} $w] >= 0}
check_eq openfile-cancel [with_answers {{faketk::event text [lindex [faketk_open_windows] end] {}}} {tk_getOpenFile}] {}
check_eq openfile-close [with_answers {{faketk::event close [lindex [faketk_open_windows] end]}} {tk_getOpenFile}] {}
check_eq savefile [with_answers {{faketk::event text [lindex [faketk_open_windows] end] {/tmp/my file.sch}}} {tk_getSaveFile -initialfile x}] {/tmp/my file.sch}
check savefile-mode {[string first {"mode":"save"} [lindex [updates window] end]] >= 0}
check_eq directory [with_answers {{faketk::event text [lindex [faketk_open_windows] end] /tmp}} {tk_chooseDirectory}] /tmp
check_eq multiple [with_answers {{faketk::event text [lindex [faketk_open_windows] end] {/tmp/a b}}} {tk_getOpenFile -multiple 1}] {{/tmp/a b}}
set r [with_answers {
  {faketk::event text [lindex [faketk_open_windows] end].e #ff8800}
  {press [lindex [faketk_open_windows] end] OK}
} {tk_chooseColor -initialcolor #000000}]
check_eq choosecolor $r #ff8800
# a dialog of Tk's own that is destroyed under the waiting code
check_eq messagebox-destroyed [with_answers {{destroy [lindex [faketk_open_windows] end]}} {tk_messageBox -message hi}] {}

# --- text viewers and editors -----------------------------------------------------
clear_updates
# viewdata does not wait: the window stays until dismissed
viewdata "line one\nline two"
faketk::flush
set w $::viewdata_w
check viewdata-text {[string first {"class":"Text","text":"line one\nline two"} [join [updates window]]] >= 0}
check viewdata-not-modal {[string first "\"path\":\"$w\",\"title\":\"View data\",\"modal\":false" [lindex [updates window] end]] >= 0}
press $w {Select all}
press $w Copy
check_eq viewdata-copy [clipboard get] "line one\nline two"
faketk::event key $w Escape
check_eq viewdata-dismissed [winfo exists $w] 0

set r [with_answers {
  {faketk::event text .editdata.text "changed\ntext"}
  {press .editdata OK}
} {editdata "original" "Edit it"}]
check_eq editdata-ok $r "changed\ntext"
check_eq editdata-cancel [with_answers {{press .editdata Cancel}} {editdata "original"}] original

# enter_text: the dialog behind "insert text"
# (xschem's C code sets these before calling it)
set props {}
set tctx::retval {}
set tctx::hsize 0.4
set tctx::vsize 0.4
set r [with_answers {
  {faketk::event text .dialog.f2.txt "Hello\nworld"}
  {press .dialog OK}
} {enter_text "Enter text:"}]
check_eq enter_text-ok $r "Hello\nworld"
set r [with_answers {{press .dialog Cancel}} {enter_text "Enter text:"}]
check_eq enter_text-cancel $r {}

# text_line: schematic header / global properties
# (it returns ok or nothing, and leaves the text in tctx::retval)
set r [with_answers {
  {faketk::event text .dialog.textinput "* edited"}
  {press .dialog OK}
} {text_line "Global SPICE property:" 0 normal}]
check_eq text_line-ok [list $r $tctx::retval] {ok {* edited}}
# Shift-Return accepts. As in Tk, the key has already put its newline into the text
# when the binding runs, and xschem's binding takes it out again.
set r [with_answers {
  {faketk::event text .dialog.textinput "* shift return\n"}
  {faketk::event key .dialog.textinput Shift-Return}
} {text_line "Global SPICE property:" 0 normal}]
check_eq text_line-shift-return [list $r $tctx::retval] {ok {* shift return}}
set r [with_answers {{press .dialog Cancel}} {text_line "Global SPICE property:" 0 normal}]
check_eq text_line-cancel $r {}

# A dialog opened from a dialog: nested waits.
set r [with_answers {
  {press .dialog Yes}
} {
  set first [ask_save "outer"]
  set second [with_answers {{press .alert OK}} {alert_ inner}]
  list $first $second
}]
check_eq nested-sequence $r {yes 1}
after 20 {after 20 {press .alert OK}; set ::inner [alert_ "from a callback"]; press .dialog No}
check_eq nested-wait [ask_save outer] no
check_eq nested-wait-inner $::inner 1

check_eq no-windows-left [faketk_open_windows] {}

# What the user types is not sent back; what the code changes in response is.
toplevel .echo
entry .echo.e
label .echo.l -text {}
pack .echo.e .echo.l
bind .echo.e <KeyRelease> {if {[.echo.e get] eq "magic"} {.echo.l configure -text found}}
faketk::flush
clear_updates
faketk::event text .echo.e abc
check_eq echo-suppressed [llength [updates window]] 0
check_eq echo-applied [.echo.e get] abc
faketk::event text .echo.e magic
check echo-response-sent {[string first {"text":"found"} [lindex [updates window] end]] >= 0}
listbox .echo.lb
.echo.lb insert end one two three
pack .echo.lb
faketk::flush
clear_updates
faketk::event select .echo.lb 1
check_eq echo-select-suppressed [llength [updates window]] 0
check_eq echo-select-applied [.echo.lb curselection] 1
destroy .echo
faketk::flush
