use super::*;

pub fn run(options: &Options, client: Client) -> Result<()> {
    let (mut app, received) = initialize(options, &client)?;
    let rendering = Rendering::from_environment();
    let mut terminal = ratatui::init();
    enable_capture();
    let result = run_loop(&mut app, &mut terminal, &received, rendering);
    app.stop_poller.store(true, Ordering::SeqCst);
    disable_capture();
    ratatui::restore();
    result
}

struct Rendering {
    no_color: bool,
    ascii: bool,
    truecolor: bool,
}

impl Rendering {
    fn from_environment() -> Self {
        Self {
            no_color: std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()),
            ascii: ascii_terminal(),
            truecolor: truecolor_terminal(),
        }
    }
}

fn initialize(options: &Options, client: &Client) -> Result<(App, mpsc::Receiver<Update>)> {
    let conversation = options.conversation(client)?;
    let (updates, received) = mpsc::channel();
    let trust_prompt = trust_prompt(options, client);
    let mut app = App::bootstrap(client.clone(), updates);
    app.project = options
        .project
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    app.conversation.clear();
    app.generation = 0;
    app.input = InputBuffer::with_text(options.message.clone().unwrap_or_default());
    app.notice = client.notice.clone();
    app.terminal_area = Rect::default();
    app.trust_prompt = trust_prompt;
    app.load_history(&options.state);
    app.register_project(workspace::ProjectConn::new(
        options.project.clone(),
        client.clone(),
    ));
    app.open(conversation);
    Ok((app, received))
}

fn trust_prompt(options: &Options, client: &Client) -> Option<String> {
    if options.trust {
        return None;
    }
    match client.call(&Request::TrustStatus) {
        Ok(value) if value["trusted"] == true => None,
        _ => Some(options.project.to_string_lossy().into_owned()),
    }
}

fn enable_capture() {
    let _ = crossterm::execute!(std::io::stdout(), EnableBracketedPaste, EnableMouseCapture);
}

fn disable_capture() {
    let _ = crossterm::execute!(
        std::io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture
    );
}

fn run_loop(
    app: &mut App,
    terminal: &mut ratatui::DefaultTerminal,
    received: &mpsc::Receiver<Update>,
    rendering: Rendering,
) -> Result<()> {
    loop {
        apply_pending(app, received);
        advance_frame(app);
        draw_frame(app, terminal, &rendering)?;
        copy_clipboard(app);
        if !handle_events(app)? {
            return Ok(());
        }
    }
}

fn apply_pending(app: &mut App, received: &mpsc::Receiver<Update>) {
    while let Ok(update) = received.try_recv() {
        app.apply_update(update);
    }
}

fn advance_frame(app: &mut App) {
    app.tick = app.tick.wrapping_add(1);
    if let Some(plan) = &mut app.plan_view {
        plan.tick = app.tick;
    }
}

fn draw_frame(
    app: &mut App,
    terminal: &mut ratatui::DefaultTerminal,
    rendering: &Rendering,
) -> Result<()> {
    terminal.draw(|frame| {
        if !rendering.no_color {
            let area = frame.area();
            frame.render_widget(
                ratatui::widgets::Block::default()
                    .style(Style::default().bg(theme::ayu::BG).fg(theme::ayu::FG)),
                area,
            );
        }
        app.draw(frame);
        if rendering.no_color {
            without_color(frame.buffer_mut());
        }
        if !rendering.no_color && !rendering.truecolor {
            to_indexed_colors(frame.buffer_mut());
        }
        if rendering.ascii {
            in_ascii(frame.buffer_mut());
        }
    })?;
    Ok(())
}

fn copy_clipboard(app: &mut App) {
    let Some(text) = app.pending_clipboard.take() else {
        return;
    };
    match clipboard::copy_text(&mut std::io::stdout().lock(), &text) {
        Ok(clipboard::CopyMethod::Osc52) => {}
        Ok(clipboard::CopyMethod::Fallback { osc52_error }) => {
            app.notice = Some(format!("OSC 52 failed: {osc52_error}; copied with pbcopy"));
        }
        Err(error) => app.notice = Some(error.to_string()),
    }
}

fn handle_events(app: &mut App) -> Result<bool> {
    if !event::poll(Duration::from_millis(100))? {
        return Ok(true);
    }
    let event = event::read()?;
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release || app.native_focused() => {
            app.notice = None;
            Ok(app.key(key))
        }
        Event::Mouse(mouse) => Ok(app.mouse(mouse)),
        Event::Paste(text) => {
            app.paste(&text);
            Ok(true)
        }
        _ => Ok(true),
    }
}
