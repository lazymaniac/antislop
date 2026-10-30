use super::super::*;

impl App {
    /// Adds a connected project to the screen, once; its conversations are
    /// listed in the background from then on. Returns its place.
    pub fn register_project(&mut self, conn: workspace::ProjectConn) -> usize {
        if let Some(index) = self.projects.iter().position(|p| p.path == conn.path) {
            return index;
        }
        let _ = projects::Projects::from_env().and_then(|list| list.remember(&conn.path));
        spawn_project_poller(conn.client.clone(), conn.path.clone(), self.updates.clone());
        self.projects.push(conn);
        self.projects.len() - 1
    }

    /// Makes a connected project the one on screen, showing `conversation`
    /// or the project's latest.
    pub fn switch_project(&mut self, index: usize, conversation: Option<String>) {
        let Some(conn) = self.projects.get(index).cloned() else {
            return;
        };
        let conversation = match conversation {
            Some(id) => id,
            None => match workspace::latest_or_new(&conn) {
                Ok(id) => id,
                Err(error) => {
                    self.notice = Some(format!("✗ {}: {error}", conn.name));
                    return;
                }
            },
        };
        self.restore_project_history(index, &conn);
        self.switch_config_workspace(&conn);
        self.active_project = index;
        self.client = conn.client.clone();
        self.project = conn.name.clone();
        self.open(conversation);
    }

    /// Opens a project by directory, or switches to it if it is open. The
    /// directory must be trusted; if it is not, the user is asked first.
    pub fn open_project(&mut self, path: &str) {
        let path = expand_home(path);
        if let Ok(canonical) = path.canonicalize()
            && let Some(index) = self.projects.iter().position(|p| p.path == canonical)
        {
            self.switch_project(index, None);
            return;
        }
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let update = match trust::Registry::from_env() {
                Err(error) => Update::Notice(format!("✗ {error}")),
                Ok(registry) => match workspace::connect(&path, &registry) {
                    Ok(conn) => match workspace::latest_or_new(&conn) {
                        Ok(id) => Update::ProjectReady {
                            conn: Box::new(conn),
                            conversation: Some(id),
                        },
                        Err(error) => Update::Notice(format!("✗ {error}")),
                    },
                    Err(workspace::ConnectError::NotTrusted(path)) => Update::ProjectTrust(path),
                    Err(workspace::ConnectError::Failed(error)) => {
                        Update::Notice(format!("✗ {error}"))
                    }
                },
            };
            let _ = updates.send(update);
        });
    }

    /// The projects open here and those opened before, to pick from.
    pub fn open_projects_picker(&mut self) {
        let mut items = self.open_project_items();
        self.append_remembered_projects(&mut items);
        let mut picker = Picker::new(Kind::Projects);
        picker.projects = items;
        self.picker = Some(picker);
    }

    fn restore_project_history(&mut self, index: usize, conn: &workspace::ProjectConn) {
        if self.active_project != index && self.history_path.is_some() {
            self.load_history(conn.client.state_dir());
        }
    }

    fn switch_config_workspace(&mut self, conn: &workspace::ProjectConn) {
        if self.client.state_dir() == conn.client.state_dir() {
            return;
        }
        if let Some(mut editor) = self.config_editor.take() {
            editor.visible = false;
            self.config_drafts.insert(
                self.client.state_dir().to_string_lossy().into_owned(),
                editor,
            );
        }
        self.config_editor = self
            .config_drafts
            .remove(conn.client.state_dir().to_string_lossy().as_ref());
    }

    fn open_project_items(&self) -> Vec<palette::ProjectItem> {
        self.projects
            .iter()
            .enumerate()
            .map(|(index, project)| project_item(project, index == self.active_project))
            .collect()
    }

    fn append_remembered_projects(&self, items: &mut Vec<palette::ProjectItem>) {
        if let Ok(list) = projects::Projects::from_env() {
            items.extend(
                list.list()
                    .into_iter()
                    .filter(|path| !self.projects.iter().any(|p| p.path == *path))
                    .map(remembered_project_item),
            );
        }
    }

    /// Keys while the user is asked whether to trust a directory to open.
    pub(in crate::lean::tui) fn project_trust_key(&mut self, key: event::KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(path) = self.project_trust.take() {
                    match trust::Registry::from_env().and_then(|r| r.trust(&path)) {
                        Ok(()) => self.open_project(&path.to_string_lossy()),
                        Err(error) => self.notice = Some(format!("✗ {error}")),
                    }
                }
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('q') => {
                self.project_trust = None;
            }
            _ => {}
        }
    }
}

fn spawn_project_poller(
    client: Client,
    path: std::path::PathBuf,
    updates: std::sync::mpsc::Sender<Update>,
) {
    std::thread::spawn(move || {
        loop {
            if let Ok(list) = client.call(&Request::Conversations) {
                let conversations = list
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|item| item["project"] == path.to_string_lossy().as_ref())
                    .cloned()
                    .collect();
                if updates
                    .send(Update::ProjectStatus {
                        path: path.clone(),
                        conversations,
                    })
                    .is_err()
                {
                    return;
                }
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}

fn project_item(project: &workspace::ProjectConn, current: bool) -> palette::ProjectItem {
    let (working, attention) = project.badges();
    let mut title = format!("{} · {}", project.name, project.path.display());
    if current {
        title.push_str(" (current)");
    }
    if working > 0 {
        title.push_str(&format!("  ●{working}"));
    }
    if attention > 0 {
        title.push_str(&format!("  !{attention}"));
    }
    palette::ProjectItem {
        path: project.path.to_string_lossy().into_owned(),
        title,
    }
}

fn remembered_project_item(path: std::path::PathBuf) -> palette::ProjectItem {
    palette::ProjectItem {
        title: format!(
            "{} · {} (not open)",
            workspace::name_of(&path),
            path.display()
        ),
        path: path.to_string_lossy().into_owned(),
    }
}
