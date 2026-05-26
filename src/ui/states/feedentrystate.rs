use std::collections::HashSet;
use std::time::Instant;

use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{ListItem, ListState},
};
use tracing::error;

use crate::{
    core::{
        feed::feedentry::FeedEntry,
        library::{feedlibrary::FeedLibrary, settings::theme::Theme},
    },
    ui::states::feedtreestate::{FeedItemInfo, FeedTreeState},
};

#[derive(Debug)]
enum ScrollMeasureMode {
    Keyboard,
    Mouse,
}

// @TODO: FeedTreeState and FeedEntryState are pretty similar...
// how much could we put into a trait for them both to share?
pub struct FeedEntryState {
    pub entries: Vec<FeedEntry>,
    pub list_state: ListState,
    pub previous_selected: String,

    // maybe these should be in MainScreen?
    // or maybe better to encapsulate things into a MouseAwareList trait
    pub last_click: Option<Instant>,
    pub visible_lines: usize,

    theme: Theme,
    last_generation: u64,
    read_later_paths: HashSet<String>,
    selected: Option<usize>,
    scroll_measure: ScrollMeasureMode,
}

impl Default for FeedEntryState {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedEntryState {
    pub fn new() -> Self {
        Self {
            entries: vec![],
            list_state: ListState::default().with_selected(Some(0)),
            previous_selected: String::new(),
            theme: Theme::default(),
            last_generation: u64::MAX,
            last_click: None,
            read_later_paths: HashSet::new(),
            visible_lines: 0,
            selected: Some(0),
            scroll_measure: ScrollMeasureMode::Keyboard,
        }
    }

    pub fn update(&mut self, library: &mut FeedLibrary, treestate: &FeedTreeState) {
        let current_selected = match treestate.get_selected() {
            Some(FeedItemInfo::Category(t)) => t.to_string(),
            Some(FeedItemInfo::Item(_, _, s)) => s.to_string(),
            Some(FeedItemInfo::ReadLater) => "read_later".to_string(),
            _ => String::new(),
        };

        self.theme = library.settings.get_theme().unwrap().clone();

        if library.generation == self.last_generation && current_selected == self.previous_selected
        {
            return;
        }

        self.last_generation = library.generation;

        let selection_changed = current_selected != self.previous_selected;
        self.previous_selected = current_selected;

        self.entries = match treestate.get_selected() {
            Some(FeedItemInfo::Category(t)) => match library.get_feed_entries_by_category(t) {
                Ok(entries) => entries,
                Err(e) => {
                    error!("Error getting feed entries by category: {:?}", e);
                    vec![]
                }
            },
            Some(FeedItemInfo::Item(_, _, s)) => match library.get_feed_entries_by_item_slug(s) {
                Ok(entries) => entries,
                Err(e) => {
                    error!("Error getting feed entries by item slug: {:?}", e);
                    vec![]
                }
            },
            Some(FeedItemInfo::ReadLater) => match library.get_read_later_feed_entries() {
                Ok(entries) => entries,
                Err(e) => {
                    error!("Error getting Read Later entries: {:?}", e);
                    vec![]
                }
            },
            _ => vec![],
        };

        // precompute read-later paths for use in get_items()
        self.read_later_paths.clear();
        if let Ok(rl_entries) = library.get_read_later_feed_entries() {
            for entry in rl_entries {
                if let Some(path) = entry.filepath.to_str() {
                    self.read_later_paths.insert(path.to_string());
                }
            }
        }

        if selection_changed {
            self.select_first();
        }
    }

    pub fn get_items(&self) -> Vec<ListItem<'_>> {
        self.entries
            .iter()
            .map(|entry| {
                let mut item_content_lines: Vec<Line> = Vec::new();

                item_content_lines.push(Line::from(""));

                let file_path = entry.filepath.to_str().unwrap_or_default();
                let read_later_icon = if self.read_later_paths.contains(file_path) {
                    " \u{f02d}" // read later icon
                } else {
                    ""
                };

                // Title
                if !entry.seen {
                    item_content_lines.push(Line::from(Span::styled(
                        format!(" \u{f1ea} {}{} \u{e3e3}", entry.title, read_later_icon),
                        Style::default()
                            .bold()
                            .fg(Color::from_u32(self.theme.base[9])),
                    )));
                } else {
                    item_content_lines.push(Line::from(Span::styled(
                        format!(" \u{f1ea} {}{}", entry.title, read_later_icon),
                        Style::default()
                            .bold()
                            .fg(Color::from_u32(self.theme.base[6])),
                    )));
                };

                // Date
                item_content_lines.push(Line::from(Span::styled(
                    format!(
                        " \u{f0520} {} | \u{f09e} {}",
                        entry.date.with_timezone(&chrono::Local).format("%Y-%m-%d"),
                        entry.author
                    ),
                    Style::default().fg(Color::from_u32(self.theme.base[5])),
                )));

                // Description
                item_content_lines.push(Line::from(Span::styled(
                    format!(" {}...", entry.description),
                    Style::default().fg(Color::from_u32(self.theme.base[4])),
                )));

                item_content_lines.push(Line::from(""));

                let item_text = Text::from(item_content_lines);
                ListItem::new(item_text)
            })
            .collect()
    }

    pub fn get_selected(&self) -> Option<FeedEntry> {
        if let Some(idx) = self.list_state.selected().or(self.selected)
            && let Some(entry) = self.entries.get(idx)
        {
            return Some(entry.clone());
        }

        None
    }

    pub fn set_current_read(&mut self) {
        if let Some(selected) = self.list_state.selected()
            && selected < self.entries.len()
        {
            self.entries[selected].seen = true;
        }
    }

    fn update_list_offset(&mut self) {
        // still overshooting the offset when going to first and last
        // first is one too high,

        let current_offset = self.list_state.offset();
        let current_selection = self.list_state.selected();

        let mut next_offset = match current_selection {
            Some(idx) if idx < current_offset => idx,
            Some(idx)
                if idx > current_offset.saturating_add(self.visible_lines.saturating_sub(1)) =>
            {
                idx.saturating_sub(self.visible_lines.saturating_sub(1))
            }
            _ => current_offset,
        }
        .min(self.max_offset());

        // offset "jump" behavior (i.e. select_next() after mousewheel scroll)
        // center the selection, favor the higher when visible_lines is even
        if next_offset.abs_diff(current_offset) > 1
            && next_offset > self.visible_lines.saturating_div(2)
        {
            if let Some(idx) = current_selection {
                next_offset = idx.saturating_sub(self.visible_lines.saturating_div(2));
                if next_offset > self.visible_lines.saturating_sub(1) {
                    next_offset = next_offset
                        .saturating_add(self.visible_lines.saturating_add(1) % 2)
                        .min(self.max_offset());
                }
            }
        };

        *self.list_state.offset_mut() = next_offset;
    }

    pub fn select_next(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        let selected = self
            .list_state
            .selected()
            .unwrap_or(self.selected.unwrap_or(0));

        if selected >= self.entries.len() {
            self.select_last();
            return;
        }

        // List::list_state.offset is retained, but gets adjusted temporarily
        // on render to get the selected item in view

        self.list_state.select(Some(selected));
        self.list_state.select_next();
        self.update_list_offset();
    }

    pub fn select_previous(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        let selected = self
            .list_state
            .selected()
            .unwrap_or(self.selected.unwrap_or(0));

        if selected >= self.entries.len() {
            self.select_last();
        } else {
            self.list_state.select(Some(selected));
            self.list_state.select_previous();
            self.update_list_offset();
        }
    }

    pub fn select_first(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        self.list_state.select_first();
        self.update_list_offset();
    }

    pub fn select_last(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        *self.list_state.offset_mut() = self.max_offset();
        self.list_state
            .select(Some(self.entries.len().saturating_sub(1)));
    }

    // users have different expectations for scrollbar position based on the
    // input method
    pub fn scroll_max(&self) -> usize {
        match self.scroll_measure {
            ScrollMeasureMode::Mouse => self.max_offset(),
            ScrollMeasureMode::Keyboard => self.entries.len().saturating_sub(1),
        }
    }

    pub fn scroll(&self) -> usize {
        match self.scroll_measure {
            ScrollMeasureMode::Mouse => self.list_state.offset(),
            ScrollMeasureMode::Keyboard => self
                .list_state
                .selected()
                .unwrap_or(self.list_state.offset()),
        }
    }

    pub fn scroll_by(&mut self, scroll_by: isize) {
        if self.entries.is_empty() || self.entries.len() < self.visible_lines {
            *self.list_state.offset_mut() = 0;
            return;
        }

        let scroll_by_u = scroll_by.abs() as usize;

        let current_selection = self.list_state.selected().or(self.selected);
        let current_offset = self.list_state.offset();

        let next_offset = match scroll_by < 0 {
            true => current_offset.saturating_sub(scroll_by_u),
            false => current_offset.saturating_add(scroll_by_u),
        }
        .min(self.max_offset());

        if let Some(current_selection) = current_selection {
            if current_selection >= (self.visible_lines + next_offset) {
                self.selected = Some(current_selection);
                self.list_state.select(None);
            } else if current_selection < next_offset {
                self.selected = Some(current_selection);
                self.list_state.select(None);
            } else {
                self.select(current_selection);
            }
        }

        *self.list_state.offset_mut() = next_offset;
    }

    pub fn select(&mut self, index: usize) {
        if index < self.entries.len() {
            self.list_state.select(Some(index));
        }

        self.update_list_offset();
    }

    fn max_offset(&self) -> usize {
        self.entries.len().saturating_sub(self.visible_lines)
    }

    pub fn set_scroll_measure_mouse(&mut self) {
        self.scroll_measure = ScrollMeasureMode::Mouse;
    }

    pub fn set_scroll_measure_key(&mut self) {
        self.scroll_measure = ScrollMeasureMode::Keyboard;
    }
}
