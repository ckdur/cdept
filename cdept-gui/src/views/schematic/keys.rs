//! egui input as xschem expects it: X button numbers, key symbols and state masks.

use egui::{Key, Modifiers, MouseWheelUnit, PointerButton, Vec2};
use libcdept::xschem::input::{button, keysym, state};

use crate::views::EditAction;

/// The `state` mask: modifier keys, and the mouse buttons in `buttons` (a mask of
/// `state::BUTTON*`) that were down before the event.
pub fn state_mask(modifiers: Modifiers, buttons: u32) -> u32 {
    let mut mask = buttons;
    if modifiers.shift {
        mask |= state::SHIFT;
    }
    if modifiers.ctrl || modifiers.command {
        mask |= state::CONTROL;
    }
    if modifiers.alt {
        mask |= state::MOD1;
    }
    mask
}

/// The X number of a mouse button. The extra (back/forward) buttons have none xschem
/// knows.
pub fn button_number(button: PointerButton) -> Option<u32> {
    match button {
        PointerButton::Primary => Some(button::LEFT),
        PointerButton::Middle => Some(button::MIDDLE),
        PointerButton::Secondary => Some(button::RIGHT),
        PointerButton::Extra1 | PointerButton::Extra2 => None,
    }
}

/// The bit a held button has in the `state` mask.
pub fn button_mask(number: u32) -> u32 {
    match number {
        button::LEFT => state::BUTTON1,
        button::MIDDLE => state::BUTTON2,
        button::RIGHT => state::BUTTON3,
        _ => 0,
    }
}

/// Wheel movement in notches: touch pads report points, mice lines.
pub fn wheel_notches(unit: MouseWheelUnit, delta: Vec2) -> Vec2 {
    match unit {
        MouseWheelUnit::Line => delta,
        MouseWheelUnit::Point => delta / 50.0,
        MouseWheelUnit::Page => delta * 3.0,
    }
}

/// Take the whole notches out of `pending` as wheel buttons, one press each: 4 up,
/// 5 down, 6 left, 7 right. What is less than a notch stays for the next event.
pub fn take_wheel_buttons(pending: &mut Vec2) -> Vec<u32> {
    let mut buttons = Vec::new();
    let mut take = |value: &mut f32, positive: u32, negative: u32| {
        while value.abs() >= 1.0 {
            buttons.push(if *value > 0.0 { positive } else { negative });
            *value -= value.signum();
        }
    };
    take(&mut pending.y, button::WHEEL_UP, button::WHEEL_DOWN);
    take(&mut pending.x, button::WHEEL_LEFT, button::WHEEL_RIGHT);
    buttons
}

/// The standard editing shortcut of a key press, which CDePT handles itself instead of
/// passing the key on to xschem: Ctrl+Z undoes, Ctrl+Shift+Z and Ctrl+Y redo. (xschem
/// reads Ctrl+Z as "zoom out"; its own undo and redo are `u` and `Shift+U`.)
pub fn edit_shortcut(key: Key, modifiers: Modifiers) -> Option<EditAction> {
    if !(modifiers.ctrl || modifiers.command) || modifiers.alt {
        return None;
    }
    match (key, modifiers.shift) {
        (Key::Z, false) | (Key::Y, true) => Some(EditAction::Undo),
        (Key::Z, true) | (Key::Y, false) => Some(EditAction::Redo),
        _ => None,
    }
}

/// The character a key types without modifiers, for the keys that type one.
fn key_char(key: Key) -> Option<char> {
    let c = match key {
        Key::Space => ' ',
        Key::Colon => ':',
        Key::Comma => ',',
        Key::Backslash | Key::IntlBackslash => '\\',
        Key::Slash => '/',
        Key::Pipe => '|',
        Key::Questionmark => '?',
        Key::Exclamationmark => '!',
        Key::OpenBracket => '[',
        Key::CloseBracket => ']',
        Key::OpenCurlyBracket => '{',
        Key::CloseCurlyBracket => '}',
        Key::Backtick => '`',
        Key::Minus => '-',
        Key::Period => '.',
        Key::Plus => '+',
        Key::Equals => '=',
        Key::Semicolon => ';',
        Key::Quote => '\'',
        _ => {
            // Letters are named "A".."Z" and digits "0".."9".
            let name = key.name();
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_uppercase() => c.to_ascii_lowercase(),
                (Some(c), None) if c.is_ascii_digit() => c,
                _ => return None,
            }
        }
    };
    Some(c)
}

/// Whether pressing `key` also arrives as typed text (`egui::Event::Text`), which then
/// carries the exact character. Not so while Ctrl is held: egui sends no text then.
pub fn types_text(key: Key, modifiers: Modifiers) -> bool {
    key_char(key).is_some() && !(modifiers.ctrl || modifiers.command || modifiers.mac_cmd)
}

/// The key symbol for a key event. Letters follow Shift (Ctrl+Shift+s is `S`).
pub fn key_symbol(key: Key, modifiers: Modifiers) -> Option<u32> {
    if let Some(c) = key_char(key) {
        let c = if modifiers.shift {
            c.to_ascii_uppercase()
        } else {
            c
        };
        return Some(keysym::from_char(c));
    }
    let symbol = match key {
        Key::ArrowDown => keysym::DOWN,
        Key::ArrowLeft => keysym::LEFT,
        Key::ArrowRight => keysym::RIGHT,
        Key::ArrowUp => keysym::UP,
        Key::Escape => keysym::ESCAPE,
        Key::Tab => keysym::TAB,
        Key::Backspace => keysym::BACKSPACE,
        Key::Enter => keysym::RETURN,
        Key::Insert => keysym::INSERT,
        Key::Delete => keysym::DELETE,
        Key::Home => keysym::HOME,
        Key::End => keysym::END,
        Key::PageUp => keysym::PAGE_UP,
        Key::PageDown => keysym::PAGE_DOWN,
        Key::ShiftLeft => keysym::SHIFT_L,
        Key::ShiftRight => keysym::SHIFT_R,
        Key::ControlLeft => keysym::CONTROL_L,
        Key::ControlRight => keysym::CONTROL_R,
        Key::AltLeft => keysym::ALT_L,
        Key::AltRight => keysym::ALT_R,
        Key::SuperLeft | Key::SuperRight => keysym::SUPER_L,
        _ => {
            let number: u32 = key.name().strip_prefix('F')?.parse().ok()?;
            keysym::function(number)
        }
    };
    Some(symbol)
}

/// The key symbol of a typed character.
pub fn char_symbol(c: char) -> u32 {
    keysym::from_char(c)
}

/// The Tk name of a key, for keys pressed in a mirrored widget.
pub fn tk_key_name(key: Key) -> Option<&'static str> {
    Some(match key {
        Key::Enter => "Return",
        Key::Escape => "Escape",
        Key::Tab => "Tab",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers::NONE;
    const SHIFT: Modifiers = Modifiers::SHIFT;
    const CTRL: Modifiers = Modifiers::CTRL;

    #[test]
    fn state_masks() {
        assert_eq!(state_mask(NONE, 0), 0);
        assert_eq!(state_mask(SHIFT, 0), state::SHIFT);
        assert_eq!(state_mask(CTRL | SHIFT, 0), state::CONTROL | state::SHIFT);
        assert_eq!(state_mask(Modifiers::ALT, 0), state::MOD1);
        // "command" is Ctrl everywhere but on a Mac, where it is Cmd.
        assert_eq!(state_mask(Modifiers::COMMAND, 0), state::CONTROL);
        assert_eq!(
            state_mask(SHIFT, state::BUTTON1 | state::BUTTON3),
            state::SHIFT | state::BUTTON1 | state::BUTTON3
        );
    }

    #[test]
    fn buttons() {
        assert_eq!(button_number(PointerButton::Primary), Some(1));
        assert_eq!(button_number(PointerButton::Middle), Some(2));
        assert_eq!(button_number(PointerButton::Secondary), Some(3));
        assert_eq!(button_number(PointerButton::Extra1), None);
        assert_eq!(button_mask(1), state::BUTTON1);
        assert_eq!(button_mask(2), state::BUTTON2);
        assert_eq!(button_mask(3), state::BUTTON3);
        assert_eq!(button_mask(4), 0, "the wheel is never held");
    }

    #[test]
    fn wheel() {
        let mut pending = Vec2::ZERO;
        pending += wheel_notches(MouseWheelUnit::Line, Vec2::new(0.0, 1.0));
        assert_eq!(take_wheel_buttons(&mut pending), [button::WHEEL_UP]);
        pending += wheel_notches(MouseWheelUnit::Line, Vec2::new(0.0, -3.0));
        assert_eq!(take_wheel_buttons(&mut pending), [button::WHEEL_DOWN; 3]);
        assert_eq!(pending, Vec2::ZERO);

        // A touch pad sends many small movements: they add up to notches.
        let mut presses = Vec::new();
        for _ in 0..10 {
            pending += wheel_notches(MouseWheelUnit::Point, Vec2::new(0.0, -12.5));
            presses.extend(take_wheel_buttons(&mut pending));
        }
        assert_eq!(presses, [button::WHEEL_DOWN, button::WHEEL_DOWN]);

        pending = wheel_notches(MouseWheelUnit::Line, Vec2::new(1.0, 0.0));
        assert_eq!(take_wheel_buttons(&mut pending), [button::WHEEL_LEFT]);
        pending = wheel_notches(MouseWheelUnit::Line, Vec2::new(-1.0, 0.0));
        assert_eq!(take_wheel_buttons(&mut pending), [button::WHEEL_RIGHT]);
    }

    #[test]
    fn printable_keys_arrive_as_text_unless_ctrl_is_held() {
        for key in [
            Key::A,
            Key::Z,
            Key::Num5,
            Key::Space,
            Key::Minus,
            Key::Slash,
        ] {
            assert!(types_text(key, NONE), "{key:?}");
            assert!(types_text(key, SHIFT), "{key:?}");
            assert!(types_text(key, Modifiers::ALT), "{key:?}");
            assert!(!types_text(key, CTRL), "{key:?}");
        }
        for key in [
            Key::Escape,
            Key::Enter,
            Key::Tab,
            Key::F5,
            Key::ArrowUp,
            Key::Delete,
        ] {
            assert!(!types_text(key, NONE), "{key:?}");
        }
    }

    #[test]
    fn key_symbols() {
        // Control combinations: the letter, in the case Shift gives it.
        assert_eq!(key_symbol(Key::S, CTRL), Some(u32::from(b's')));
        assert_eq!(key_symbol(Key::S, CTRL | SHIFT), Some(u32::from(b'S')));
        assert_eq!(key_symbol(Key::Num1, CTRL), Some(u32::from(b'1')));
        assert_eq!(key_symbol(Key::Minus, CTRL), Some(u32::from(b'-')));
        assert_eq!(key_symbol(Key::Plus, CTRL | SHIFT), Some(u32::from(b'+')));
        // Keys that type nothing.
        assert_eq!(key_symbol(Key::Escape, NONE), Some(keysym::ESCAPE));
        assert_eq!(key_symbol(Key::Enter, NONE), Some(keysym::RETURN));
        assert_eq!(key_symbol(Key::Tab, NONE), Some(keysym::TAB));
        assert_eq!(key_symbol(Key::Backspace, NONE), Some(keysym::BACKSPACE));
        assert_eq!(key_symbol(Key::Delete, NONE), Some(keysym::DELETE));
        assert_eq!(key_symbol(Key::Insert, NONE), Some(keysym::INSERT));
        assert_eq!(key_symbol(Key::ArrowLeft, SHIFT), Some(keysym::LEFT));
        assert_eq!(key_symbol(Key::PageDown, NONE), Some(keysym::PAGE_DOWN));
        assert_eq!(key_symbol(Key::F1, NONE), Some(keysym::F1));
        assert_eq!(key_symbol(Key::F12, NONE), Some(keysym::function(12)));
        assert_eq!(key_symbol(Key::ShiftLeft, SHIFT), Some(keysym::SHIFT_L));
        assert_eq!(key_symbol(Key::ControlRight, CTRL), Some(keysym::CONTROL_R));
        assert_eq!(key_symbol(Key::Copy, NONE), None);
        assert_eq!(key_symbol(Key::BrowserBack, NONE), None);
    }

    #[test]
    fn undo_and_redo_shortcuts() {
        assert_eq!(edit_shortcut(Key::Z, CTRL), Some(EditAction::Undo));
        assert_eq!(
            edit_shortcut(Key::Z, Modifiers::COMMAND),
            Some(EditAction::Undo)
        );
        assert_eq!(edit_shortcut(Key::Z, CTRL | SHIFT), Some(EditAction::Redo));
        assert_eq!(edit_shortcut(Key::Y, CTRL), Some(EditAction::Redo));
        // Plain keys stay xschem's: `u` undoes, `Shift+U` redoes, `z` is the zoom box.
        assert_eq!(edit_shortcut(Key::Z, NONE), None);
        assert_eq!(edit_shortcut(Key::U, NONE), None);
        assert_eq!(edit_shortcut(Key::U, SHIFT), None);
        assert_eq!(edit_shortcut(Key::Y, NONE), None);
        assert_eq!(edit_shortcut(Key::Z, Modifiers::ALT), None);
        assert_eq!(edit_shortcut(Key::Z, CTRL | Modifiers::ALT), None);
        assert_eq!(edit_shortcut(Key::S, CTRL), None);
    }

    #[test]
    fn typed_characters() {
        assert_eq!(char_symbol('a'), 0x61);
        assert_eq!(char_symbol('A'), 0x41);
        assert_eq!(char_symbol('#'), 0x23);
        assert_eq!(char_symbol('ü'), 0xfc);
        assert_eq!(char_symbol('€'), 0x0100_20ac);
    }

    #[test]
    fn tk_names() {
        assert_eq!(tk_key_name(Key::Enter), Some("Return"));
        assert_eq!(tk_key_name(Key::ArrowDown), Some("Down"));
        assert_eq!(tk_key_name(Key::A), None);
    }
}
