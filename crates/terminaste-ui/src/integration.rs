use super::*;

#[derive(Debug, Deserialize)]
struct IntegrationEnvelope {
    session: String,
    shell_id: Option<String>,
    sequence: u64,
    data: serde_json::Value,
}

impl TerminalPane {
    pub(super) fn process_ordered_pty_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if self.suppress_prompt_output && self.prompt_capture.len() < 16 * 1024 {
                self.prompt_capture.push(*byte);
            }
            for event in self.model.process_bytes(std::slice::from_ref(byte)) {
                self.handle_terminal_event(event);
            }
        }
        if self.surface.submit_pending && self.active_command.is_none() {
            self.model.discard_pending_output();
        }
        let responses = self.model.take_responses();
        self.write_terminal(responses);
    }

    fn finish_prompt_capture(&mut self) {
        if self.prompt_capture.is_empty() {
            return;
        }
        let prompt = shell_prompt::plain_text(&String::from_utf8_lossy(&self.prompt_capture));
        self.prompt_capture.clear();
        self.set_input_prompt_text(prompt);
    }

    fn set_input_prompt_text(&mut self, prompt: String) {
        let lines = prompt
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        if !lines.is_empty() {
            self.input_prompt_text = Some(lines.join("\n"));
        }
    }

    fn handle_terminal_event(&mut self, event: TerminalEvent) {
        match event {
            TerminalEvent::Bell => {
                self.surface.bell = true;
            }
            TerminalEvent::TitleChanged(title) => {
                self.pending_title = Some((title, Instant::now()));
            }
            TerminalEvent::ClipboardWriteRequested(payload) => {
                if let Some((_, data)) = payload.split_once(';') {
                    if let Ok(bytes) = base64::prelude::BASE64_STANDARD.decode(data) {
                        self.surface.clipboard = String::from_utf8(bytes).ok();
                    }
                }
            }
            TerminalEvent::Integration(event) => {
                self.apply_integration_event(event.name, event.payload)
            }
        }
    }

    fn apply_integration_event(&mut self, name: String, payload: String) {
        let Ok(decoded) = BASE64_URL_SAFE_NO_PAD.decode(payload.as_bytes()) else {
            return;
        };
        let Ok(envelope) = serde_json::from_slice::<IntegrationEnvelope>(&decoded) else {
            return;
        };
        let session = self.pty.as_ref().map(|pty| pty.id.to_string());
        if session.as_deref() != Some(envelope.session.as_str()) {
            return;
        }
        if let Some(shell_id) = &envelope.shell_id {
            if self.surface.shell_id.as_ref() != Some(shell_id) {
                if let Some(index) = self
                    .surface
                    .parent_shells
                    .iter()
                    .position(|shell| &shell.id == shell_id)
                {
                    let parent = self.surface.parent_shells.remove(index);
                    self.surface.parent_shells.truncate(index);
                    if self.active_command.take().is_some() {
                        self.model.finish_running_command(0);
                    }
                    self.surface.last_sequence = parent.sequence;
                    self.surface.input_revision = parent.revision;
                    self.surface.input_bridge = parent.input_bridge;
                    self.surface.query_bridge = parent.query_bridge;
                    self.surface.control_keys = parent.control_keys;
                    self.surface.isolated_shell = parent.isolated_shell;
                    self.editor.shell_bridge = parent.input_bridge;
                    self.surface.submit_pending = false;
                    self.surface.prompt_ansi = None;
                    self.surface.right_prompt_ansi = None;
                    self.surface.shell_id = Some(parent.id);
                    self.editor.clear();
                    self.focus_input();
                    self.dismiss_completions();
                } else if name == "ready" {
                    if let Some(id) = self.surface.shell_id.take() {
                        self.surface
                            .parent_shells
                            .push(terminal_surface::ShellContext {
                                id,
                                sequence: self.surface.last_sequence,
                                revision: self.surface.input_revision,
                                input_bridge: self.surface.input_bridge,
                                query_bridge: self.surface.query_bridge,
                                control_keys: self.surface.control_keys,
                                isolated_shell: self.surface.isolated_shell,
                            });
                    }
                    self.surface.shell_id = Some(shell_id.clone());
                    self.surface.last_sequence = 0;
                } else {
                    return;
                }
            }
        }
        if envelope.sequence <= self.surface.last_sequence {
            if name != "ready" || self.active_command.is_none() {
                return;
            }
            self.surface.last_sequence = 0;
        }
        self.surface.last_sequence = envelope.sequence;
        if let Some(cwd) = envelope
            .data
            .get("current_directory")
            .and_then(|value| value.as_str())
        {
            self.cwd = PathBuf::from(cwd);
        }
        match name.as_str() {
            "editor-ready" => {
                if envelope
                    .data
                    .get("revision")
                    .and_then(|value| value.as_u64())
                    .is_none_or(|revision| revision < self.surface.input_revision)
                {
                    return;
                }
                self.surface.submit_pending = false;
                if self.active_command.is_none() {
                    self.model.discard_submitted_command();
                }
                self.editor.focus_requested = true;
            }
            "completions" => {
                let revision = envelope
                    .data
                    .get("revision")
                    .and_then(|value| value.as_u64());
                if revision.is_some()
                    && revision == self.surface.completion_request
                    && revision == Some(self.surface.input_revision)
                    && envelope.data.get("text").and_then(|value| value.as_str())
                        == Some(self.editor.text())
                {
                    self.surface.completion_request = None;
                    self.completion_revision = self.completion_revision.saturating_add(1);
                    if let Some(items) = envelope
                        .data
                        .get("items")
                        .and_then(|value| value.as_array())
                    {
                        self.completions.clear();
                        self.surface.completion_cursors.clear();
                        let mut seen = std::collections::HashSet::new();
                        for item in items {
                            if let (Some(text), Some(cursor)) = (
                                item.get("text").and_then(|value| value.as_str()),
                                item.get("cursor").and_then(|value| value.as_u64()),
                            ) {
                                if !seen.insert(text) {
                                    continue;
                                }
                                self.completions.push(CompletionItem {
                                    label: text.to_owned(),
                                    replacement: text.to_owned(),
                                    description: if self.history_search.is_some() {
                                        "history".to_owned()
                                    } else {
                                        String::new()
                                    },
                                    kind: CompletionKind::Command,
                                    range: 0..self.editor.text().len(),
                                    score: 0,
                                });
                                self.surface.completion_cursors.push(cursor as usize);
                            }
                        }
                        self.selected_completion = 0;
                        self.completion_navigating = false;
                        if self.history_search.is_some() || self.ghost_history_request {
                            for item in self.completions.iter().rev() {
                                self.history_suggestions
                                    .retain(|command| command != &item.replacement);
                                self.history_suggestions.insert(0, item.replacement.clone());
                            }
                            self.history_suggestions.truncate(1_000);
                        }
                        if self.ghost_history_request {
                            self.dismiss_completions();
                            return;
                        }
                        if self.completions.is_empty() && self.history_search.is_none() {
                            self.refresh_local_completions();
                        }
                    }
                }
            }
            "input-buffer" => {
                if self.surface.submit_pending || self.active_command.is_some() {
                    return;
                }
                let revision = envelope
                    .data
                    .get("revision")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0);
                if revision < self.surface.input_revision {
                    return;
                }
                self.surface.prompt_cols = envelope
                    .data
                    .get("columns")
                    .and_then(|value| value.as_u64())
                    .and_then(|cols| u16::try_from(cols).ok())
                    .filter(|cols| *cols > 0);
                if let Some(text) = envelope.data.get("text").and_then(|value| value.as_str()) {
                    if self.editor.text() != text {
                        self.dismiss_completions();
                        self.editor.set_text(text.to_owned());
                    }
                    if let Some(cursor) =
                        envelope.data.get("cursor").and_then(|value| value.as_u64())
                    {
                        self.editor.cursor = char_to_byte_index(text, cursor as usize);
                    }
                }
                self.editor.shell_command_mode = envelope
                    .data
                    .get("keymap")
                    .and_then(|value| value.as_str())
                    .is_some_and(|keymap| keymap == "vicmd");
                if let Some(prompt) = envelope.data.get("prompt").and_then(|value| value.as_str()) {
                    self.surface.prompt_ansi = Some(prompt.to_owned());
                    self.set_input_prompt_text(shell_prompt::plain_text(prompt));
                }
                self.surface.right_prompt_ansi = envelope
                    .data
                    .get("right_prompt")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned);
            }
            "prompt-start" => {
                let snapshot = self.model.snapshot();
                self.surface.prompt_row = snapshot.visible_row_start + snapshot.cursor_row as u64;
                self.finish_prompt_capture();
                self.prompt_capture.clear();
                self.suppress_prompt_output = true;
            }
            "prompt-end" => {
                self.finish_prompt_capture();
                self.suppress_prompt_output = false;
            }
            "ready" => {
                if let Some(active) = self.active_command.take() {
                    self.model
                        .finish_running_command_matching(Some(&active.command), 0);
                }
                self.integration_ready = true;
                self.surface.input_revision = 0;
                self.surface.submit_pending = false;
                self.surface.prompt_ansi = None;
                self.surface.right_prompt_ansi = None;
                self.editor.clear();
                self.focus_input();
                self.dismiss_completions();
                self.surface.input_bridge = envelope
                    .data
                    .get("input_bridge")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                self.editor.shell_bridge = self.surface.input_bridge;
                self.surface.query_bridge = envelope
                    .data
                    .get("query_bridge")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                self.surface.control_keys = envelope
                    .data
                    .get("control_keys")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                self.surface.isolated_shell = envelope
                    .data
                    .get("isolated")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                if let Some(shell) = envelope
                    .data
                    .get("shell_family")
                    .and_then(|value| value.as_str())
                {
                    self.status = format!("{shell} integration ready");
                }
            }
            "aliases" => {
                if let Some(map) = envelope.data.get("map").and_then(|value| value.as_object()) {
                    self.aliases = map
                        .iter()
                        .filter_map(|(key, value)| {
                            value.as_str().map(|value| (key.clone(), value.to_owned()))
                        })
                        .collect();
                }
            }
            "command-start" => {
                self.dismiss_completions();
                self.editor.clear();
                self.editor.focus_requested = true;
                self.finish_prompt_capture();
                self.suppress_prompt_output = false;
                if let Some(command) = envelope
                    .data
                    .get("command")
                    .and_then(|value| value.as_str())
                {
                    let command = command.to_owned();
                    if self
                        .active_command
                        .as_ref()
                        .is_none_or(|active| active.command != command)
                    {
                        self.active_command = Some(ActiveCommandMeta {
                            command: command.clone(),
                            started_at: Instant::now(),
                        });
                    }
                    self.status = "command running".to_owned();
                    if self.surface.submit_pending && self.model.has_running_command() {
                        self.model.confirm_submitted_command(command);
                    } else {
                        self.model.start_integrated_command(command);
                    }
                    self.surface.submit_pending = false;
                    self.select_running_block();
                }
            }
            "command-end" => {
                let exit_code = envelope
                    .data
                    .get("exit_code")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(0) as i32;
                let command = envelope
                    .data
                    .get("command")
                    .and_then(|value| value.as_str())
                    .or_else(|| {
                        self.active_command
                            .as_ref()
                            .map(|active| active.command.as_str())
                    })
                    .map(str::to_owned);
                self.model
                    .finish_running_command_matching(command.as_deref(), exit_code);
                if self.active_command.as_ref().is_some_and(|active| {
                    command
                        .as_deref()
                        .is_none_or(|command| active.command == command)
                }) {
                    let active = self.active_command.take().unwrap();
                    self.last_command_summary = Some(format!(
                        "{} exit {exit_code} in {}",
                        active.command,
                        format_duration(millis(active.started_at.elapsed()))
                    ));
                }
                self.status = if exit_code == 0 {
                    "ready".to_owned()
                } else {
                    format!("last command failed: {exit_code}")
                };
                self.focus_input();
            }
            _ => {}
        }
    }
}
