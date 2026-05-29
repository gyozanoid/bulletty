use std::collections::HashMap;

use ratatui::widgets::{ListItem, ListState};

use crate::core::library::feedlibrary::FeedLibrary;

pub enum FeedItemInfo {
    /// Represents the category title
    Category(String),
    /// Represents an item in the feed tree with a title, categore, and slug
    Item(String, String, String),
    /// Represents a separator in the menu
    Separator,
    /// Represents the Read Later category
    ReadLater,
}

// @TODO: FeedTreeState and FeedEntryState are pretty similar...
// how much could we put into a trait for them both to share?
pub struct FeedTreeState {
    pub treeitems: Vec<FeedItemInfo>,
    pub list_state: ListState,

    // maybe this should be in MainScreen?
    pub visible_lines: usize,

    last_generation: u64,
    unread_counts: HashMap<(String, String), u16>,
    read_later_count: usize,
    selected: Option<usize>,
}

impl Default for FeedTreeState {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedTreeState {
    pub fn new() -> Self {
        Self {
            treeitems: vec![],
            list_state: ListState::default().with_selected(Some(0)),
            last_generation: u64::MAX,
            unread_counts: HashMap::new(),
            read_later_count: 0,
            visible_lines: 0,
            selected: Some(0),
        }
    }

    pub fn update(&mut self, library: &mut FeedLibrary) {
        if library.generation == self.last_generation {
            return;
        }
        self.last_generation = library.generation;

        self.treeitems.clear();
        self.unread_counts.clear();

        for category in library.feedcategories.iter() {
            self.treeitems
                .push(FeedItemInfo::Category(category.title.clone()));
            for item in category.feeds.iter() {
                self.treeitems.push(FeedItemInfo::Item(
                    item.title.clone(),
                    category.title.clone(),
                    item.slug.clone(),
                ));

                if let Ok(count) = library.data.get_unread_feed(&category.title, &item.slug) {
                    self.unread_counts
                        .insert((category.title.clone(), item.slug.clone()), count);
                }
            }
        }

        // display Read Later section if it has entries
        match library.get_read_later_feed_entries() {
            Ok(entries) if !entries.is_empty() => {
                self.read_later_count = entries.len();
                self.treeitems.push(FeedItemInfo::Separator);
                self.treeitems.push(FeedItemInfo::ReadLater);
            }
            _ => {
                self.read_later_count = 0;
            }
        }

        // move selection to the last item if the list gets truncated,
        // most likely because Read Later became empty
        if let Some(idx) = self.list_state.selected().or(self.selected)
            && idx >= self.treeitems.len()
        {
            self.select_last();
        }
    }

    pub fn get_items(&self) -> Vec<ListItem<'_>> {
        self.treeitems
            .iter()
            .map(|item| {
                let title = match item {
                    FeedItemInfo::Category(t) => format!("\u{f07c} {t}"),
                    FeedItemInfo::Item(t, c, s) => {
                        let unread = self
                            .unread_counts
                            .get(&(c.clone(), s.clone()))
                            .copied()
                            .unwrap_or(0);
                        if unread > 0 {
                            format!(" \u{f09e}  {t} ({unread})")
                        } else {
                            format!(" \u{f09e}  {t}")
                        }
                    }
                    FeedItemInfo::Separator => "".to_string(),
                    FeedItemInfo::ReadLater => {
                        if self.read_later_count > 0 {
                            format!("\u{f02d} Read Later ({})", self.read_later_count)
                        } else {
                            "\u{f02d} Read Later".to_string()
                        }
                    }
                };

                ListItem::new(title.clone())
            })
            .collect()
    }

    pub fn get_selected(&self) -> Option<&FeedItemInfo> {
        if let Some(idx) = self.list_state.selected().or(self.selected)
            && let Some(entry) = self.treeitems.get(idx)
        {
            return Some(entry);
        }

        None
    }

    // find the best list offset for showing the current selection
    fn update_list_offset(&mut self) {
        let current_offset = self.list_state.offset();
        let visible_selection = self.list_state.selected();

        let mut next_offset = match visible_selection {
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
        // center the selection and favor the higher line when visible_lines is even
        if next_offset.abs_diff(current_offset) > 1
            && let Some(idx) = visible_selection
        {
            next_offset = idx.saturating_sub(self.visible_lines.saturating_div(2));
            if next_offset > 0 {
                // make sure we're centered with odd # of visible_lines
                next_offset = next_offset.saturating_add(self.visible_lines.saturating_add(1) % 2);
            }
        }

        *self.list_state.offset_mut() = next_offset.min(self.max_offset());
    }

    pub fn select_next(&mut self) {
        let selected = self.list_state.selected().or(self.selected);

        match selected {
            Some(idx) if matches!(self.treeitems.get(idx + 1), Some(_)) => {
                self.list_state.select(selected);
                self.list_state.select_next();

                if self.is_selected_separator() {
                    self.select_next();
                }

                self.update_list_offset();
            }
            _ => {
                self.select_last();
            }
        }
    }

    pub fn select_previous(&mut self) {
        let selected = self.list_state.selected().or(self.selected);

        match selected {
            Some(idx) if matches!(self.treeitems.get(idx.saturating_sub(1)), Some(_)) => {
                self.list_state.select(selected);
                self.list_state.select_previous();

                if self.is_selected_separator() {
                    self.select_previous();
                }

                self.update_list_offset();
            }
            _ => {
                self.select_last();
            }
        }
    }

    pub fn select_first(&mut self) {
        if self.treeitems.is_empty() {
            return;
        }

        self.list_state.select_first();
        self.update_list_offset();
    }

    pub fn select_last(&mut self) {
        if self.treeitems.is_empty() {
            return;
        }

        self.list_state
            .select(Some(self.treeitems.len().saturating_sub(1)));
        self.update_list_offset();
    }

    pub fn select_next_category(&mut self) {
        let current = self.list_state.selected().or(self.selected).unwrap_or(0);
        for (i, item) in self.treeitems.iter().enumerate().skip(current + 1) {
            if matches!(item, FeedItemInfo::Category(_) | FeedItemInfo::ReadLater) {
                self.list_state.select(Some(i));
                self.update_list_offset();
                return;
            }
        }
    }

    pub fn select_previous_category(&mut self) {
        let current = self.list_state.selected().or(self.selected).unwrap_or(0);
        for (i, item) in self.treeitems.iter().enumerate().take(current).rev() {
            if matches!(item, FeedItemInfo::Category(_) | FeedItemInfo::ReadLater) {
                self.list_state.select(Some(i));
                self.update_list_offset();
                return;
            }
        }
    }

    fn is_selected_separator(&self) -> bool {
        if let Some(index) = self.list_state.selected() {
            index < self.treeitems.len() && matches!(self.treeitems[index], FeedItemInfo::Separator)
        } else {
            false
        }
    }

    pub fn scroll_by(&mut self, scroll_by: isize) {
        if self.treeitems.is_empty() || self.treeitems.len() < self.visible_lines {
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
        match self.treeitems.get(index) {
            Some(FeedItemInfo::Separator) => {}
            Some(_) => self.list_state.select(Some(index)),
            _ => {}
        }

        self.update_list_offset();
    }

    fn max_offset(&self) -> usize {
        self.treeitems.len().saturating_sub(self.visible_lines)
    }
}
