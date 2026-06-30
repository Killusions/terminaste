use std::env;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::thread;

use crossbeam_channel::{unbounded, Receiver, Sender};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use uuid::Uuid;

mod shell_integration;

#[cfg(all(test, unix))]
mod bridge_tests;

pub use shell_integration::{
    detect_shell_family, encode_integration_frame, generated_script, shell_startup, ShellFamily,
    ShellStartup,
};

#[derive(Debug, Clone)]
pub struct PtyConfig {
    pub cols: u16,
    pub rows: u16,
    pub shell: Option<String>,
    pub working_directory: Option<PathBuf>,
}

impl Default for PtyConfig {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            shell: None,
            working_directory: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyCommand {
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyEvent {
    Output(Vec<u8>),
    Exited(Option<i32>),
    Error(String),
}

pub struct PtySession {
    pub id: Uuid,
    pub tx: Sender<PtyCommand>,
    pub rx: Receiver<PtyEvent>,
    wakeup: OutputWakeup,
}

type OutputWakeup = Arc<OnceLock<Box<dyn Fn() + Send + Sync>>>;

#[derive(Clone)]
struct EventSender {
    tx: Sender<PtyEvent>,
    wakeup: OutputWakeup,
}

impl EventSender {
    fn send(&self, event: PtyEvent) -> Result<(), crossbeam_channel::SendError<PtyEvent>> {
        self.tx.send(event)?;
        if let Some(wakeup) = self.wakeup.get() {
            wakeup();
        }
        Ok(())
    }
}

impl PtySession {
    pub fn set_output_wakeup(&self, wakeup: impl Fn() + Send + Sync + 'static) {
        self.wakeup.get_or_init(|| Box::new(wakeup));
    }

    pub fn spawn(config: PtyConfig) -> anyhow::Result<Self> {
        Self::spawn_with_environment(config, &[])
    }

    fn spawn_with_environment(
        config: PtyConfig,
        environment: &[(&str, &str)],
    ) -> anyhow::Result<Self> {
        let id = Uuid::new_v4();
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
        let wakeup = Arc::new(OnceLock::new());
        let event_tx = EventSender {
            tx: event_tx,
            wakeup: wakeup.clone(),
        };
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows: config.rows.max(1),
            cols: config.cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let shell = config.shell.unwrap_or_else(default_shell);
        let mut command = shell_integration::command_builder_for_shell(&shell, &id.to_string())
            .unwrap_or_else(|_| CommandBuilder::new(shell));
        if let Some(cwd) = config.working_directory {
            command.cwd(cwd);
        }
        command.env("TERMINASTE_SESSION", id.to_string());
        command.env("TERM", "xterm-256color");
        for (key, value) in environment {
            command.env(key, value);
        }
        let child = pair.slave.spawn_command(command)?;
        let killer = child.clone_killer();
        drop(pair.slave);
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let master = pair.master;

        spawn_reader_thread(reader, event_tx.clone());
        spawn_writer_thread(master, writer, killer, command_rx, event_tx.clone());
        spawn_exit_thread(child, event_tx);

        Ok(Self {
            id,
            tx: command_tx,
            rx: event_rx,
            wakeup,
        })
    }

    pub fn fake() -> (Self, Sender<PtyEvent>, Receiver<PtyCommand>) {
        let (command_tx, command_rx) = unbounded();
        let (event_tx, event_rx) = unbounded();
        (
            Self {
                id: Uuid::nil(),
                tx: command_tx,
                rx: event_rx,
                wakeup: Arc::new(OnceLock::new()),
            },
            event_tx,
            command_rx,
        )
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.tx.send(PtyCommand::Shutdown);
    }
}

fn spawn_reader_thread(mut reader: Box<dyn Read + Send>, event_tx: EventSender) {
    thread::spawn(move || {
        let mut buffer = vec![0; 256 * 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(size) => {
                    if event_tx
                        .send(PtyEvent::Output(buffer[..size].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    let _ = event_tx.send(PtyEvent::Error(error.to_string()));
                    break;
                }
            }
        }
    });
}

fn spawn_writer_thread(
    master: Box<dyn MasterPty + Send>,
    mut writer: Box<dyn Write + Send>,
    mut killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    command_rx: Receiver<PtyCommand>,
    event_tx: EventSender,
) {
    thread::spawn(move || {
        for command in command_rx {
            match command {
                PtyCommand::Write(bytes) => {
                    if let Err(error) = writer.write_all(&bytes).and_then(|_| writer.flush()) {
                        let _ = event_tx.send(PtyEvent::Error(error.to_string()));
                        break;
                    }
                }
                PtyCommand::Resize { cols, rows } => {
                    if let Err(error) = master.resize(PtySize {
                        rows: rows.max(1),
                        cols: cols.max(1),
                        pixel_width: 0,
                        pixel_height: 0,
                    }) {
                        let _ = event_tx.send(PtyEvent::Error(error.to_string()));
                    }
                }
                PtyCommand::Shutdown => break,
            }
        }
        let _ = killer.kill();
    });
}

fn spawn_exit_thread(mut child: Box<dyn portable_pty::Child + Send + Sync>, event_tx: EventSender) {
    thread::spawn(move || {
        let status = child.wait().ok().map(|status| status.exit_code() as i32);
        let _ = event_tx.send(PtyEvent::Exited(status));
    });
}

pub fn default_shell() -> String {
    if let Some(shell) = env::var_os("TERMINASTE_SHELL") {
        return shell.to_string_lossy().to_string();
    }
    if cfg!(windows) {
        for shell in ["pwsh.exe", "powershell.exe", "wsl.exe"] {
            if command_exists(shell) {
                return shell.to_owned();
            }
        }
        "powershell.exe".to_owned()
    } else if let Some(shell) = env::var_os("SHELL") {
        shell.to_string_lossy().to_string()
    } else {
        for shell in ["/bin/zsh", "/bin/bash", "/bin/sh"] {
            if std::path::Path::new(shell).exists() {
                return shell.to_owned();
            }
        }
        "/bin/sh".to_owned()
    }
}

fn command_exists(command: &str) -> bool {
    env::var_os("PATH")
        .map(|path| env::split_paths(&path).any(|dir| dir.join(command).exists()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_wakes_ui_after_output_is_available_without_polling() {
        let (tx, rx) = unbounded();
        let (wake_tx, wake_rx) = unbounded();
        let wakeup: OutputWakeup = Arc::new(OnceLock::new());
        assert!(wakeup
            .set(Box::new(move || {
                wake_tx.send(()).unwrap();
            }))
            .is_ok());
        spawn_reader_thread(
            Box::new(std::io::Cursor::new(b"interactive output".to_vec())),
            EventSender { tx, wakeup },
        );
        wake_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            rx.try_recv().unwrap(),
            PtyEvent::Output(b"interactive output".to_vec())
        );
    }

    #[test]
    fn fake_session_captures_commands() {
        let (session, events, commands) = PtySession::fake();
        session.tx.send(PtyCommand::Write(b"abc".to_vec())).unwrap();
        events.send(PtyEvent::Output(b"ok".to_vec())).unwrap();
        assert_eq!(commands.recv().unwrap(), PtyCommand::Write(b"abc".to_vec()));
        assert_eq!(session.rx.recv().unwrap(), PtyEvent::Output(b"ok".to_vec()));
    }

    #[cfg(unix)]
    #[test]
    fn zsh_bridge_preserves_completion_bindings_prompt_and_mouse_edits() {
        use base64::Engine;
        use std::time::{Duration, Instant};
        if !std::path::Path::new("/bin/zsh").exists() {
            return;
        }
        let home = std::env::temp_dir().join(format!("terminaste-zsh-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let prefix = "sample-";
        let directories =
            ["one", "one-more", "two", "three", "four"].map(|name| format!("{prefix}{name}"));
        for directory in &directories {
            std::fs::create_dir(home.join(directory)).unwrap();
        }
        std::fs::write(
            home.join(".zshenv"),
            "typeset -g startup_order=env\nHISTFILE=$HOME/.zsh_history\n",
        )
        .unwrap();
        std::fs::write(
            home.join(".zsh_history"),
            "echo saved older\necho saved newer\n",
        )
        .unwrap();
        std::fs::write(home.join(".zprofile"), "startup_order+=,profile\nalias profile_alias='print -r -- profile-alias-ok; [[ $startup_order == env,profile,rc,login ]]'\n").unwrap();
        std::fs::write(home.join(".zshrc"), "startup_order+=,rc\nPROMPT=$'%F{green}test%f \"quoted\"\\n> '\nautoload -Uz compinit\ncompinit -D\nzmodload zsh/complist\nzstyle ':completion:*' menu select\nterminaste_test_complete() { compadd alpha beta; }\ncompdef terminaste_test_complete demo\nbindkey '^E' end-of-line\n").unwrap();
        std::fs::write(home.join(".zlogin"), "startup_order+=,login\n").unwrap();
        let session = PtySession::spawn_with_environment(
            PtyConfig {
                shell: Some("/bin/zsh".to_owned()),
                working_directory: Some(home.clone()),
                ..Default::default()
            },
            &[
                ("HOME", home.to_str().unwrap()),
                ("TERMINASTE_ORIGINAL_ZDOTDIR", ""),
            ],
        )
        .unwrap();
        let mut buffer = Vec::new();
        let mut terminal_output = Vec::new();
        let mut wait_event = |expected_event: &str, expected_input: Option<&str>| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                while let Some(start) = buffer.windows(2).position(|bytes| bytes == b"\x1bP") {
                    let Some(end) = buffer[start + 2..]
                        .windows(2)
                        .position(|bytes| bytes == b"\x1b\\")
                        .map(|offset| start + 2 + offset)
                    else {
                        break;
                    };
                    let frame = String::from_utf8_lossy(&buffer[start + 2..end]).to_string();
                    terminal_output.extend_from_slice(&buffer[..start]);
                    buffer.drain(..end + 2);
                    if let Some(frame) = frame.strip_prefix("terminaste;1;") {
                        let (name, payload) = frame.split_once(";json64;").unwrap();
                        let decoded = base64::prelude::BASE64_URL_SAFE_NO_PAD
                            .decode(payload)
                            .unwrap();
                        let envelope: serde_json::Value = serde_json::from_slice(&decoded)
                            .unwrap_or_else(|error| {
                                panic!("invalid {name} integration event: {error}")
                            });
                        assert_eq!(envelope["session"], session.id.to_string());
                        if name == expected_event
                            && expected_input.is_none_or(|text| envelope["data"]["text"] == text)
                        {
                            return envelope["data"].clone();
                        }
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "shell did not report expected {expected_event} event; pending output: {:?}",
                    String::from_utf8_lossy(&buffer)
                );
                match session.rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(PtyEvent::Output(bytes)) => buffer.extend(bytes),
                    Ok(event) => panic!("unexpected shell lifecycle event: {event:?}"),
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    Err(error) => panic!("shell event stream failed: {error}"),
                }
            }
        };
        let ready = wait_event("ready", None);
        assert_eq!(ready["input_bridge"], true);
        assert_eq!(
            PathBuf::from(ready["current_directory"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            home.canonicalize().unwrap()
        );
        let initial = wait_event("input-buffer", Some(""));
        assert_eq!(initial["columns"], 80);
        assert!(initial["prompt"].as_str().unwrap().contains("\x1b[32mtest"));
        assert!(initial["prompt"]
            .as_str()
            .unwrap()
            .contains("\"quoted\"\n>"));
        session
            .tx
            .send(PtyCommand::Write(b"\x1b[94~".to_vec()))
            .unwrap();
        let history = wait_event("completions", None);
        assert_eq!(history["items"][0]["text"], "echo saved newer");
        assert_eq!(history["items"][1]["text"], "echo saved older");
        wait_event("input-buffer", Some(""));
        let input = format!("cd {prefix}");
        let payload = format!(
            "{};0;{}",
            input.len(),
            base64::prelude::BASE64_STANDARD.encode(&input)
        );
        session
            .tx
            .send(PtyCommand::Write(
                format!("\x1b[99~{:08x}{payload}\x1b[97~", payload.len()).into_bytes(),
            ))
            .unwrap();
        let candidates = wait_event("completions", Some(&input));
        let items = candidates["items"].as_array().unwrap();
        for directory in &directories {
            assert!(
                items
                    .iter()
                    .any(|item| item["text"].as_str().unwrap().trim_end()
                        == format!("cd {directory}/")),
                "missing {directory}: {candidates}"
            );
        }
        wait_event("input-buffer", Some(&input));
        let payload = "0;0;";
        session
            .tx
            .send(PtyCommand::Write(
                format!("\x1b[99~{:08x}{payload}", payload.len()).into_bytes(),
            ))
            .unwrap();
        wait_event("input-buffer", Some(""));
        session
            .tx
            .send(PtyCommand::Write(b"demo al\t".to_vec()))
            .unwrap();
        let completed = wait_event("input-buffer", Some("demo alpha "));
        assert_eq!(completed["cursor"], 11);
        let payload = format!("5;0;{}", base64::prelude::BASE64_STANDARD.encode("demo "));
        session
            .tx
            .send(PtyCommand::Write(
                format!("\x1b[99~{:08x}{payload}\x1b[97~", payload.len()).into_bytes(),
            ))
            .unwrap();
        let candidates = wait_event("completions", None);
        assert_eq!(
            candidates["items"].as_array().unwrap().len(),
            2,
            "{candidates}"
        );
        assert_eq!(candidates["items"][0]["text"], "demo alpha");
        assert_eq!(candidates["items"][1]["text"], "demo beta");
        wait_event("input-buffer", Some("demo "));
        session
            .tx
            .send(PtyCommand::Write(b"al\t".to_vec()))
            .unwrap();
        wait_event("input-buffer", Some("demo alpha "));
        session
            .tx
            .send(PtyCommand::Write(b"\x01\x05".to_vec()))
            .unwrap();
        assert_eq!(
            wait_event("input-buffer", Some("demo alpha "))["cursor"],
            11
        );
        let payload = format!("3;1;{}", base64::prelude::BASE64_STANDARD.encode("echo 界"));
        session
            .tx
            .send(PtyCommand::Write(
                format!("\x1b[99~{:08x}{payload}", payload.len()).into_bytes(),
            ))
            .unwrap();
        let replaced = wait_event("input-buffer", Some("echo 界"));
        assert_eq!(replaced["cursor"], 3);
        assert_eq!(replaced["revision"], 1);
        session.tx.send(PtyCommand::Write(b"\r".to_vec())).unwrap();
        let started = wait_event("command-start", None);
        assert_eq!(started["command"], "echo 界");
        let finished = wait_event("command-end", None);
        assert_eq!(finished["command"], "echo 界");
        assert_eq!(finished["exit_code"], 0);
        wait_event("prompt-start", None);
        wait_event("input-buffer", Some(""));
        for iteration in 0..5 {
            let revision = 2 + iteration * 21;
            let payload = format!(
                "5;{revision};{}",
                base64::prelude::BASE64_STANDARD.encode("demo ")
            );
            session
                .tx
                .send(PtyCommand::Write(
                    format!("\x1b[99~{:08x}{payload}\x1b[97~", payload.len()).into_bytes(),
                ))
                .unwrap();
            let candidates = wait_event("completions", None);
            assert_eq!(candidates["items"].as_array().unwrap().len(), 2);
            wait_event("input-buffer", Some("demo "));
            let mut burst = Vec::new();
            for index in 1..=20 {
                let command = format!("print -r -- burst-{iteration}-{index}");
                let payload = format!(
                    "{};{};{}",
                    command.len(),
                    revision + index,
                    base64::prelude::BASE64_STANDARD.encode(&command)
                );
                burst.extend(format!("\x1b[99~{:08x}{payload}", payload.len()).into_bytes());
            }
            burst.push(b'\r');
            session.tx.send(PtyCommand::Write(burst)).unwrap();
            assert_eq!(
                wait_event("command-start", None)["command"],
                format!("print -r -- burst-{iteration}-20")
            );
            assert_eq!(wait_event("command-end", None)["exit_code"], 0);
            assert_eq!(wait_event("editor-ready", None)["revision"], revision + 20);
            wait_event("input-buffer", Some(""));
        }
        let prefix = "print -r -- burst-";
        let payload = format!(
            "{};107;{}",
            prefix.len(),
            base64::prelude::BASE64_STANDARD.encode(prefix)
        );
        session
            .tx
            .send(PtyCommand::Write(
                format!("\x1b[99~{:08x}{payload}\x1b[96~", payload.len()).into_bytes(),
            ))
            .unwrap();
        assert_eq!(
            wait_event("input-buffer", Some("print -r -- burst-4-20"))["cursor"],
            "print -r -- burst-4-20".len()
        );
        session
            .tx
            .send(PtyCommand::Write(b"\x1b[96~".to_vec()))
            .unwrap();
        wait_event("input-buffer", Some("print -r -- burst-3-20"));
        session
            .tx
            .send(PtyCommand::Write(b"\x1b[95~".to_vec()))
            .unwrap();
        wait_event("input-buffer", Some("print -r -- burst-4-20"));
        session
            .tx
            .send(PtyCommand::Write(b"\x1b[95~".to_vec()))
            .unwrap();
        wait_event("input-buffer", Some(prefix));
        let payload = format!("0;108;{}", base64::prelude::BASE64_STANDARD.encode(""));
        session
            .tx
            .send(PtyCommand::Write(
                format!("\x1b[99~{:08x}{payload}\x1b[96~", payload.len()).into_bytes(),
            ))
            .unwrap();
        wait_event("input-buffer", Some("print -r -- burst-4-20"));
        session
            .tx
            .send(PtyCommand::Write(b"\x1b[95~".to_vec()))
            .unwrap();
        wait_event("input-buffer", Some(""));
        for prefix in [
            "print -r -- burst-",
            "",
            "no-history-match-xyz",
            "echo one\necho two",
        ] {
            let payload = format!("0;109;{}", base64::prelude::BASE64_STANDARD.encode(prefix));
            session
                .tx
                .send(PtyCommand::Write(
                    format!("\x1b[99~{:08x}{payload}\x1b[94~", payload.len()).into_bytes(),
                ))
                .unwrap();
            let options = wait_event("completions", None);
            let items = options["items"].as_array().unwrap();
            assert_eq!(options["text"], prefix);
            assert!(items
                .iter()
                .all(|item| prefix.contains('\n')
                    || item["text"].as_str().unwrap().starts_with(prefix)));
            if prefix == "no-history-match-xyz" {
                assert!(items.is_empty());
            } else {
                assert_eq!(items[0]["text"], "print -r -- burst-4-20");
                assert!(items.len() >= 5);
            }
            wait_event("input-buffer", Some(prefix));
        }
        let payload = format!("0;110;{}", base64::prelude::BASE64_STANDARD.encode(""));
        session
            .tx
            .send(PtyCommand::Write(
                format!("\x1b[99~{:08x}{payload}", payload.len()).into_bytes(),
            ))
            .unwrap();
        wait_event("input-buffer", Some(""));
        session
            .tx
            .send(PtyCommand::Write(b"profile_alias\r".to_vec()))
            .unwrap();
        wait_event("command-start", None);
        assert_eq!(wait_event("command-end", None)["exit_code"], 0);
        wait_event("input-buffer", Some(""));
        if std::path::Path::new("/bin/bash").exists() {
            std::fs::create_dir(home.join("subdir")).unwrap();
            session
                .tx
                .send(PtyCommand::Write(b"bash\r".to_vec()))
                .unwrap();
            assert_eq!(wait_event("command-start", None)["command"], "bash");
            let ready = wait_event("ready", None);
            assert_eq!(ready["shell_family"], "bash");
            if ready["input_bridge"] == true {
                wait_event("input-buffer", Some(""));
                let input = "cd sub";
                let payload = format!(
                    "{};1;{}",
                    input.len(),
                    base64::prelude::BASE64_STANDARD.encode(input)
                );
                session
                    .tx
                    .send(PtyCommand::Write(
                        format!("\x1b[99~{:08x}{payload}\x1b[98~\x1b[97~", payload.len())
                            .into_bytes(),
                    ))
                    .unwrap();
                let completions = wait_event("completions", None);
                assert!(completions["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| item["text"] == "cd subdir/"));

                let input = "exit";
                let payload = format!(
                    "{};2;{}",
                    input.len(),
                    base64::prelude::BASE64_STANDARD.encode(input)
                );
                session
                    .tx
                    .send(PtyCommand::Write(
                        format!("\x1b[99~{:08x}{payload}\x1b[98~\r", payload.len()).into_bytes(),
                    ))
                    .unwrap();
            } else {
                session
                    .tx
                    .send(PtyCommand::Write(b"exit\r".to_vec()))
                    .unwrap();
            }
            wait_event("command-end", None);
            wait_event("input-buffer", Some(""));
        }
        assert!(String::from_utf8_lossy(&terminal_output).contains("profile-alias-ok"));
        session
            .tx
            .send(PtyCommand::Write(b"exit\r".to_vec()))
            .unwrap();
        while !matches!(
            session.rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            PtyEvent::Exited(_)
        ) {}
        drop(session);
        std::fs::remove_dir_all(home).unwrap();
    }
}
