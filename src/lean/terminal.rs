//! Persistent, viewport-sized terminal state owned by the daemon.
use super::{Core, native};
use crate::{Result, require};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use native::invalid;
use portable_pty::{CommandBuilder, MasterPty, NativePtySystem, PtySize, PtySystem};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use wezterm_term::{Terminal, TerminalConfiguration, TerminalSize};

mod controls;
mod emulator;
mod session;
use controls::Filter;
use emulator::GraphemeStream;
pub(crate) use session::handles::{quiet_for, tail};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    Key { event: KeyEvent },
    Paste { text: String },
    Mouse { event: MouseEvent },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub x: u16,
    pub y: u16,
    pub text: String,
    pub width: u16,
    pub foreground: Option<[u8; 3]>,
    pub background: Option<[u8; 3]>,
    pub modifiers: u16,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub handle: String,
    pub revision: u64,
    pub columns: u16,
    pub rows: u16,
    pub cursor: Option<(u16, u16)>,
    pub cells: Vec<Cell>,
    pub error: Option<String>,
}

#[derive(Debug)]
struct Config;
impl TerminalConfiguration for Config {
    fn scrollback_size(&self) -> usize {
        0
    }
    fn color_palette(&self) -> wezterm_term::color::ColorPalette {
        Default::default()
    }
    fn unicode_version(&self) -> wezterm_term::UnicodeVersion {
        wezterm_term::UnicodeVersion::new(14)
    }
}
pub(crate) struct State {
    term: Terminal,
    revision: u64,
    error: Option<String>,
    /// When the CLI last printed anything.
    output: std::time::Instant,
}
struct Handle {
    sequence: u64,
    seen: std::time::Instant,
}
pub struct Session {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    state: Arc<Mutex<State>>,
    handles: HashMap<String, Handle>,
    registration: Option<(std::sync::Weak<Core>, u32)>,
    /// Tags the CLI and everything it starts; see `process_tree`.
    marker: String,
}

#[derive(Clone)]
struct Writer(std::sync::mpsc::SyncSender<Vec<u8>>);
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            // WezTerm calls this on its own writer thread. Apply backpressure
            // there: WouldBlock would terminate that thread and lose later
            // keystrokes even after the PTY became writable again.
            .send(bytes.to_vec())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::BrokenPipe, e))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn size(columns: u16, rows: u16) -> Result<TerminalSize> {
    require(
        columns > 0 && rows > 0 && columns <= 500 && rows <= 200,
        "terminal dimensions must be within 1x1 and 500x200",
    )?;
    Ok(TerminalSize {
        rows: rows.into(),
        cols: columns.into(),
        ..Default::default()
    })
}
fn pty_size(columns: u16, rows: u16) -> PtySize {
    PtySize {
        cols: columns,
        rows,
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[cfg(test)]
mod tests {
    /// What a CLI that asked for bracketed paste receives for `text`.
    fn typed_into_a_cli(text: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let received = dir.path().join("received");
        let script = format!(
            "import os,sys,tty\ntty.setraw(0)\nsys.stdout.write('\\x1b[?2004h');sys.stdout.flush()\n\
             buf=b''\nwhile True:\n d=os.read(0,65536)\n if not d: break\n buf+=d\n \
             if b'\\x1b[201~' in buf and b'\\r' in buf[buf.index(b'\\x1b[201~'):]:\n  \
             open({received:?},'wb').write(buf);break\n",
            received = received.to_string_lossy()
        );
        let config =
            crate::config::Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let mut command = CommandBuilder::new("python3");
        command.args(["-c", &script]);
        let mut terminal =
            Session::spawn(command, core.weak.get().unwrap(), "python3", 120, 40).unwrap();
        let screen = terminal.shared();
        while quiet_for(&screen) < std::time::Duration::from_millis(600) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Session::submit(&screen, text).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !received.exists() {
            assert!(std::time::Instant::now() < deadline, "nothing received");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        let bytes = std::fs::read(&received).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        let start = text.find("\x1b[200~").unwrap() + 6;
        let end = text.find("\x1b[201~").unwrap();
        let _ = terminal.exited();
        text[start..end].to_owned()
    }

    /// Whether a command the CLI started in its own group outlives the
    /// session; `cli_waits` keeps the CLI running, otherwise it exits first.
    fn command_outlives_session(cli_waits: bool) -> bool {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        let script = format!(
            "import subprocess,sys,time\n\
             p=subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)'],start_new_session=True,stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n\
             open({pid:?},'w').write(str(p.pid))\ntime.sleep(60 if {cli_waits} else 0)\n",
            pid = pid_file.to_string_lossy(),
            cli_waits = if cli_waits { "True" } else { "False" },
        );
        let config =
            crate::config::Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let mut command = CommandBuilder::new("python3");
        command.args(["-c", &script]);
        let terminal =
            Session::spawn(command, core.weak.get().unwrap(), "python3", 120, 40).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let pid = loop {
            if let Some(pid) = std::fs::read_to_string(&pid_file)
                .ok()
                .and_then(|text| text.parse::<i32>().ok())
            {
                break pid;
            }
            assert!(std::time::Instant::now() < deadline, "no command started");
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        std::thread::sleep(std::time::Duration::from_millis(300));
        drop(terminal);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // SAFETY: signal 0 only checks that the process exists.
        while unsafe { libc::kill(pid, 0) } == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        if alive {
            // SAFETY: the test's own grandchild.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
        alive
    }

    #[test]
    fn a_command_in_its_own_group_dies_with_its_session() {
        assert!(!command_outlives_session(true));
    }

    #[test]
    fn a_command_its_exited_cli_abandoned_dies_with_the_session() {
        assert!(!command_outlives_session(false));
    }

    #[test]
    fn a_typed_prompt_arrives_exactly_as_written() {
        let line = "{\"k\": \"zażółć gęślą jaźń 🙂 \\u00e9 tab\\there\"}";
        let big = format!(
            "first\n{}\n\n  indented\ttab \"quoted\" 'single' `tick`\ntrailing\n",
            line.repeat(2000)
        );
        let received = typed_into_a_cli(&big);
        assert_eq!(received.replace('\r', "\n"), big);
    }

    use super::*;
    #[test]
    fn split_palette_and_cursor_queries_receive_terminal_responses() {
        let (sender, received) = std::sync::mpsc::sync_channel(8);
        let mut term = Terminal::new(
            size(40, 4).unwrap(),
            Arc::new(Config),
            "test",
            "test",
            Box::new(Writer(sender)),
        );
        let mut filter = Filter::default();
        let mut graphemes = GraphemeStream::default();
        for &byte in b"\x1b]11;?\x1b\\\x1b[6n" {
            graphemes.feed(&mut term, &filter.feed(&[byte])).unwrap();
        }
        let mut output = String::new();
        for _ in 0..8 {
            output.push_str(
                &String::from_utf8(
                    received
                        .recv_timeout(std::time::Duration::from_secs(1))
                        .unwrap(),
                )
                .unwrap(),
            );
            if output.contains("11;rgb:") && output.contains("1;1R") {
                break;
            }
        }
        assert!(output.contains("11;rgb:"), "{output:?}");
        assert!(output.contains("1;1R"), "{output:?}");
        assert!(text(&term).trim().is_empty());
        assert!(filter.feed(b"\x1b]52;c;?\x07").is_empty());
    }
    #[test]
    fn writer_backpressure_keeps_the_complete_paste_and_later_input() {
        let (sender, received) = std::sync::mpsc::sync_channel(1);
        sender.send(b"occupied".to_vec()).unwrap();
        let mut writer = Writer(sender);
        let (ready, started) = std::sync::mpsc::sync_channel(1);
        let (finished, done) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            ready.send(()).unwrap();
            let paste = "\x1b[200~draft 👩‍💻\x1b[201~";
            finished
                .send(writer.write_all(paste.as_bytes()).is_ok())
                .unwrap();
            writer.write_all(b"next").unwrap();
        });
        started.recv().unwrap();
        assert!(
            done.recv_timeout(std::time::Duration::from_millis(50))
                .is_err()
        );
        assert_eq!(received.recv().unwrap(), b"occupied");
        assert!(
            done.recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
        );
        assert_eq!(
            received.recv().unwrap(),
            "\x1b[200~draft 👩‍💻\x1b[201~".as_bytes()
        );
        assert_eq!(received.recv().unwrap(), b"next");
        worker.join().unwrap();
    }
    fn emulator(columns: usize, rows: usize) -> Terminal {
        Terminal::new(
            TerminalSize {
                cols: columns,
                rows,
                ..Default::default()
            },
            Arc::new(Config),
            "antislop",
            "test",
            Box::new(std::io::sink()),
        )
    }
    fn text(term: &Terminal) -> String {
        let top = term.screen().scrollback_or_visible_row(0);
        term.screen()
            .lines_in_phys_range(top..top + term.get_size().rows)
            .iter()
            .map(|line| {
                line.visible_cells()
                    .map(|cell| cell.str().to_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn ansi_and_utf8_sequences_survive_split_reads_and_alternate_screen() {
        let mut term = emulator(40, 4);
        let mut filter = Filter::default();
        for bytes in [
            b"\x1b[3".as_slice(),
            b"1mred\x1b[0",
            b"m \xf0\x9f",
            b"\x90\xa2",
        ] {
            term.advance_bytes(filter.feed(bytes));
        }
        assert!(text(&term).contains("red 🐢"));
        term.advance_bytes(b"\x1b[?1049hALT");
        assert!(text(&term).contains("ALT"));
        term.advance_bytes(b"\x1b[?1049l");
        assert!(text(&term).contains("red 🐢"));
    }
    #[test]
    fn unicode_clusters_and_screen_storage_stay_bounded() {
        let mut term = emulator(40, 3);
        term.advance_bytes("👩‍💻 e\u{301} 漢".as_bytes());
        assert!(text(&term).contains("👩‍💻"));
        assert!(text(&term).contains("e\u{301}"));
        for _ in 0..1000 {
            term.advance_bytes(b"line\r\n");
        }
        assert!(term.screen().scrollback_rows() <= 3);
        assert!(term.screen().lines_in_phys_range(0..10).len() <= 3);
    }
    #[test]
    fn zwj_and_combining_clusters_survive_byte_by_byte_output() {
        let mut term = emulator(40, 3);
        let mut stream = GraphemeStream::default();
        for byte in "👩‍💻 e\u{301}".as_bytes() {
            stream.feed(&mut term, &[*byte]).unwrap();
            let _ = text(&term); // Publishing a screen cannot split the cluster.
        }
        assert!(text(&term).contains("👩‍💻"));
        assert!(text(&term).contains("e\u{301}"));
    }
    #[test]
    fn a_cluster_can_continue_after_a_resize_reflows_its_cell() {
        let mut term = emulator(40, 4);
        let mut stream = GraphemeStream::default();
        stream.feed(&mut term, "12345👩".as_bytes()).unwrap();
        term.resize(TerminalSize {
            cols: 4,
            rows: 4,
            ..Default::default()
        });
        stream.feed(&mut term, "‍💻".as_bytes()).unwrap();
        let text = text(&term);
        assert!(text.contains("👩‍💻"));
        assert!(!text.replace("👩‍💻", "").contains("👩"));
    }
    #[test]
    fn string_controls_and_oversized_sequences_cannot_allocate_image_buffers() {
        let mut filter = Filter::default();
        let mut term = emulator(20, 2);
        term.advance_bytes(filter.feed(b"before\x1b]1337;File=huge"));
        term.advance_bytes(filter.feed(&vec![b'x'; 1024 * 1024]));
        term.advance_bytes(filter.feed(b"\x1b\\after"));
        assert!(text(&term).contains("beforeafter"));
        assert!(filter.sequence.len() <= 1024);
        let mut huge = vec![27, b'['];
        huge.extend_from_slice(&vec![b'1'; 4096]);
        huge.extend_from_slice(b"mOK");
        term.advance_bytes(filter.feed(&huge));
        assert!(text(&term).contains("OK"));
    }
    #[test]
    fn terminal_dimensions_are_bounded_before_allocation() {
        assert!(size(0, 1).is_err());
        assert!(size(501, 20).is_err());
        assert!(size(80, 201).is_err());
        assert!(size(12, 5).is_ok());
    }
}
