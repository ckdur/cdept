//! The desktop's clipboard, read for xschem.
//!
//! egui hands over the clipboard only when the user pastes (`Event::Paste`), and has no
//! primary selection (the text last selected, which X programs paste with the middle
//! button). xschem's Tk code may ask for either (`clipboard get`, `selection get`), and
//! Tk's text fields paste the primary selection with the middle button. Both are read
//! here directly from the desktop.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, MutexGuard};

use egui::Context;
use libcdept::xschem::protocol::UiEvent;

/// Longest text passed on to xschem, in bytes; more is left out.
const MAX_TEXT: usize = 1 << 20;

/// One connection to the desktop's clipboard for the whole program (made on first
/// use; `None` when there is no clipboard to reach).
fn desktop() -> MutexGuard<'static, Option<Option<arboard::Clipboard>>> {
    static DESKTOP: Mutex<Option<Option<arboard::Clipboard>>> = Mutex::new(None);
    DESKTOP.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the script driver says the selections hold, in place of the desktop's.
static FAKE: Mutex<Option<(Option<String>, Option<String>)>> = Mutex::new(None);

/// For the script driver: let [`read`] answer with these texts (clipboard, primary
/// selection) instead of asking the desktop.
pub fn pretend(clipboard: Option<String>, primary: Option<String>) {
    *FAKE.lock().unwrap_or_else(|e| e.into_inner()) = Some((clipboard, primary));
}

/// Which of the desktop's selections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    Clipboard,
    Primary,
}

impl Selection {
    /// Tk's name for it.
    pub fn tk_name(self) -> &'static str {
        match self {
            Selection::Clipboard => "CLIPBOARD",
            Selection::Primary => "PRIMARY",
        }
    }
}

/// The text of a selection, if it holds text. May wait a little for the program that
/// owns it.
pub fn read(which: Selection) -> Option<String> {
    if let Some((clipboard, primary)) = FAKE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return match which {
            Selection::Clipboard => clipboard,
            Selection::Primary => primary,
        };
    }
    let mut desktop = desktop();
    let clipboard = desktop
        .get_or_insert_with(|| match arboard::Clipboard::new() {
            Ok(clipboard) => Some(clipboard),
            Err(error) => {
                tracing::debug!(target: "xschem", "no desktop clipboard: {error}");
                None
            }
        })
        .as_mut()?;
    let text = match which {
        Selection::Clipboard => clipboard.get_text(),
        #[cfg(target_os = "linux")]
        Selection::Primary => {
            use arboard::{GetExtLinux, LinuxClipboardKind};
            clipboard
                .get()
                .clipboard(LinuxClipboardKind::Primary)
                .text()
        }
        #[cfg(not(target_os = "linux"))]
        Selection::Primary => return None,
    };
    let mut text = text.ok()?;
    if text.len() > MAX_TEXT {
        let mut end = MAX_TEXT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    Some(text)
}

/// Keeps xschem's idea of the desktop's clipboard and primary selection up to date.
///
/// [`ClipboardSync::refresh`] reads both in the background (the program that owns a selection
/// may take a moment to answer); [`ClipboardSync::poll`] returns what changed since it was last
/// passed on, as events for the helper.
#[derive(Default)]
pub struct ClipboardSync {
    reading: Option<Receiver<[Option<String>; 2]>>,
    /// Again requested while a read was going on.
    again: bool,
    /// What xschem was last told, clipboard and primary selection.
    told: [Option<String>; 2],
}

impl ClipboardSync {
    /// Read the desktop's selections again.
    pub fn refresh(&mut self, ctx: &Context) {
        if self.reading.is_some() {
            self.again = true;
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("clipboard".into())
            .spawn(move || {
                let texts = [read(Selection::Clipboard), read(Selection::Primary)];
                let _ = sender.send(texts);
                ctx.request_repaint();
            });
        if spawned.is_ok() {
            self.reading = Some(receiver);
        }
    }

    /// The user pasted `text` from the clipboard: xschem gets it too.
    pub fn pasted(&mut self, text: &str) -> Option<UiEvent> {
        self.changed(Selection::Clipboard, text.to_string())
    }

    /// The helper was started again: it knows nothing yet.
    pub fn forget(&mut self) {
        self.told = [None, None];
    }

    /// What changed since xschem was last told.
    pub fn poll(&mut self, ctx: &Context) -> Vec<UiEvent> {
        let Some(receiver) = &self.reading else {
            return Vec::new();
        };
        let [clipboard, primary] = match receiver.try_recv() {
            Ok(texts) => texts,
            Err(mpsc::TryRecvError::Empty) => return Vec::new(),
            Err(mpsc::TryRecvError::Disconnected) => [None, None],
        };
        self.reading = None;
        if std::mem::take(&mut self.again) {
            self.refresh(ctx);
        }
        let mut events = Vec::new();
        for (which, text) in [
            (Selection::Clipboard, clipboard),
            (Selection::Primary, primary),
        ] {
            if let Some(event) = text.and_then(|text| self.changed(which, text)) {
                events.push(event);
            }
        }
        events
    }

    fn changed(&mut self, which: Selection, text: String) -> Option<UiEvent> {
        let slot = &mut self.told[which as usize];
        if slot.as_deref() == Some(text.as_str()) {
            return None;
        }
        *slot = Some(text.clone());
        Some(UiEvent::Clipboard {
            selection: which.tk_name().to_string(),
            text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_news_is_passed_on() {
        let mut sync = ClipboardSync::default();
        let event = sync.pasted("hello");
        assert_eq!(
            event,
            Some(UiEvent::Clipboard {
                selection: "CLIPBOARD".into(),
                text: "hello".into()
            })
        );
        assert_eq!(sync.pasted("hello"), None);
        assert_eq!(
            sync.changed(Selection::Primary, "hello".into()),
            Some(UiEvent::Clipboard {
                selection: "PRIMARY".into(),
                text: "hello".into()
            })
        );
        sync.forget();
        assert!(sync.pasted("hello").is_some());
    }
}
