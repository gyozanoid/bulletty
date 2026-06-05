use std::time::{Duration, Instant};
use std::{cell::RefCell, rc::Rc};

use color_eyre::Result;
use crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    widgets::{Block, List, Padding, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use tracing::error;

use crate::{
    app::AppWorkStatus,
    core::{
        config::Config,
        feed::feedentry::FeedEntry,
        hooks::AppHooks,
        library::feedlibrary::FeedLibrary,
        ui::{
            appscreen::{AppScreen, AppScreenEvent},
            instructiondetails::{InstructionCategory, InstructionDetail, ScreenInstructions},
            notification::{AppNotification, NotificationPriority},
        },
    },
    ui::{
        screens::{readerscreen::ReaderScreen, themedialog::ThemeDialog, urldialog::UrlDialog},
        states::{
            feedentrystate::FeedEntryState,
            feedtreestate::{FeedItemInfo, FeedTreeState},
        },
        tools::mouse::find_mouse,
    },
};

use super::helpdialog::HelpDialog;

// feels like these could be options in the theme system
const FEED_TREE_PADDING: u16 = 2;
const FEED_ENTRIES_PADDING: (u16, u16) = (2, 1);

// feed entry list items are rendered in FeedEntryState, should this go there?
// or should we compute it from the output of FeedEntryState::get_items?
const FEED_ENTRIES_HEIGHT: u16 = 5;

// @TODO: add config option for double click speed
const DOUBLE_CLICK_TIME: u32 = 250_000_000;

#[derive(PartialEq, Eq)]
enum MainInputState {
    Menu,
    Content,
}

pub struct MainScreen {
    library: Rc<RefCell<FeedLibrary>>,
    feedtreestate: FeedTreeState,
    feedentrystate: FeedEntryState,
    inputstate: MainInputState,
    hooks: Rc<AppHooks>,
    config: Rc<Config>,
    layout: Rc<[Rect]>,
}

impl MainScreen {
    pub fn new(library: Rc<RefCell<FeedLibrary>>, hooks: Rc<AppHooks>, config: Rc<Config>) -> Self {
        Self {
            library,
            feedtreestate: FeedTreeState::new(),
            feedentrystate: FeedEntryState::new(),
            inputstate: MainInputState::Menu,
            hooks,
            config,
            layout: Rc::default(),
        }
    }

    fn set_all_read(&self) {
        let entries = match self.feedtreestate.get_selected() {
            Some(FeedItemInfo::Category(t)) => {
                match self.library.borrow().get_feed_entries_by_category(t) {
                    Ok(entries) => entries,
                    Err(e) => {
                        error!("Error getting feed entries by category: {:?}", e);
                        vec![]
                    }
                }
            }
            Some(FeedItemInfo::Item(_, _, s)) => {
                match self.library.borrow().get_feed_entries_by_item_slug(s) {
                    Ok(entries) => entries,
                    Err(e) => {
                        error!("Error getting feed entries by item slug: {:?}", e);
                        vec![]
                    }
                }
            }
            Some(FeedItemInfo::ReadLater) => {
                match self.library.borrow_mut().get_read_later_feed_entries() {
                    Ok(entries) => entries,
                    Err(e) => {
                        error!("Error getting Read Later entries: {:?}", e);
                        vec![]
                    }
                }
            }
            _ => vec![],
        };

        let mut lib = self.library.borrow_mut();
        for entry in entries.iter() {
            lib.data.set_entry_seen(entry);
        }
        lib.bump_generation();
    }

    fn open_external_url(&self, url: &str) -> Result<AppScreenEvent> {
        if self.hooks.run_open_link(url) {
            return Ok(AppScreenEvent::Notify(AppNotification::new(
                "Link opened externally with user hook",
                NotificationPriority::Low,
            )));
        }

        match open::that_detached(url) {
            Ok(_) => Ok(AppScreenEvent::Notify(AppNotification::new(
                "Link opened externally",
                NotificationPriority::Low,
            ))),
            Err(_) => {
                error!("Couldn't invoke system browser");
                Ok(AppScreenEvent::OpenDialog(Box::new(UrlDialog::new(
                    url.to_string(),
                ))))
            }
        }
    }

    fn open_theme_selector(&self) -> Result<AppScreenEvent> {
        Ok(AppScreenEvent::OpenDialog(Box::new(ThemeDialog::new(
            self.library.clone(),
        ))))
    }

    fn toggle_read_later(&mut self, entry: &FeedEntry) -> bool {
        let file_path = entry.filepath.to_str().unwrap_or_default();

        if self.library.borrow_mut().is_in_read_later(file_path) {
            if let Err(e) = self.library.borrow_mut().remove_from_read_later(file_path) {
                error!("Failed to remove from read later: {:?}", e);
            }
            false
        } else {
            if let Err(e) = self.library.borrow_mut().add_to_read_later(entry) {
                error!("Failed to add entry to read later: {:?}", e);
            }
            true
        }
    }

    fn increase_tree_width(&mut self) -> color_eyre::Result<()> {
        let mut l = self.library.borrow_mut();
        l.settings.appearance.main_screen_tree_width = l
            .settings
            .appearance
            .main_screen_tree_width
            .saturating_add(2)
            .max(30)
            .min(100);
        l.settings.appearance.save()
    }

    fn decrease_tree_width(&mut self) -> color_eyre::Result<()> {
        let mut l = self.library.borrow_mut();
        l.settings.appearance.main_screen_tree_width = l
            .settings
            .appearance
            .main_screen_tree_width
            .saturating_sub(2)
            .max(30)
            .min(100);
        l.settings.appearance.save()
    }

    fn calculate_layout(&mut self, area: Rect) {
        let treewidth = self
            .library
            .borrow()
            .settings
            .appearance
            .main_screen_tree_width;

        self.layout = Layout::horizontal([
            Constraint::Length(treewidth),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .split(area)
    }
}

impl AppScreen for MainScreen {
    fn start(&mut self) {
        if !matches!(self.config.tui_auto_update, Some(false)) {
            self.library.borrow_mut().start_updater();
        }
    }

    fn quit(&mut self) {}

    fn pause(&mut self) {}

    fn unpause(&mut self) {}

    fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        self.library.borrow_mut().update();

        let theme = {
            let library = self.library.borrow();
            library.settings.get_theme().unwrap().clone()
        };

        self.calculate_layout(area);

        // Feed tree
        self.feedtreestate.update(&mut self.library.borrow_mut());
        self.feedtreestate.visible_lines =
            usize::from(self.layout[0].height.saturating_sub(FEED_TREE_PADDING * 2));
        // .min(self.feedtreestate.treeitems.len());

        let (treestyle, treeselectionstyle) = if self.inputstate == MainInputState::Menu {
            (
                Block::default()
                    .fg(Color::from_u32(theme.base[5]))
                    .bg(Color::from_u32(theme.base[1]))
                    .padding(Padding::uniform(FEED_TREE_PADDING)),
                Style::default()
                    .fg(Color::from_u32(theme.base[0x2]))
                    .bg(Color::from_u32(theme.base[0x8])),
            )
        } else {
            (
                Block::default()
                    .fg(Color::from_u32(theme.base[4]))
                    .bg(Color::from_u32(theme.base[1]))
                    .padding(Padding::uniform(FEED_TREE_PADDING)),
                Style::default()
                    .fg(Color::from_u32(theme.base[5]))
                    .bg(Color::from_u32(theme.base[2])),
            )
        };

        let treelist = List::new(self.feedtreestate.get_items())
            .block(treestyle)
            .highlight_style(treeselectionstyle);

        let mut treestate = self.feedtreestate.list_state;
        frame.render_stateful_widget(treelist, self.layout[0], &mut treestate);

        // The feed entries
        self.feedentrystate
            .update(&mut self.library.borrow_mut(), &self.feedtreestate);
        self.feedentrystate.visible_lines = usize::from(
            self.layout[0]
                .height
                .saturating_sub(FEED_ENTRIES_PADDING.1 * 2)
                / FEED_ENTRIES_HEIGHT,
        );
        // .min(self.feedentrystate.entries.len());

        let mut entryliststate = self.feedentrystate.list_state;

        let entryselectionstyle = if self.inputstate == MainInputState::Content {
            Style::default()
                .fg(Color::from_u32(theme.base[0x2]))
                .bg(Color::from_u32(theme.base[0x8]))
        } else {
            Style::default().bg(Color::from_u32(theme.base[2]))
        };

        let list_widget = List::new(self.feedentrystate.get_items())
            .block(
                Block::default()
                    .style(Style::default().bg(Color::from_u32(theme.base[2])))
                    .padding(Padding::symmetric(
                        FEED_ENTRIES_PADDING.0,
                        FEED_ENTRIES_PADDING.1,
                    )),
            )
            .highlight_style(entryselectionstyle);

        frame.render_stateful_widget(list_widget, self.layout[1], &mut entryliststate);

        // Scrollbar
        let mut scrollbarstate = ScrollbarState::new(self.feedentrystate.scroll_max())
            .position(self.feedentrystate.scroll());
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight).style(
            Style::new()
                .fg(Color::from_u32(theme.base[3]))
                .bg(Color::from_u32(theme.base[2])),
        );
        frame.render_stateful_widget(scrollbar, self.layout[2], &mut scrollbarstate);
    }

    fn handle_event(&mut self, event: Event) -> Result<AppScreenEvent> {
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_keypress(key),
            Event::Mouse(mouse_event) => self.handle_mouse(mouse_event),
            Event::Resize(_, _) => Ok(AppScreenEvent::None),
            _ => Ok(AppScreenEvent::None),
        }
    }

    fn handle_mouse(&mut self, mouse_event: MouseEvent) -> Result<AppScreenEvent> {
        match find_mouse(&mouse_event, &self.layout) {
            // would it be clearer to have a enum for layout slots/chunks?
            // this feature branch is getting to be a pretty big diff already...
            Some(0) /*Feed List*/ => {
                match mouse_event.kind {
                    MouseEventKind::ScrollDown => { self.feedtreestate.scroll_by(1); },
                    MouseEventKind::ScrollUp => { self.feedtreestate.scroll_by(-1); },
                    MouseEventKind::Up(MouseButton::Left) => {
                        self.inputstate = MainInputState::Menu;
                        let click_row = mouse_event.row.saturating_sub(self.layout[0].top() + FEED_TREE_PADDING);
                        let current_offset = self.feedtreestate.list_state.offset();
                        self.feedtreestate.select(current_offset + usize::from(click_row));
                    },
                    _ => {},
                }
            },
            Some(1) /*Feed Entries List*/ => {
                self.feedentrystate.set_scroll_measure_mouse();
                match mouse_event.kind {
                    MouseEventKind::ScrollDown => { self.feedentrystate.scroll_by(1); },
                    MouseEventKind::ScrollUp => { self.feedentrystate.scroll_by(-1); },
                    MouseEventKind::Up(MouseButton::Left) => {
                        // @TODO: this is a lot, move it somewhere else!
                        self.inputstate = MainInputState::Content;

                        let click_row = mouse_event.row
                                .saturating_sub(self.layout[1].top() + FEED_ENTRIES_PADDING.1)
                                .saturating_div(FEED_ENTRIES_HEIGHT);
                        let current_offset = self.feedentrystate.list_state.offset();
                        let new_selected = current_offset + usize::from(click_row);

                        // @TODO: maybe just check for double click by testing the cell coords instead? then we can make it more reusable
                        if let Some(last_click) = self.feedentrystate.last_click {
                            let double_click = new_selected == self.feedentrystate.list_state.selected().unwrap_or(0);
                            let double_click_time = Instant::now().duration_since(last_click);

                            if double_click && double_click_time < Duration::new(0, DOUBLE_CLICK_TIME) {
                                // duplicated from key handler for enter below
                                // @TODO: build MainScreen::open_post
                                if let Some(entry) = self.feedentrystate.get_selected() {
                                    self.library.borrow_mut().set_entry_seen(&entry);
                                    self.feedentrystate.set_current_read();

                                    return Ok(AppScreenEvent::ChangeState(Box::new(ReaderScreen::new(
                                        self.library.clone(),
                                        self.feedentrystate.entries.clone(),
                                        self.feedentrystate.list_state.selected().unwrap_or(0),
                                        self.hooks.clone(),
                                    ))));
                                }
                            }
                        }

                        self.feedentrystate.select(new_selected);
                        self.feedentrystate.last_click = Some(Instant::now());
                    },
                    _ => {},
                }
            },
            Some(2) /*Feed Entries Scrollbar*/ => {
                // @TODO: somebody out there doesn't have a scroll wheel, so i guess we better do something here
            },
            _ => {},
        }
        Ok(AppScreenEvent::None)
    }

    fn handle_keypress(&mut self, key: crossterm::event::KeyEvent) -> Result<AppScreenEvent> {
        self.feedentrystate.set_scroll_measure_key();
        match self.inputstate {
            MainInputState::Menu => match (key.modifiers, key.code) {
                (_, KeyCode::Esc | KeyCode::Char('q'))
                | (KeyModifiers::CONTROL, KeyCode::Char('c') | KeyCode::Char('C')) => {
                    Ok(AppScreenEvent::ExitApp)
                }
                (_, KeyCode::Down | KeyCode::Char('j')) => {
                    self.feedtreestate.select_next();
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Up | KeyCode::Char('k')) => {
                    self.feedtreestate.select_previous();
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Home | KeyCode::Char('g')) => {
                    self.feedtreestate.select_first();
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::End | KeyCode::Char('G')) => {
                    self.feedtreestate.select_last();
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Right | KeyCode::Enter | KeyCode::Tab | KeyCode::Char('l')) => {
                    self.inputstate = MainInputState::Content;
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Char('R')) => {
                    self.set_all_read();
                    Ok(AppScreenEvent::Notify(AppNotification::new(
                        "All marked as Read",
                        NotificationPriority::Low,
                    )))
                }
                (_, KeyCode::Char('U')) => {
                    let mut feed_library = self.library.borrow_mut();
                    if feed_library.updater.is_none() {
                        feed_library.start_updater();
                    }
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Char('>')) => {
                    self.increase_tree_width()?;
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Char('<')) => {
                    self.decrease_tree_width()?;
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Char('n')) => {
                    self.feedtreestate.select_next_category();
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Char('p')) => {
                    self.feedtreestate.select_previous_category();
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Char('t')) => self.open_theme_selector(),
                (_, KeyCode::Char('?')) => Ok(AppScreenEvent::OpenDialog(Box::new(
                    HelpDialog::new(self.library.clone(), self.get_full_instructions()),
                ))),

                // @TODO: remember to remove these!
                (_, KeyCode::Char('J')) => {
                    self.feedtreestate.scroll_by(1);
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Char('K')) => {
                    self.feedtreestate.scroll_by(-1);
                    Ok(AppScreenEvent::None)
                }

                _ => Ok(AppScreenEvent::None),
            },
            MainInputState::Content => match (key.modifiers, key.code) {
                (_, KeyCode::Char('q'))
                | (KeyModifiers::CONTROL, KeyCode::Char('c') | KeyCode::Char('C')) => {
                    Ok(AppScreenEvent::ExitApp)
                }
                (_, KeyCode::Down | KeyCode::Char('j')) => {
                    self.feedentrystate.select_next();
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Up | KeyCode::Char('k')) => {
                    self.feedentrystate.select_previous();
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Home | KeyCode::Char('g')) => {
                    self.feedentrystate.select_first();
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::End | KeyCode::Char('G')) => {
                    self.feedentrystate.select_last();
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Esc) => {
                    self.inputstate = MainInputState::Menu;
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Left | KeyCode::Char('h')) => {
                    self.inputstate = MainInputState::Menu;
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Enter) => {
                    if let Some(entry) = self.feedentrystate.get_selected() {
                        self.library.borrow_mut().set_entry_seen(&entry);
                        self.feedentrystate.set_current_read();

                        Ok(AppScreenEvent::ChangeState(Box::new(ReaderScreen::new(
                            self.library.clone(),
                            self.feedentrystate.entries.clone(),
                            self.feedentrystate.list_state.selected().unwrap_or(0),
                            self.hooks.clone(),
                        ))))
                    } else {
                        Ok(AppScreenEvent::None)
                    }
                }
                (_, KeyCode::Char('r')) => {
                    if let Some(entry) = self.feedentrystate.get_selected() {
                        let was_seen = entry.seen;
                        self.library.borrow_mut().toggle_entry_seen(&entry);
                        let message = if was_seen {
                            "Marked as Unread"
                        } else {
                            "Marked as Read"
                        };
                        Ok(AppScreenEvent::Notify(AppNotification::new(
                            message,
                            NotificationPriority::Low,
                        )))
                    } else {
                        Ok(AppScreenEvent::None)
                    }
                }
                (_, KeyCode::Char('R')) => {
                    self.set_all_read();
                    Ok(AppScreenEvent::Notify(AppNotification::new(
                        "All marked as Read",
                        NotificationPriority::Low,
                    )))
                }
                (_, KeyCode::Char('>')) => {
                    self.increase_tree_width()?;
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Char('<')) => {
                    self.decrease_tree_width()?;
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Char('o')) => {
                    if let Some(entry) = self.feedentrystate.get_selected() {
                        self.library.borrow_mut().set_entry_seen(&entry);
                        self.open_external_url(&entry.url)
                    } else {
                        Ok(AppScreenEvent::Notify(AppNotification::new(
                            "Couldn't open link externally",
                            NotificationPriority::High,
                        )))
                    }
                }
                (_, KeyCode::Char('L')) => {
                    if let Some(entry) = self.feedentrystate.get_selected() {
                        let added = self.toggle_read_later(&entry);
                        let message = if added {
                            "Added to Read Later"
                        } else {
                            "Removed from Read Later"
                        };
                        Ok(AppScreenEvent::Notify(AppNotification::new(
                            message,
                            NotificationPriority::Low,
                        )))
                    } else {
                        Ok(AppScreenEvent::None)
                    }
                }
                (_, KeyCode::Char('U')) => {
                    let mut feed_library = self.library.borrow_mut();
                    if feed_library.updater.is_none() {
                        feed_library.start_updater();
                    }
                    Ok(AppScreenEvent::None)
                }
                (_, KeyCode::Char('t')) => self.open_theme_selector(),
                (_, KeyCode::Char('?')) => Ok(AppScreenEvent::OpenDialog(Box::new(
                    HelpDialog::new(self.library.clone(), self.get_full_instructions()),
                ))),
                _ => Ok(AppScreenEvent::None),
            },
        }
    }

    fn get_title(&self) -> String {
        String::from("Main")
    }

    // TODO: format this better
    fn get_instructions(&self) -> String {
        if self.inputstate == MainInputState::Menu {
            String::from(
                "?: Help | j/k/↓/↑: move | n/p: next/prev category | Enter: select | Esc: quit",
            )
        } else {
            String::from(
                "?: Help | j/k/↓/↑: move | o: open | L: add/remove read later | Enter: read | Esc: back",
            )
        }
    }

    fn get_work_status(&self) -> AppWorkStatus {
        self.library.borrow().get_update_status()
    }

    fn get_full_instructions(&self) -> ScreenInstructions {
        ScreenInstructions::new(vec![
            InstructionCategory::new(
                "Navigation",
                vec![
                    InstructionDetail::new("j/k/↓/↑", "move selection"),
                    InstructionDetail::new("n/p", "next/previous category"),
                    InstructionDetail::new("g/G/Home/End", "beginning and end of list"),
                ],
            ),
            InstructionCategory::new(
                "Actions",
                vec![
                    InstructionDetail::new("o", "open link externally"),
                    InstructionDetail::new("L", "add/remove read later"),
                    InstructionDetail::new("Enter", "select category or read entry"),
                    InstructionDetail::new("r", "toggle item read state"),
                    InstructionDetail::new("R", "mark all items as read"),
                    InstructionDetail::new("U", "Fetch all feed updates"),
                ],
            ),
            InstructionCategory::new(
                "App",
                vec![
                    InstructionDetail::new("</>", "change feed column width"),
                    InstructionDetail::new("t", "open theme picker"),
                    InstructionDetail::new("Esc/q", "back from entries or quit"),
                ],
            ),
        ])
    }
}
