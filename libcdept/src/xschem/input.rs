//! Pointer and keyboard input for xschem's drawing window.
//!
//! xschem takes input the way X11 delivers it: an event type, the pointer position in
//! window pixels, a key symbol, a button number and a mask of the modifier keys and
//! mouse buttons that were down *before* the event. The numbers below are those of the
//! X protocol, which xschem's key bindings are written against.

use serde::{Deserialize, Serialize};

/// Modifier and button masks (`state`).
pub mod state {
    pub const SHIFT: u32 = 1 << 0;
    pub const CAPS_LOCK: u32 = 1 << 1;
    pub const CONTROL: u32 = 1 << 2;
    /// Alt.
    pub const MOD1: u32 = 1 << 3;
    /// The "Windows"/Super key.
    pub const MOD4: u32 = 1 << 6;
    pub const BUTTON1: u32 = 1 << 8;
    pub const BUTTON2: u32 = 1 << 9;
    pub const BUTTON3: u32 = 1 << 10;
}

/// Mouse buttons (`button`).
pub mod button {
    pub const LEFT: u32 = 1;
    pub const MIDDLE: u32 = 2;
    pub const RIGHT: u32 = 3;
    /// The wheel turns arrive as presses of buttons 4 to 7.
    pub const WHEEL_UP: u32 = 4;
    pub const WHEEL_DOWN: u32 = 5;
    pub const WHEEL_LEFT: u32 = 6;
    pub const WHEEL_RIGHT: u32 = 7;
}

/// Key symbols (`keysym`). A printable Latin-1 character is its own code, with the case
/// the key produces: Shift+a is `'A'` *and* [`state::SHIFT`].
pub mod keysym {
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const RETURN: u32 = 0xff0d;
    pub const ESCAPE: u32 = 0xff1b;
    pub const HOME: u32 = 0xff50;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const END: u32 = 0xff57;
    pub const INSERT: u32 = 0xff63;
    pub const F1: u32 = 0xffbe;
    pub const SHIFT_L: u32 = 0xffe1;
    pub const SHIFT_R: u32 = 0xffe2;
    pub const CONTROL_L: u32 = 0xffe3;
    pub const CONTROL_R: u32 = 0xffe4;
    pub const ALT_L: u32 = 0xffe9;
    pub const ALT_R: u32 = 0xffea;
    pub const SUPER_L: u32 = 0xffeb;
    pub const DELETE: u32 = 0xffff;

    /// `F1` is 1.
    pub fn function(n: u32) -> u32 {
        F1 + n.saturating_sub(1)
    }

    /// The key symbol of a character: Latin-1 as is, anything else as Unicode.
    pub fn from_char(c: char) -> u32 {
        match c as u32 {
            code @ 0x20..=0x7e | code @ 0xa0..=0xff => code,
            code => 0x0100_0000 | code,
        }
    }
}

/// X event type numbers.
pub mod event {
    pub const KEY_PRESS: i32 = 2;
    pub const KEY_RELEASE: i32 = 3;
    pub const BUTTON_PRESS: i32 = 4;
    pub const BUTTON_RELEASE: i32 = 5;
    pub const MOTION_NOTIFY: i32 = 6;
    pub const ENTER_NOTIFY: i32 = 7;
    pub const LEAVE_NOTIFY: i32 = 8;
    pub const FOCUS_IN: i32 = 9;
    pub const FOCUS_OUT: i32 = 10;
    pub const EXPOSE: i32 = 12;
    pub const CONFIGURE_NOTIFY: i32 = 22;
}

/// One input event in the drawing window. Positions are in pixels of the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Input {
    Motion {
        x: i32,
        y: i32,
        state: u32,
    },
    ButtonPress {
        button: u32,
        x: i32,
        y: i32,
        state: u32,
    },
    ButtonRelease {
        button: u32,
        x: i32,
        y: i32,
        state: u32,
    },
    KeyPress {
        keysym: u32,
        x: i32,
        y: i32,
        state: u32,
    },
    KeyRelease {
        keysym: u32,
        x: i32,
        y: i32,
        state: u32,
    },
    /// The pointer entered the drawing window.
    Enter {
        x: i32,
        y: i32,
        state: u32,
    },
    Leave,
    FocusIn,
    FocusOut,
}

/// An [`Input`] as the arguments of xschem's event entry point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct XEvent {
    pub kind: i32,
    pub x: i32,
    pub y: i32,
    pub keysym: u32,
    pub button: u32,
    pub state: u32,
}

impl Input {
    pub fn to_x(self) -> XEvent {
        let at = |kind, x, y, keysym, button, state| XEvent {
            kind,
            x,
            y,
            keysym,
            button,
            state,
        };
        match self {
            Input::Motion { x, y, state } => at(event::MOTION_NOTIFY, x, y, 0, 0, state),
            Input::ButtonPress {
                button,
                x,
                y,
                state,
            } => at(event::BUTTON_PRESS, x, y, 0, button, state),
            Input::ButtonRelease {
                button,
                x,
                y,
                state,
            } => at(event::BUTTON_RELEASE, x, y, 0, button, state),
            Input::KeyPress {
                keysym,
                x,
                y,
                state,
            } => at(event::KEY_PRESS, x, y, keysym, 0, state),
            Input::KeyRelease {
                keysym,
                x,
                y,
                state,
            } => at(event::KEY_RELEASE, x, y, keysym, 0, state),
            Input::Enter { x, y, state } => at(event::ENTER_NOTIFY, x, y, 0, 0, state),
            Input::Leave => at(event::LEAVE_NOTIFY, 0, 0, 0, 0, 0),
            Input::FocusIn => at(event::FOCUS_IN, 0, 0, 0, 0, 0),
            Input::FocusOut => at(event::FOCUS_OUT, 0, 0, 0, 0, 0),
        }
    }

    /// Pointer movement: only the last of a run matters.
    pub fn is_motion(self) -> bool {
        matches!(self, Input::Motion { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_symbols() {
        assert_eq!(keysym::from_char('a'), 0x61);
        assert_eq!(keysym::from_char('A'), 0x41);
        assert_eq!(keysym::from_char('é'), 0xe9);
        assert_eq!(keysym::from_char('€'), 0x0100_20ac);
        assert_eq!(keysym::function(5), 0xffc2);
    }

    #[test]
    fn inputs_map_to_x_events() {
        let press = Input::ButtonPress {
            button: button::LEFT,
            x: 3,
            y: 4,
            state: state::SHIFT,
        };
        assert_eq!(
            press.to_x(),
            XEvent {
                kind: event::BUTTON_PRESS,
                x: 3,
                y: 4,
                keysym: 0,
                button: 1,
                state: 1
            }
        );
        let json = serde_json::to_string(&press).unwrap();
        assert!(json.contains(r#""kind":"button_press""#), "{json}");
        assert_eq!(serde_json::from_str::<Input>(&json).unwrap(), press);
    }
}
