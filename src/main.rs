use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    time::{Duration, Instant},
};

use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
};
use diffio::{app::App, git::Repository, ui, worker::Worker};

#[derive(Parser)]
#[command(
    version,
    about = "A read-only, live Git diff viewer",
    after_help = "Inside diffio: ? help, 1/2/3 views, v layout, t tree, q quit."
)]
struct Args {
    /// Repository or subdirectory to watch (defaults to the current directory)
    #[arg(default_value = ".")]
    path: PathBuf,
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableMouseCapture);
        ratatui::restore();
    }
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("diffio: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let repo = Repository::discover(&args.path)?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err("an interactive terminal is required; use --help for usage".into());
    }
    let mut app = App::new(repo.root.clone());
    let mut worker = Worker::new(repo)?;
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableMouseCapture);
        ratatui::restore();
        previous_hook(info);
    }));
    let _guard = TerminalGuard;
    let mut terminal = ratatui::try_init()?;
    execute!(io::stdout(), EnableMouseCapture)?;
    let interval = Duration::from_millis(500);
    let mut last_refresh = Instant::now();
    let mut redraw = true;
    while !app.quit {
        if let Some(response) = worker.receive()? {
            app.apply(response);
            redraw = true;
        }
        if !worker.busy() && (app.refresh_requested || last_refresh.elapsed() >= interval) {
            worker.request(app.request())?;
            app.refresh_requested = false;
            last_refresh = Instant::now();
        }
        if redraw {
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            redraw = false;
        }
        if event::poll(Duration::from_millis(40))? {
            match event::read()? {
                Event::Key(key) => app.key(key),
                Event::Mouse(mouse) => app.mouse(mouse),
                Event::Resize(_, _) => {}
                _ => continue,
            }
            redraw = true;
        }
    }
    Ok(())
}
