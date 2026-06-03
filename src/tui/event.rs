use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;

pub enum AppEvent {
    Tick,
    Quit,
    CycleSortMode,
    ResetDisplay,
    SelectPrevious,
    SelectNext,
    ExpandSelected,
    CollapseSelected,
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
        KeyCode::Up => AppEvent::SelectPrevious,
        KeyCode::Down => AppEvent::SelectNext,
        KeyCode::Enter | KeyCode::Right => AppEvent::ExpandSelected,
        KeyCode::Esc | KeyCode::Left => AppEvent::CollapseSelected,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => AppEvent::Quit,
        _ => AppEvent::Tick,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn maps_navigation_keys_to_app_events() {
        assert!(matches!(
            handle_key(key(KeyCode::Up)),
            AppEvent::SelectPrevious
        ));
        assert!(matches!(
            handle_key(key(KeyCode::Down)),
            AppEvent::SelectNext
        ));
        assert!(matches!(
            handle_key(key(KeyCode::Enter)),
            AppEvent::ExpandSelected
        ));
        assert!(matches!(
            handle_key(key(KeyCode::Right)),
            AppEvent::ExpandSelected
        ));
        assert!(matches!(
            handle_key(key(KeyCode::Esc)),
            AppEvent::CollapseSelected
        ));
        assert!(matches!(
            handle_key(key(KeyCode::Left)),
            AppEvent::CollapseSelected
        ));
    }
}
