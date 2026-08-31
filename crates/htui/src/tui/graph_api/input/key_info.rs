use crossterm::event::{self, KeyCode, KeyEvent, KeyEventKind, MediaKeyCode, ModifierKeyCode};

use crate::events::KeyEventInfo;

/// Convert a crossterm `KeyEvent` into the terminal-agnostic payload the
/// script layer dispatches as `keydown` / `keyup`, mirroring the browser
/// `KeyboardEvent` fields (`key` / `code` / `keyCode` + modifier flags). Every
/// `KeyCode` variant maps to a stable string so pages can branch on the same
/// values they would in a real browser.
pub fn key_event_info(ev: &KeyEvent) -> KeyEventInfo {
    let (key, code, key_code): (String, String, u32) = match ev.code {
        KeyCode::Backspace => ("Backspace".into(), "Backspace".into(), 8),
        KeyCode::Enter => ("Enter".into(), "Enter".into(), 13),
        KeyCode::Left => ("ArrowLeft".into(), "ArrowLeft".into(), 37),
        KeyCode::Right => ("ArrowRight".into(), "ArrowRight".into(), 39),
        KeyCode::Up => ("ArrowUp".into(), "ArrowUp".into(), 38),
        KeyCode::Down => ("ArrowDown".into(), "ArrowDown".into(), 40),
        KeyCode::Home => ("Home".into(), "Home".into(), 36),
        KeyCode::End => ("End".into(), "End".into(), 35),
        KeyCode::PageUp => ("PageUp".into(), "PageUp".into(), 33),
        KeyCode::PageDown => ("PageDown".into(), "PageDown".into(), 34),
        KeyCode::Tab | KeyCode::BackTab => ("Tab".into(), "Tab".into(), 9),
        KeyCode::Delete => ("Delete".into(), "Delete".into(), 46),
        KeyCode::Insert => ("Insert".into(), "Insert".into(), 45),
        KeyCode::F(n) => (format!("F{n}"), format!("F{n}"), 111 + n as u32),
        KeyCode::Char(c) => (c.to_string(), char_code(c), c as u32),
        KeyCode::Null => ("Unidentified".into(), "Unidentified".into(), 0),
        KeyCode::Esc => ("Escape".into(), "Escape".into(), 27),
        KeyCode::CapsLock => ("CapsLock".into(), "CapsLock".into(), 20),
        KeyCode::ScrollLock => ("ScrollLock".into(), "ScrollLock".into(), 145),
        KeyCode::NumLock => ("NumLock".into(), "NumLock".into(), 144),
        KeyCode::PrintScreen => ("PrintScreen".into(), "PrintScreen".into(), 44),
        KeyCode::Pause => ("Pause".into(), "Pause".into(), 19),
        KeyCode::Menu => ("ContextMenu".into(), "ContextMenu".into(), 93),
        KeyCode::KeypadBegin => ("Unidentified".into(), "Unidentified".into(), 0),
        KeyCode::Media(m) => media_key(m),
        KeyCode::Modifier(m) => modifier_key(m),
    };
    KeyEventInfo {
        key,
        code,
        key_code,
        shift: ev.modifiers.contains(event::KeyModifiers::SHIFT),
        ctrl: ev.modifiers.contains(event::KeyModifiers::CONTROL),
        alt: ev.modifiers.contains(event::KeyModifiers::ALT),
        meta: ev.modifiers.contains(event::KeyModifiers::SUPER)
            || ev.modifiers.contains(event::KeyModifiers::HYPER)
            || ev.modifiers.contains(event::KeyModifiers::META),
        repeat: ev.kind == KeyEventKind::Repeat,
    }
}

/// `event.code` for a character key: `KeyA`…`KeyZ`, `Digit0`…`Digit9`, `Space`
/// and `Unidentified` for everything else (matching the DOM physical-key
/// vocabulary).
fn char_code(c: char) -> String {
    if c.is_ascii_alphabetic() {
        format!("Key{}", c.to_ascii_uppercase())
    } else if c.is_ascii_digit() {
        format!("Digit{c}")
    } else if c == ' ' {
        "Space".to_string()
    } else {
        "Unidentified".to_string()
    }
}

/// `(key, code, keyCode)` for a crossterm media key.
fn media_key(m: MediaKeyCode) -> (String, String, u32) {
    let (k, c, code) = match m {
        MediaKeyCode::Play | MediaKeyCode::PlayPause => ("MediaPlayPause", "MediaPlayPause", 179),
        MediaKeyCode::Pause => ("Pause", "MediaPause", 179),
        MediaKeyCode::Reverse => ("MediaReverse", "MediaReverse", 227),
        MediaKeyCode::Stop => ("MediaStop", "MediaStop", 178),
        MediaKeyCode::FastForward => ("MediaFastForward", "MediaFastForward", 228),
        MediaKeyCode::Rewind => ("MediaRewind", "MediaRewind", 227),
        MediaKeyCode::TrackNext => ("MediaTrackNext", "MediaTrackNext", 176),
        MediaKeyCode::TrackPrevious => ("MediaTrackPrevious", "MediaTrackPrevious", 177),
        MediaKeyCode::Record => ("MediaRecord", "MediaRecord", 0),
        MediaKeyCode::LowerVolume => ("AudioVolumeDown", "AudioVolumeDown", 174),
        MediaKeyCode::RaiseVolume => ("AudioVolumeUp", "AudioVolumeUp", 175),
        MediaKeyCode::MuteVolume => ("AudioVolumeMute", "AudioVolumeMute", 173),
    };
    (k.into(), c.into(), code)
}

/// `(key, code, keyCode)` for a crossterm modifier-key press (only readable
/// with the keyboard-enhancement flags enabled on the terminal).
fn modifier_key(m: ModifierKeyCode) -> (String, String, u32) {
    let (k, c, code) = match m {
        ModifierKeyCode::LeftShift => ("Shift", "ShiftLeft", 16),
        ModifierKeyCode::RightShift => ("Shift", "ShiftRight", 16),
        ModifierKeyCode::LeftControl => ("Control", "ControlLeft", 17),
        ModifierKeyCode::RightControl => ("Control", "ControlRight", 17),
        ModifierKeyCode::LeftAlt => ("Alt", "AltLeft", 18),
        ModifierKeyCode::RightAlt => ("Alt", "AltRight", 18),
        ModifierKeyCode::LeftSuper => ("Meta", "MetaLeft", 91),
        ModifierKeyCode::RightSuper => ("Meta", "MetaRight", 91),
        ModifierKeyCode::LeftHyper => ("Hyper", "HyperLeft", 91),
        ModifierKeyCode::RightHyper => ("Hyper", "HyperRight", 91),
        ModifierKeyCode::LeftMeta => ("Meta", "MetaLeft", 91),
        ModifierKeyCode::RightMeta => ("Meta", "MetaRight", 91),
        ModifierKeyCode::IsoLevel3Shift => ("AltGraph", "AltGraph", 18),
        ModifierKeyCode::IsoLevel5Shift => ("Unidentified", "Unidentified", 0),
    };
    (k.into(), c.into(), code)
}
