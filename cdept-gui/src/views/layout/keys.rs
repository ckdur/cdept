//! egui's input as KLayout's (docs/KLAYOUT.md §4), and the shortcuts that are CDePT's.

use egui::{Key, Modifiers, MouseWheelUnit, Vec2};
use libcdept::klayout::protocol::{Button, Modifiers as KModifiers};

pub fn button(button: egui::PointerButton) -> Option<Button> {
    match button {
        egui::PointerButton::Primary => Some(Button::Left),
        egui::PointerButton::Secondary => Some(Button::Right),
        egui::PointerButton::Middle => Some(Button::Middle),
        _ => None,
    }
}

pub fn modifiers(m: Modifiers) -> KModifiers {
    KModifiers {
        shift: m.shift,
        ctrl: m.ctrl || m.command,
        alt: m.alt,
    }
}

/// The name KLayout's helper knows a key by, for keys that do not type a character.
pub fn key_name(key: Key) -> Option<&'static str> {
    Some(match key {
        Key::Escape => "Escape",
        Key::Enter => "Return",
        Key::Backspace => "Backspace",
        Key::Delete => "Delete",
        Key::Tab => "Tab",
        Key::ArrowLeft => "Left",
        Key::ArrowRight => "Right",
        Key::ArrowUp => "Up",
        Key::ArrowDown => "Down",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::Home => "Home",
        Key::End => "End",
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        _ => return None,
    })
}

/// What CDePT does itself with a key, with KLayout's default shortcuts where it has them
/// (Ctrl+Z, Ctrl+Y, Delete, F2, Ctrl++, Ctrl+-; Ctrl+S is CDePT's save).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shortcut {
    Undo,
    Redo,
    Save,
    ZoomFit,
    ZoomIn,
    ZoomOut,
    SelectAll,
    Delete,
}

pub fn shortcut(key: Key, m: Modifiers) -> Option<Shortcut> {
    let command = m.command || m.ctrl;
    Some(match (key, command) {
        (Key::Z, true) if m.shift => Shortcut::Redo,
        (Key::Z, true) => Shortcut::Undo,
        (Key::Y, true) => Shortcut::Redo,
        (Key::S, true) => Shortcut::Save,
        (Key::A, true) => Shortcut::SelectAll,
        (Key::Plus | Key::Equals, true) => Shortcut::ZoomIn,
        (Key::Minus, true) => Shortcut::ZoomOut,
        (Key::F2, false) => Shortcut::ZoomFit,
        (Key::Delete, false) => Shortcut::Delete,
        _ => return None,
    })
}

/// Wheel movement as notches: a line is a notch, a page is ten, and points add up.
pub fn wheel_notches(unit: MouseWheelUnit, delta: Vec2) -> Vec2 {
    match unit {
        MouseWheelUnit::Line => delta,
        MouseWheelUnit::Page => delta * 10.0,
        MouseWheelUnit::Point => delta / 50.0,
    }
}

/// Whole notches out of the movement collected so far, as `(delta, horizontal)` in Qt
/// units (120 per notch); what is left stays in `wheel`.
pub fn take_wheel(wheel: &mut Vec2) -> Vec<(i32, bool)> {
    let mut out = Vec::new();
    for (value, horizontal) in [(&mut wheel.y, false), (&mut wheel.x, true)] {
        let whole = value.trunc();
        if whole != 0.0 {
            *value -= whole;
            out.push((whole as i32 * 120, horizontal));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn klayout_shortcuts() {
        let ctrl = Modifiers::COMMAND | Modifiers::CTRL;
        assert_eq!(shortcut(Key::Z, ctrl), Some(Shortcut::Undo));
        assert_eq!(
            shortcut(Key::Z, ctrl | Modifiers::SHIFT),
            Some(Shortcut::Redo)
        );
        assert_eq!(shortcut(Key::Y, ctrl), Some(Shortcut::Redo));
        assert_eq!(shortcut(Key::F2, Modifiers::NONE), Some(Shortcut::ZoomFit));
        assert_eq!(
            shortcut(Key::Delete, Modifiers::NONE),
            Some(Shortcut::Delete)
        );
        assert_eq!(shortcut(Key::Escape, Modifiers::NONE), None);
        assert_eq!(shortcut(Key::Z, Modifiers::NONE), None);
    }

    #[test]
    fn wheel_comes_in_notches_of_120() {
        let mut wheel = wheel_notches(MouseWheelUnit::Line, Vec2::new(0.0, -2.0));
        assert_eq!(take_wheel(&mut wheel), [(-240, false)]);
        let mut wheel = Vec2::ZERO;
        for _ in 0..3 {
            wheel += wheel_notches(MouseWheelUnit::Point, Vec2::new(0.0, 20.0));
        }
        // 60 points are 1.2 notches: one now, the rest later.
        assert_eq!(take_wheel(&mut wheel), [(120, false)]);
        assert!((wheel.y - 0.2).abs() < 1e-5);
    }

    #[test]
    fn named_keys() {
        assert_eq!(key_name(Key::Enter), Some("Return"));
        assert_eq!(key_name(Key::A), None);
    }
}
