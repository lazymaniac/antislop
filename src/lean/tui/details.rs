//! Full-screen details of one agent or conversation: what it was asked,
//! its session history, messages, deliverables, obligations, and comments.

use super::super::tasks::{TaskSpec, obligations, status};
use super::view::{View, agent_name, dim, duration, polish, wrap};
use crate::lean::store::SessionInfo;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use serde_json::Value;
use std::collections::HashMap;

pub const TABS: [&str; 6] = [
    "Przegląd",
    "Sesja",
    "Wiadomości",
    "Wynik",
    "Kontrole",
    "Komentarze",
];
pub const RESULT: usize = 3;
pub const CHECKS: usize = 4;
pub const COMMENTS: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct DetailRecord {
    pub lines: Vec<(Style, String)>,
    pub search_text: String,
    pub raw_value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    pub key: String,
    pub tab: usize,
    /// Lines scrolled down from the top.
    pub scroll: usize,
    /// The task as the daemon stores it, once fetched.
    pub task: Option<Value>,
    /// The selected comment on the Komentarze tab.
    pub comment: usize,
    /// Scroll memory per (agent_key, tab_index)
    pub scroll_memory: HashMap<(String, usize), usize>,
    /// Query memory per (agent_key, tab_index)
    pub query_memory: HashMap<(String, usize), String>,
    /// Active search/filter query on current tab
    pub query: String,
    /// Whether user is typing a query in search mode
    pub searching: bool,
    /// Widok czytelny (false) vs raw JSON (true)
    pub raw: bool,
    /// Retained sessions history for Sesja tab
    pub sessions: Vec<SessionInfo>,
    /// Current page of sessions (for pagination)
    pub session_page: usize,
    /// Explicit flag indicating missing/pruned historical data
    pub history_unavailable: bool,
}

impl Details {
    pub fn new(key: String) -> Self {
        Details {
            key,
            tab: 0,
            scroll: 0,
            task: None,
            comment: 0,
            scroll_memory: HashMap::new(),
            query_memory: HashMap::new(),
            query: String::new(),
            searching: false,
            raw: false,
            sessions: Vec::new(),
            session_page: 0,
            history_unavailable: false,
        }
    }

    pub fn switch_tab(&mut self, next: usize) {
        let next_tab = next % TABS.len();
        self.scroll_memory
            .insert((self.key.clone(), self.tab), self.scroll);
        self.query_memory
            .insert((self.key.clone(), self.tab), self.query.clone());
        self.tab = next_tab;
        self.scroll = *self
            .scroll_memory
            .get(&(self.key.clone(), self.tab))
            .unwrap_or(&0);
        self.query = self
            .query_memory
            .get(&(self.key.clone(), self.tab))
            .cloned()
            .unwrap_or_default();
        self.searching = false;
    }

    pub fn switch_key(&mut self, new_key: String) {
        self.scroll_memory
            .insert((self.key.clone(), self.tab), self.scroll);
        self.query_memory
            .insert((self.key.clone(), self.tab), self.query.clone());
        self.key = new_key;
        self.scroll = *self
            .scroll_memory
            .get(&(self.key.clone(), self.tab))
            .unwrap_or(&0);
        self.query = self
            .query_memory
            .get(&(self.key.clone(), self.tab))
            .cloned()
            .unwrap_or_default();
        self.searching = false;
    }

    pub fn save_current_tab(&mut self) {
        self.scroll_memory
            .insert((self.key.clone(), self.tab), self.scroll);
        self.query_memory
            .insert((self.key.clone(), self.tab), self.query.clone());
    }

    pub fn restore_current_tab(&mut self) {
        self.scroll = *self
            .scroll_memory
            .get(&(self.key.clone(), self.tab))
            .unwrap_or(&0);
        self.query = self
            .query_memory
            .get(&(self.key.clone(), self.tab))
            .cloned()
            .unwrap_or_default();
    }

    pub fn next_session_page(&mut self) {
        self.session_page = self.session_page.saturating_add(1);
        self.scroll = 0;
    }

    pub fn prev_session_page(&mut self) {
        self.session_page = self.session_page.saturating_sub(1);
        self.scroll = 0;
    }

    /// The id of the selected comment, on the Komentarze tab.
    pub fn selected_comment(&self, view: &View) -> Option<u64> {
        (self.tab == COMMENTS)
            .then(|| view.comments_on(&self.key).get(self.comment).map(|c| c.id))
            .flatten()
    }

    /// The task id of a worker or verifier.
    pub fn task_id(&self) -> Option<&str> {
        self.key.split_once(':').map(|(_, id)| id)
    }

    pub fn lines(&mut self, view: &View) -> Vec<(Style, String)> {
        let records = self.records(view);
        let mut out = vec![];
        let query_lower = self.query.trim().to_lowercase();
        if !self.query.is_empty() {
            out.push((
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                format!(
                    "🔍 Filtr rekordów: \"{}\"  (v: przełącz widok raw/czytelny · Esc: wyczyść)",
                    self.query
                ),
            ));
            out.push((Style::default(), String::new()));
        }
        let mut matched_records = 0;
        for rec in records {
            let is_match =
                query_lower.is_empty() || rec.search_text.to_lowercase().contains(&query_lower);
            if is_match {
                matched_records += 1;
                if self.raw {
                    if let Some(val) = &rec.raw_value {
                        let pretty = serde_json::to_string_pretty(val).unwrap_or_default();
                        for line in pretty.lines() {
                            out.push((Style::default().fg(Color::Green), line.to_owned()));
                        }
                    } else {
                        for (style, line) in &rec.lines {
                            out.push((*style, format!("[RAW] {line}")));
                        }
                    }
                    out.push((Style::default(), String::new()));
                } else {
                    out.extend(rec.lines);
                }
            }
        }
        if !query_lower.is_empty() && matched_records == 0 {
            out.push((
                dim(),
                format!("Brak rekordów pasujących do zapytania \"{}\".", self.query),
            ));
        }
        out
    }

    fn records(&mut self, view: &View) -> Vec<DetailRecord> {
        let plain = |text: String| (Style::default(), text);
        let label = |text: &str| {
            (
                Style::default().add_modifier(Modifier::BOLD),
                text.to_owned(),
            )
        };
        let is_conv = self.key == "conversation" || self.key == "rozmowa";
        let agent = view.agents.get(&self.key);
        let task = self
            .task_id()
            .and_then(|id| view.tasks.iter().find(|task| task.id == id));
        let verifier = self.key.starts_with("verifier:");
        let mut records = vec![];

        match self.tab {
            0 => {
                if is_conv {
                    let mut conv_lines = vec![];
                    conv_lines.push(label("Szczegóły rozmowy"));
                    conv_lines.push(plain(format!(
                        "Projekt: {}",
                        view.plan.as_deref().unwrap_or("bieżący")
                    )));
                    conv_lines.push(plain(format!(
                        "Tryb: {}",
                        match view.autonomous {
                            Some(true) => "autonomiczny",
                            Some(false) => "nadzorowany",
                            None => "nieokreślony",
                        }
                    )));
                    conv_lines.push(plain(format!(
                        "Stan: {}",
                        if view.paused {
                            "wstrzymano"
                        } else if view.idle {
                            "bezczynny"
                        } else {
                            "aktywny"
                        }
                    )));
                    if let Some(p) = &view.plan {
                        conv_lines.push(plain(format!("Aktywny plan: {p}")));
                    }
                    let total_tasks = view.tasks.len();
                    let done_tasks = view
                        .tasks
                        .iter()
                        .filter(|t| status::finished(&t.status))
                        .count();
                    conv_lines.push(plain(format!(
                        "Zadania: ogółem {total_tasks}, wykonano {done_tasks}"
                    )));
                    let _missing_reports = view
                        .tasks
                        .iter()
                        .filter(|t| status::finished(&t.status) && t.reason.is_none())
                        .count();
                    conv_lines.push(plain(format!(
                        "Raporty zadań: {done_tasks}/{total_tasks} gotowych (jawne braki: {} w toku)",
                        total_tasks.saturating_sub(done_tasks)
                    )));
                    let text = conv_lines
                        .iter()
                        .map(|(_, t)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    records.push(DetailRecord {
                        lines: conv_lines,
                        search_text: text,
                        raw_value: Some(serde_json::json!({
                            "plan": view.plan,
                            "tasks_total": total_tasks,
                            "tasks_done": done_tasks,
                            "autonomous": view.autonomous,
                            "paused": view.paused,
                            "idle": view.idle,
                        })),
                    });
                } else {
                    if let Some(agent) = agent.filter(|agent| !agent.provider.is_empty()) {
                        let mut cli = format!("CLI: {}", agent.provider);
                        if let Some(model) = &agent.model {
                            cli.push_str(&format!(" · model {model}"));
                        }
                        if let Some(effort) = &agent.effort {
                            cli.push_str(&format!(" · effort {effort}"));
                        }
                        let mut cli_lines = vec![plain(cli)];
                        if agent.turns > 0 {
                            cli_lines.push(plain(format!("Tokeny: {}", agent.tokens())));
                        }
                        if let Some(last) = agent.last_ms {
                            cli_lines.push(plain(format!(
                                "Czas w turach: {} (ostatnia {})",
                                duration(agent.busy_ms),
                                duration(last)
                            )));
                        }
                        let text = cli_lines
                            .iter()
                            .map(|(_, t)| t.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        records.push(DetailRecord {
                            lines: cli_lines,
                            search_text: text,
                            raw_value: Some(serde_json::json!({
                                "provider": agent.provider,
                                "model": agent.model,
                                "effort": agent.effort,
                                "turns": agent.turns,
                                "tokens_in": agent.tokens_in,
                                "tokens_out": agent.tokens_out,
                                "busy_ms": agent.busy_ms,
                            })),
                        });
                    }
                    if let Some(task) = task {
                        let mut task_lines = vec![];
                        task_lines.push(plain(format!(
                            "Status zadania: {}{}",
                            polish(&task.status),
                            if task.round > 0 {
                                format!(" (runda {})", task.round)
                            } else {
                                String::new()
                            }
                        )));
                        if let Some(reason) = &task.reason {
                            task_lines.push(plain(format!("Dlaczego: {reason}")));
                        }
                        task_lines.push(plain(format!("Zadanie {}: {}", task.id, task.title)));
                        task_lines.push(plain(format!(
                            "Rodzaj: {}{}",
                            task.kind,
                            if task.writes {
                                " · zmienia pliki"
                            } else {
                                " · tylko odczyt"
                            }
                        )));
                        if !task.depends_on.is_empty() {
                            task_lines.push(plain(format!(
                                "Zależy od: {}",
                                task.depends_on.join(", ")
                            )));
                        }
                        if !task.scope.is_empty() {
                            task_lines.push(plain(format!("Zakres: {}", task.scope.join(", "))));
                        }
                        task_lines.push(plain(String::new()));
                        task_lines.push(label("Cel"));
                        task_lines.push(plain(task.goal.clone()));
                        if verifier || !task.focus.is_empty() {
                            task_lines.push(plain(String::new()));
                            task_lines.push(label("Weryfikacja"));
                            task_lines.push(plain(task.focus.clone()));
                        }
                        if verifier && !task.risks.is_empty() {
                            task_lines.push(plain(String::new()));
                            task_lines.push(label("Ryzyka"));
                            for risk in &task.risks {
                                task_lines.push(plain(format!("- {risk}")));
                            }
                        }
                        let text = task_lines
                            .iter()
                            .map(|(_, t)| t.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        records.push(DetailRecord {
                            lines: task_lines,
                            search_text: text,
                            raw_value: self.task.clone(),
                        });
                    }
                    if let Some(review) = view.reviews.get(&self.key) {
                        let mut rev_lines = vec![plain(String::new()), label("Ostatni wynik")];
                        rev_lines.extend(review.iter().cloned().map(plain));
                        let text = rev_lines
                            .iter()
                            .map(|(_, t)| t.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        records.push(DetailRecord {
                            lines: rev_lines,
                            search_text: text,
                            raw_value: Some(serde_json::json!({ "review": review })),
                        });
                    }
                    if let Some(agent) = agent {
                        let mut act_lines = vec![plain(String::new()), label("Teraz")];
                        act_lines.push(plain(if agent.running {
                            format!("pracuje: {}", agent.activity)
                        } else {
                            format!("bezczynny · {}", agent.activity)
                        }));
                        let text = act_lines
                            .iter()
                            .map(|(_, t)| t.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        records.push(DetailRecord {
                            lines: act_lines,
                            search_text: text,
                            raw_value: Some(serde_json::json!({
                                "running": agent.running,
                                "activity": agent.activity,
                            })),
                        });
                    } else if task.is_none() {
                        records.push(DetailRecord {
                            lines: vec![plain("Ten agent jeszcze nie pracował.".into())],
                            search_text: "Ten agent jeszcze nie pracował.".into(),
                            raw_value: None,
                        });
                    }
                }
            }
            1 => {
                if self.history_unavailable {
                    records.push(DetailRecord {
                        lines: vec![(
                            Style::default().fg(Color::Yellow),
                            "⚠ [Część danych historycznych sesji jest niedostępna w archiwum bazy]"
                                .into(),
                        )],
                        search_text: "dane historyczne niedostępne".into(),
                        raw_value: None,
                    });
                }
                if !self.sessions.is_empty() {
                    let page_size = 5;
                    let total_pages = self.sessions.len().div_ceil(page_size);
                    self.session_page = self.session_page.min(total_pages.saturating_sub(1));
                    let start = self.session_page * page_size;
                    let end = (start + page_size).min(self.sessions.len());
                    records.push(DetailRecord {
                        lines: vec![(
                            Style::default().fg(Color::Cyan),
                            format!(
                                "── Zachowane sesje w bazie (strona {}/{}, pozycje {}-{} z {}) ──",
                                self.session_page + 1,
                                total_pages,
                                start + 1,
                                end,
                                self.sessions.len()
                            ),
                        )],
                        search_text: "sesje zachowane strona".into(),
                        raw_value: None,
                    });
                    for sess in &self.sessions[start..end] {
                        let mut sess_lines = vec![];
                        sess_lines.push((
                            Style::default().add_modifier(Modifier::BOLD),
                            format!(
                                "• Tura {} · Rola: {} ({}) · Stan: {}",
                                sess.turn_id, sess.role, sess.provider, sess.status
                            ),
                        ));
                        if let Some(sid) = &sess.session_id {
                            sess_lines.push(plain(format!("  Sesja CLI: {sid}")));
                        } else {
                            sess_lines.push((
                                dim(),
                                "  Sesja CLI: [brak zapisanego session_id w historii]".into(),
                            ));
                        }
                        let dur_str = if let Some(fin) = sess.finished_ms {
                            duration(fin.saturating_sub(sess.started_ms).max(0) as u64)
                        } else {
                            "w toku".into()
                        };
                        sess_lines.push(plain(format!(
                            "  Czas trwania: {dur_str} (start: {} ms)",
                            sess.started_ms
                        )));
                        if let Some(pid) = sess.pid {
                            sess_lines.push(plain(format!("  Proces PID: {pid}")));
                        }
                        sess_lines.push(plain(format!("  Do wiadomości: #{}", sess.upto_message)));
                        if let Some(err) = &sess.error {
                            sess_lines
                                .push((Style::default().fg(Color::Red), format!("  Błąd: {err}")));
                        }
                        let text = sess_lines
                            .iter()
                            .map(|(_, t)| t.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        records.push(DetailRecord {
                            lines: sess_lines,
                            search_text: text,
                            raw_value: Some(serde_json::to_value(sess).unwrap_or_default()),
                        });
                    }
                } else {
                    match agent {
                        Some(agent) if !agent.session.is_empty() => {
                            for line in &agent.session {
                                records.push(DetailRecord {
                                    lines: vec![plain(line.clone())],
                                    search_text: line.clone(),
                                    raw_value: Some(serde_json::json!({ "event": line })),
                                });
                            }
                        }
                        _ => {
                            records.push(DetailRecord {
                                lines: vec![(
                                    dim(),
                                    "Brak zdarzeń sesji dla tego agenta. [dane historyczne niedostępne lub agent jeszcze nie pracował]. Zdarzenia narzędzi pojawią się w trakcie pracy.".into(),
                                )],
                                search_text: "brak zdarzeń sesji dane historyczne niedostępne".into(),
                                raw_value: None,
                            });
                        }
                    }
                }
            }
            2 => {
                let mailbox_items = view.mailbox.get(&self.key);
                match mailbox_items {
                    Some(mail) if !mail.is_empty() => {
                        for item in mail {
                            records.push(DetailRecord {
                                lines: vec![plain(item.clone())],
                                search_text: item.clone(),
                                raw_value: Some(serde_json::json!({ "mail": item })),
                            });
                        }
                    }
                    _ => {
                        let id = self.task_id().unwrap_or(&self.key);
                        records.push(DetailRecord {
                            lines: vec![(
                                dim(),
                                format!("Brak wiadomości. [brak zarejestrowanych wiadomości w historii dla tego agenta]. Użyj '@{id} wiadomość' w czacie, aby napisać do tego agenta."),
                            )],
                            search_text: "brak wiadomości".into(),
                            raw_value: None,
                        });
                    }
                }
            }
            COMMENTS => {
                let comments = view.comments_on(&self.key);
                if comments.is_empty() {
                    records.push(DetailRecord {
                        lines: vec![(
                            dim(),
                            "Brak komentarzy. [brak komentarzy w historii dla tego elementu]. Naciśnij 'c', aby dodać komentarz do tego elementu.".into(),
                        )],
                        search_text: "brak komentarzy".into(),
                        raw_value: None,
                    });
                } else {
                    self.comment = self.comment.min(comments.len().saturating_sub(1));
                    for (index, comment) in comments.iter().enumerate() {
                        let mut style = if comment.status == "open" {
                            Style::default()
                        } else {
                            dim()
                        };
                        if index == self.comment {
                            style = style.add_modifier(Modifier::REVERSED);
                        }
                        let state = match comment.status.as_str() {
                            "open" => "otwarty",
                            "resolved" => "rozwiązany",
                            other => other,
                        };
                        let stale = if view.stale(comment) {
                            " · dotyczy poprzedniego planu"
                        } else {
                            ""
                        };
                        let header_str = format!(
                            "#{} {} · {state}{stale}",
                            comment.id,
                            super::view::target_name(&comment.target)
                        );
                        let text_str = format!("  {}", comment.text);
                        let search_text = format!("{header_str} {text_str}");
                        records.push(DetailRecord {
                            lines: vec![(style, header_str), (Style::default(), text_str)],
                            search_text,
                            raw_value: Some(serde_json::to_value(comment).unwrap_or_default()),
                        });
                    }
                }
            }
            CHECKS => match (&self.task, task) {
                (_, None) => records.push(DetailRecord {
                    lines: vec![(
                        dim(),
                        "Kontrole mają zadania: ryzyka zadania i lista kontrolna jego rodzaju, rozliczane przez weryfikatora. [brak danych kontroli w historii]".into(),
                    )],
                    search_text: "kontrole mają zadania".into(),
                    raw_value: None,
                }),
                (None, Some(_)) => records.push(DetailRecord {
                    lines: vec![(dim(), "Pobieram…".into())],
                    search_text: "pobieram".into(),
                    raw_value: None,
                }),
                (Some(stored), Some(_)) => {
                    let check_lines = checks(stored);
                    for line in check_lines {
                        records.push(DetailRecord {
                            search_text: line.1.clone(),
                            lines: vec![line],
                            raw_value: Some(stored["verdict"]["coverage"].clone()),
                        });
                    }
                }
            },
            _ => match (&self.task, task) {
                (_, None) => records.push(DetailRecord {
                    lines: vec![(
                        dim(),
                        "Wynik mają workery i weryfikatorzy zadań; wyniki weryfikatora planu i audytora są w Przeglądzie. [brak danych historycznych wyniku]".into(),
                    )],
                    search_text: "wynik mają workery".into(),
                    raw_value: None,
                }),
                (None, Some(_)) => records.push(DetailRecord {
                    lines: vec![(dim(), "Pobieram…".into())],
                    search_text: "pobieram".into(),
                    raw_value: None,
                }),
                (Some(stored), Some(_)) if verifier => match &stored["verdict"] {
                    Value::Null => records.push(DetailRecord {
                        lines: vec![(
                            dim(),
                            "Weryfikator jeszcze nie ocenił zadania (werdykt pojawi się po weryfikacji). [brak danych historycznych werdyktu]".into(),
                        )],
                        search_text: "weryfikator jeszcze nie ocenił".into(),
                        raw_value: None,
                    }),
                    verdict => {
                        let mut lines = vec![];
                        lines.extend(
                            super::super::cli::review_lines("werdykt", verdict)
                                .into_iter()
                                .map(plain),
                        );
                        if let Some(findings) = verdict["findings"].as_array() {
                            if findings.is_empty() {
                                lines.push(plain("Ustalenia: brak zastrzeżeń (weryfikator zatwierdził zadanie bez uwag).".into()));
                            } else {
                                lines.push(label("Ustalenia weryfikatora"));
                                for f in findings {
                                    let sev = f["severity"].as_str().unwrap_or("uwaga");
                                    let issue = f["issue"].as_str().unwrap_or_default();
                                    let where_str = f["where"]
                                        .as_str()
                                        .map(|w| format!(" ({w})"))
                                        .unwrap_or_default();
                                    lines.push(plain(format!("• [{sev}] {issue}{where_str}")));
                                }
                            }
                        } else {
                            lines.push((
                                dim(),
                                "Ustalenia: [brak danych historycznych ustaleń]".into(),
                            ));
                        }
                        let text = lines
                            .iter()
                            .map(|(_, t)| t.as_str())
                            .collect::<Vec<_>>()
                            .join(" ");
                        records.push(DetailRecord {
                            lines,
                            search_text: text,
                            raw_value: Some(verdict.clone()),
                        });
                    }
                },
                (Some(stored), Some(_)) => {
                    let mut art_lines = vec![];
                    art_lines.push(label("Artefakty i zmiany"));
                    if let (Some(base), Some(result)) =
                        (stored["base"].as_str(), stored["result"].as_str())
                    {
                        art_lines.push(plain(format!(
                            "Zmiany git: {}..{}",
                            &base[..base.len().min(10)],
                            &result[..result.len().min(10)]
                        )));
                    } else {
                        art_lines.push((
                            dim(),
                            "Artefakty: brak zarejestrowanych zmian git [dane historyczne niedostępne]".into(),
                        ));
                    }
                    let art_text = art_lines
                        .iter()
                        .map(|(_, t)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    records.push(DetailRecord {
                        lines: art_lines,
                        search_text: art_text,
                        raw_value: Some(serde_json::json!({
                            "base": stored["base"],
                            "result": stored["result"],
                        })),
                    });

                    let mut rep_lines = vec![];
                    rep_lines.push(label("Raport"));
                    match stored["report"].as_str() {
                        Some(report) => {
                            rep_lines.push(plain(report.to_owned()));
                        }
                        None => {
                            rep_lines.push((
                                dim(),
                                "Worker jeszcze nie oddał raportu (trwa praca lub oczekiwanie na start) [brak danych historycznych raportu].".into(),
                            ));
                        }
                    }
                    let rep_text = rep_lines
                        .iter()
                        .map(|(_, t)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    records.push(DetailRecord {
                        lines: rep_lines,
                        search_text: rep_text,
                        raw_value: Some(serde_json::json!({
                            "report": stored["report"],
                        })),
                    });

                    let mut ev_lines = vec![label("Dowody i ustalenia")];
                    if let Some(verdict) = stored.get("verdict").filter(|v| !v.is_null()) {
                        if let Some(findings) = verdict["findings"].as_array() {
                            if findings.is_empty() {
                                ev_lines.push(plain("Ustalenia: brak zastrzeżeń.".into()));
                            } else {
                                for f in findings {
                                    let sev = f["severity"].as_str().unwrap_or("uwaga");
                                    let issue = f["issue"].as_str().unwrap_or_default();
                                    ev_lines.push(plain(format!("• [{sev}] {issue}")));
                                }
                            }
                        }
                    } else {
                        ev_lines.push((dim(), "Dowody: [brak załączonych dowodów w historii / oczekuje na weryfikację]".into()));
                    }
                    let ev_text = ev_lines
                        .iter()
                        .map(|(_, t)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                    records.push(DetailRecord {
                        lines: ev_lines,
                        search_text: ev_text,
                        raw_value: Some(stored["verdict"].clone()),
                    });
                }
            },
        }
        records
    }

    pub fn draw(&mut self, frame: &mut Frame, view: &View) {
        if self.tab >= TABS.len() {
            self.tab = 0;
        }
        let area = frame.area();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);

        let mut tabs_line = format_tabs(&self.key, self.tab, usize::from(header.width));
        if self.raw {
            tabs_line.spans.push(Span::styled(
                " [RAW] ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        if !self.query.is_empty() {
            tabs_line.spans.push(Span::styled(
                format!(" [🔍 \"{}\"]", self.query),
                Style::default().fg(Color::Cyan),
            ));
        }
        frame.render_widget(Paragraph::new(tabs_line), header);

        let block = Block::default().borders(Borders::TOP);
        let inner = block.inner(body);
        frame.render_widget(block, body);
        let width = usize::from(inner.width.saturating_sub(1));
        let lines: Vec<Line> = self
            .lines(view)
            .into_iter()
            .flat_map(|(style, text)| {
                wrap(&text, width)
                    .into_iter()
                    .map(move |line| Line::styled(line, style))
            })
            .collect();
        let height = usize::from(inner.height);
        self.scroll = self.scroll.min(lines.len().saturating_sub(height));
        let end = (self.scroll + height).min(lines.len());
        frame.render_widget(Paragraph::new(lines[self.scroll..end].to_vec()), inner);

        let hint = if self.searching {
            format!(
                "Szukaj w rekordach: {}_ (Enter: zatwierdź · Esc: anuluj)",
                self.query
            )
        } else {
            details_footer_hint(self.tab, usize::from(footer.width)).to_string()
        };
        frame.render_widget(Paragraph::new(Span::styled(hint, dim())), footer);
    }
}

pub fn details_footer_hint(tab: usize, width: usize) -> &'static str {
    if tab == COMMENTS {
        if width >= 95 {
            "←/→ zakładka · ↑/↓ wybór · / szukaj · v raw · c skomentuj · r rozwiąż · o otwórz · x usuń · d diff · ? pomoc · Esc powrót"
        } else if width >= 65 {
            "←/→ zakładki · ↑/↓ wybór · / szukaj · v raw · c dodaj · r/o/x status · d diff · Esc powrót"
        } else if width >= 45 {
            "←/→ tab · ↑/↓ wybór · c dodaj · r/o/x · Esc"
        } else {
            "c kom · Esc powrót"
        }
    } else {
        if width >= 90 {
            "←/→ zakładka · ↑/↓ PgUp/PgDn przewiń · / szukaj · v raw · c skomentuj · d diff · ? pomoc · Esc powrót do grafu"
        } else if width >= 65 {
            "←/→ zakładki · ↑/↓ przewiń · / szukaj · v raw · c skomentuj · d diff · Esc powrót"
        } else if width >= 45 {
            "←/→ tab · ↑/↓ przewiń · c kom · d diff · Esc"
        } else {
            "↑/↓ · Esc powrót"
        }
    }
}

pub fn format_tabs<'a>(agent_key: &str, active_tab: usize, max_width: usize) -> Line<'a> {
    let agent = format!(" {} ", agent_name(agent_key));
    let agent_w = agent.len();
    let full_tabs_len: usize = agent_w
        + TABS
            .iter()
            .enumerate()
            .map(|(i, t)| t.len() + if i == active_tab { 4 } else { 3 })
            .sum::<usize>();

    if full_tabs_len <= max_width {
        let mut spans = vec![Span::styled(
            agent,
            Style::default().add_modifier(Modifier::BOLD),
        )];
        for (index, name) in TABS.iter().enumerate() {
            spans.push(Span::raw(" "));
            spans.push(if index == active_tab {
                Span::styled(
                    format!("[{name}]"),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!(" {name} "), dim())
            });
        }
        return Line::from(spans);
    }

    // Narrow terminal: ensure active tab is visible with sliding window
    let mut spans = vec![Span::styled(
        agent,
        Style::default().add_modifier(Modifier::BOLD),
    )];

    let mut left = active_tab;
    let mut right = active_tab;
    let mut current_len = agent_w + TABS[active_tab].len() + 4;
    while left > 0 || right + 1 < TABS.len() {
        let mut expanded = false;
        if right + 1 < TABS.len() {
            let next_len = TABS[right + 1].len() + 3;
            if current_len + next_len + 2 <= max_width {
                right += 1;
                current_len += next_len;
                expanded = true;
            }
        }
        if left > 0 {
            let prev_len = TABS[left - 1].len() + 3;
            if current_len + prev_len + 2 <= max_width {
                left -= 1;
                current_len += prev_len;
                expanded = true;
            }
        }
        if !expanded {
            break;
        }
    }

    if left > 0 {
        spans.push(Span::styled("◀ ", dim()));
    }
    for (index, &name) in TABS.iter().enumerate().take(right + 1).skip(left) {
        spans.push(if index == active_tab {
            Span::styled(
                format!("[{name}] "),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!("{name} "), dim())
        });
    }
    if right + 1 < TABS.len() {
        spans.push(Span::styled("▶", dim()));
    }

    Line::from(spans)
}

/// The verifier's obligations (the task's risks, then its kind's checklist)
/// and how the latest verdict accounted for each.
fn checks(stored: &Value) -> Vec<(Style, String)> {
    let Ok(spec) = serde_json::from_value::<TaskSpec>(stored["spec"].clone()) else {
        return vec![(dim(), "Nie umiem odczytać zadania.".into())];
    };
    let coverage = &stored["verdict"]["coverage"];
    let mut lines = vec![];
    if stored["verdict"].is_null() {
        lines.push((
            dim(),
            "Weryfikator jeszcze nie ocenił. Ryzyka i lista kontrolna zostaną rozliczone po zakończeniu zadania [brak danych historycznych].".into(),
        ));
        lines.push((Style::default(), String::new()));
    }
    for (id, text) in obligations(&spec) {
        let entry = &coverage[id.as_str()];
        let (mark, style) = match entry["status"]
            .as_str()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("ok") => ("✓", Style::default().fg(Color::Green)),
            Some("issue") => ("✗", Style::default().fg(Color::Red)),
            Some("blocked") => ("!", Style::default().fg(Color::Yellow)),
            Some("n/a") => ("–", dim()),
            _ if stored["verdict"].is_null() => ("○", dim()),
            _ => ("?", Style::default().fg(Color::Yellow)),
        };
        let kind = if id.starts_with('R') {
            "ryzyko"
        } else if id.starts_with('V') {
            "wspólna"
        } else {
            "kontrola"
        };
        lines.push((style, format!("{mark} {id} {kind}: {text}")));
        if let Some(note) = entry["note"].as_str() {
            lines.push((Style::default(), format!("    {note}")));
        } else if !stored["verdict"].is_null() {
            lines.push((dim(), "    nierozliczone [brak dowodu w historii]".into()));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn ac_copy_01_polish_tab_names_preserved_and_windowed_when_narrow() {
        // Wide: 100 columns -> all tabs rendered
        let line = format_tabs("orchestrator", 0, 100);
        let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        for tab in TABS {
            assert!(rendered.contains(tab), "missing tab {tab} in {rendered}");
        }

        // Narrow: 50 columns -> active tab still visible
        let line = format_tabs("worker:t1", 3, 50);
        let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            rendered.contains("[Wynik]"),
            "active tab Wynik missing in {rendered}"
        );
    }

    #[test]
    fn details_footer_hints_prioritized_by_width() {
        // COMMENTS tab
        assert!(details_footer_hint(COMMENTS, 100).contains("d diff"));
        assert!(!details_footer_hint(COMMENTS, 30).contains("d diff"));
        assert!(details_footer_hint(COMMENTS, 30).contains("Esc"));

        // Other tabs
        assert!(details_footer_hint(0, 100).contains("PgUp/PgDn"));
        assert!(!details_footer_hint(0, 40).contains("PgUp/PgDn"));
        assert!(details_footer_hint(0, 40).contains("Esc"));
    }

    #[test]
    fn details_empty_states_in_polish() {
        let view = View::default();
        let mut details = Details::new("worker:t1".into());

        // Tab 1: Sesja empty state
        details.tab = 1;
        let lines = details.lines(&view);
        assert!(
            lines
                .iter()
                .any(|(_, text)| text.contains("Brak zdarzeń sesji"))
        );

        // Tab 2: Wiadomości empty state
        details.tab = 2;
        let lines = details.lines(&view);
        assert!(
            lines
                .iter()
                .any(|(_, text)| text.contains("Brak wiadomości"))
        );

        // Tab 5: Komentarze empty state
        details.tab = COMMENTS;
        let lines = details.lines(&view);
        assert!(
            lines
                .iter()
                .any(|(_, text)| text.contains("Brak komentarzy"))
        );
    }

    #[test]
    fn details_draw_on_tiny_terminals() {
        let view = View::default();
        let mut details = Details::new("orchestrator".into());

        // 60x12 guard floor
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                details.draw(frame, &view);
            })
            .unwrap();

        // 10x4 tiny terminal (must not panic)
        let backend = TestBackend::new(10, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                details.draw(frame, &view);
            })
            .unwrap();

        // 2x2 extreme tiny terminal (must not panic)
        let backend = TestBackend::new(2, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                details.draw(frame, &view);
            })
            .unwrap();
    }
}
