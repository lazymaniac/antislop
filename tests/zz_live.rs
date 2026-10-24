mod common;
use antislop::{config::Config, lean::Core};
use std::time::{Duration, Instant};
#[test]
#[ignore]
fn live() {
    let project = std::path::PathBuf::from(std::env::var("LIVE_PROJECT").unwrap());
    let text = std::fs::read_to_string(std::env::var("LIVE_CONFIG").unwrap()).unwrap();
    common::trust(&project);
    let state = project.join(".state-test");
    let _ = std::fs::remove_dir_all(&state);
    let core = Core::open(&state, Config::from_toml(&text).unwrap()).unwrap();
    let conversation = core.start(&project).unwrap();
    core.send(&conversation.id, "Reply with exactly one word: pong").unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    while !core.is_idle(&conversation.id) && Instant::now() < deadline { std::thread::sleep(Duration::from_millis(200)); }
    for t in core.store.sessions(&conversation.id).unwrap() { eprintln!("TURN {} {} {:?} session={:?}", t.role, t.status, t.error, t.session_id); }
    for m in core.store.messages_by(&conversation.id, "orchestrator", 0).unwrap() { eprintln!("MSG {}", m.text); }
    let screen = core.terminal_open(&conversation.id, "orchestrator", 100, 30, false).unwrap();
    let mut rows = vec![String::new(); screen.rows as usize];
    for c in &screen.cells { rows[c.y as usize].push_str(&c.text); }
    for r in rows { if !r.trim().is_empty() { eprintln!("SCREEN| {}", r.trim_end()); } }
}
