use std::fs;
use std::io::Write;
use std::path::Path;

use super::*;

#[derive(Serialize, Deserialize)]
struct SavedSession {
    version: u8,
    active_tab: usize,
    tabs: Vec<SavedTab>,
}

#[derive(Serialize, Deserialize)]
struct SavedTab {
    title: String,
    active_pane: usize,
    tree: pane_tree::PaneTree,
    panes: Vec<SavedPane>,
}

#[derive(Serialize, Deserialize)]
struct SavedPane {
    id: Uuid,
    cwd: PathBuf,
    title: String,
    blocks: Vec<CommandBlock>,
}

impl SavedSession {
    fn capture(app: &TerminasteApp) -> Self {
        Self {
            version: 1,
            active_tab: app.active_tab,
            tabs: app
                .tabs
                .iter()
                .map(|tab| SavedTab {
                    title: tab.title.clone(),
                    active_pane: tab.active_pane,
                    tree: tab.tree.clone(),
                    panes: tab
                        .panes
                        .iter()
                        .map(|pane| SavedPane {
                            id: pane.id,
                            cwd: pane.cwd.clone(),
                            title: pane.title.clone(),
                            blocks: pane.model.command_blocks(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    fn apply(self, app: &mut TerminasteApp) -> anyhow::Result<()> {
        anyhow::ensure!(self.version == 1, "unsupported session version");
        anyhow::ensure!(!self.tabs.is_empty(), "empty session");
        for tab in &self.tabs {
            let mut leaves = Vec::new();
            tab.tree.leaves(&mut leaves);
            let mut ids = tab.panes.iter().map(|pane| pane.id).collect::<Vec<_>>();
            leaves.sort();
            ids.sort();
            anyhow::ensure!(
                !ids.is_empty() && leaves == ids && !ids.windows(2).any(|pair| pair[0] == pair[1]),
                "invalid session layout"
            );
        }
        app.tabs = self
            .tabs
            .into_iter()
            .map(|tab| {
                let panes = tab
                    .panes
                    .into_iter()
                    .map(|saved| {
                        let mut settings = app.loaded.settings.clone();
                        if saved.cwd.is_dir() {
                            settings.startup.working_directory = Some(saved.cwd.clone());
                        }
                        let mut pane = if app.headless {
                            TerminalPane::fake()
                        } else {
                            TerminalPane::new(&settings)
                        };
                        pane.id = saved.id;
                        pane.cwd = startup_directory(&settings);
                        pane.title = saved.title;
                        pane.model.restore_blocks(saved.blocks);
                        pane
                    })
                    .collect::<Vec<_>>();
                TabState {
                    title: tab.title,
                    active_pane: tab.active_pane.min(panes.len() - 1),
                    tree: tab.tree,
                    panes,
                }
            })
            .collect();
        app.active_tab = self.active_tab.min(app.tabs.len() - 1);
        Ok(())
    }

    fn write(&self, path: &Path) -> anyhow::Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid session path"))?;
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!("session-{}.tmp", Uuid::new_v4()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let result = (|| -> anyhow::Result<()> {
            let mut file = options.open(&temporary)?;
            file.write_all(&serde_json::to_vec(self)?)?;
            file.sync_all()?;
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
}

impl pane_tree::PaneTree {
    fn leaves(&self, ids: &mut Vec<Uuid>) {
        match self {
            Self::Leaf(id) => ids.push(*id),
            Self::Split { first, second, .. } => {
                first.leaves(ids);
                second.leaves(ids);
            }
        }
    }
}

impl TerminasteApp {
    fn session_path(&self) -> PathBuf {
        self.loaded.path.with_file_name("session.json")
    }

    pub(super) fn restore_session(&mut self) {
        if !self.loaded.settings.workspace.restore_sessions {
            return;
        }
        let result = fs::read(self.session_path())
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(serde_json::from_slice::<SavedSession>(&bytes)?))
            .and_then(|saved| saved.apply(self));
        if let Err(error) = result {
            if !error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                self.toast(format!("Could not restore session: {error}"));
            }
        }
    }

    pub(super) fn persist_session(&mut self) {
        if self.headless {
            return;
        }
        if !self.loaded.settings.workspace.restore_sessions {
            let _ = fs::remove_file(self.session_path());
            return;
        }
        if let Err(error) = SavedSession::capture(self).write(&self.session_path()) {
            self.toast(format!("Could not save session: {error}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_restores_tabs_splits_directories_and_blocks_without_running_commands() {
        let directory = std::env::temp_dir().join(format!("terminaste-session-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("session.json");
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.submit_active_input_for_tests("echo old");
        app.active_terminal_mut()
            .unwrap()
            .model
            .append_running_command_output("old\n");
        app.finish_active_command_for_tests(0);
        app.new_tab();
        app.split_active_for_tests();
        let pane = app.active_terminal_mut().unwrap();
        pane.cwd = directory.clone();
        pane.model.start_integrated_command("sleep 100".to_owned());
        pane.model.append_running_command_output("partial");
        SavedSession::capture(&app).write(&path).unwrap();
        let mut restored = TerminasteApp::headless_for_tests(Settings::default());
        restored.loaded.path = directory.join("settings.toml");
        restored.restore_session();
        assert_eq!(restored.tabs.len(), 2);
        assert_eq!(restored.active_tab, 1);
        assert_eq!(restored.active_pane_count(), 2);
        assert_eq!(restored.tabs[1].active_pane, 1);
        assert_eq!(restored.active_terminal().unwrap().cwd, directory);
        let blocks = restored.active_pane_snapshot_for_tests().unwrap().blocks;
        assert_eq!(blocks[0].output, "partial");
        assert!(!blocks[0].running);
        assert_eq!(blocks[0].exit_code, None);
        assert!(restored.active_terminal().unwrap().active_command.is_none());
        assert_eq!(
            restored.tabs[0].panes[0].model.snapshot().blocks[0].output,
            "old"
        );
        restored.loaded.settings.workspace.restore_sessions = false;
        restored.tabs.truncate(1);
        restored.restore_session();
        assert_eq!(restored.tabs.len(), 1);
        fs::write(&path, b"{broken").unwrap();
        restored.loaded.settings.workspace.restore_sessions = true;
        restored.restore_session();
        assert_eq!(restored.tabs.len(), 1);
        assert!(restored.toast.is_some());
        fs::remove_dir_all(directory).unwrap();
    }
}
