# lib.tcl: helpers for the tests of the Tk stand-in.

# Keep a copy of every update sent, for the tests to look at.
set ::ui_log {}
if {[llength [info commands ::cdept::ui]] && ![llength [info commands ::cdept::ui_real]]} {
  rename ::cdept::ui ::cdept::ui_real
  proc ::cdept::ui {json} {
    lappend ::ui_log $json
    ::cdept::ui_real $json
  }
}

# The updates of one kind sent since the log was last cleared, newest last.
proc updates {kind} {
  set out {}
  foreach json $::ui_log {
    if {[string match "\{\"kind\":\"$kind\"*" $json]} {lappend out $json}
  }
  return $out
}

proc clear_updates {} {
  faketk::flush
  set ::ui_log {}
}

proc check {name condition} {
  if {[uplevel 1 [list expr $condition]]} {
    puts "PASS $name"
  } else {
    puts "FAIL $name: $condition"
  }
}

proc check_eq {name got expected} {
  if {$got eq $expected} {
    puts "PASS $name"
  } else {
    puts "FAIL $name: got {$got}, expected {$expected}"
  }
}

# Run `script` (which opens a dialog and waits for it) while `answers` runs later, the
# way CDePT's events arrive while xschem waits. `answers` is a list of scripts, run
# 20 ms apart. Returns what `script` returns. A dialog nobody answers is closed after
# 3 s and reported.
proc with_answers {answers script} {
  set delay 20
  foreach a $answers {
    after $delay [list uplevel #0 $a]
    incr delay 20
  }
  set guard [after 3000 {
    puts "FAIL dialog not answered: [faketk_open_windows]"
    foreach w [faketk_open_windows] {catch {destroy $w}}
  }]
  set code [catch {uplevel 1 $script} result]
  after cancel $guard
  if {$code == 1} {
    puts "FAIL error in dialog script: $result"
    return {}
  }
  return $result
}

# Toplevels other than the main window that are shown now.
proc faketk_open_windows {} {
  set out {}
  foreach w $::faketk::toporder {
    if {[winfo exists $w] && [winfo ismapped $w]} {lappend out $w}
  }
  return $out
}

# The path of the first widget below `w` of class `class` whose text matches `pattern`.
proc find_widget {w class pattern} {
  foreach k [winfo children $w] {
    if {[winfo class $k] eq $class && [string match $pattern [::faketk::text_of $k]]} {
      return $k
    }
    set found [find_widget $k $class $pattern]
    if {$found ne ""} {return $found}
  }
  return {}
}

# Press the button labelled `pattern` in window `w`.
proc press {w pattern} {
  set b [find_widget $w Button $pattern]
  if {$b eq ""} {
    puts "FAIL no button '$pattern' in $w"
    return
  }
  faketk::event invoke $b
}
