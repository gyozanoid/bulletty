use std::rc::Rc;

use crossterm::event::MouseEvent;
use ratatui::layout::{Position, Rect};

// @TODO: probably a better place for these
pub fn mouse_inside(mouse_event: &MouseEvent, bounds: &Rect) -> bool {
    let pos = Position::new(mouse_event.column, mouse_event.row);
    bounds.contains(pos)
}

pub fn find_mouse(mouse_event: &MouseEvent, rects: &Rc<[Rect]>) -> Option<usize> {
    rects
        .into_iter()
        .position(|bounds| mouse_inside(mouse_event, bounds))
}
