use std::io::stdout;

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
};
use tracing::info;

use crate::{app, core::config::Config};

pub fn run_main_ui(config: &Config) -> color_eyre::Result<()> {
    info!("Initializing UI");

    if let Some(hooks) = &config.hooks {
        hooks.run_before_tui();
    }

    let terminal = ratatui::init();

    if config.use_mouse {
        execute!(stdout(), EnableMouseCapture).ok();

        // substitue hook to DisableMouseCapture on panic
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            execute!(stdout(), DisableMouseCapture).ok();
            ratatui::restore();
            hook(info);
        }));
    }

    let mut app = app::App::new(config);
    app.initmain();
    let result = app.run(terminal);

    execute!(stdout(), DisableMouseCapture).ok();
    ratatui::restore();

    if let Some(hooks) = &config.hooks {
        hooks.run_after_tui();
    }

    result
}
