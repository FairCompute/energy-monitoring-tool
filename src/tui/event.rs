use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;

pub enum AppEvent {
    Tick,
    Quit,
    CycleSortMode,
    ResetDisplay,
}

pub fn poll(timeout: Duration) -> Option<AppEvent> {
    if event::poll(timeout).ok()? {
        if let Ok(Event::Key(key)) = event::read() {
            return Some(handle_key(key));
        }
    }
    Some(AppEvent::Tick)
}

fn handle_key(key: KeyEvent) -> AppEvent {
    match key.code {
        KeyCode::Char('q') => AppEvent::Quit,
        KeyCode::Char('s') => AppEvent::CycleSortMode,
        KeyCode::Char('r') => AppEvent::ResetDisplay,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => AppEvent::Quit,
        _ => AppEvent::Tick,
    }
}
