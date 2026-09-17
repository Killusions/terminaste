use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

use base64::prelude::{Engine, BASE64_STANDARD, BASE64_URL_SAFE_NO_PAD};
use serde_json::Value;

use super::*;

struct ShellTest {
    session: PtySession,
    buffer: Vec<u8>,
    home: PathBuf,
    terminal: terminaste_core::TerminalModel,
    control_keys: bool,
    query_only: bool,
    output: Vec<u8>,
}

impl ShellTest {
    fn new(shell: &str) -> Self {
        let home = env::temp_dir().join(format!("terminaste-bridge-{}", Uuid::new_v4()));
        std::fs::create_dir_all(home.join("subdir")).unwrap();
        let session = PtySession::spawn_with_environment(
            PtyConfig {
                shell: Some(shell.to_owned()),
                working_directory: Some(home.clone()),
                ..Default::default()
            },
            &[
                ("HOME", home.to_str().unwrap()),
                ("XDG_CONFIG_HOME", home.to_str().unwrap()),
                ("SHELL", shell),
                ("TERMINASTE_ORIGINAL_ZDOTDIR", ""),
            ],
        )
        .unwrap();
        Self {
            session,
            buffer: Vec::new(),
            home,
            terminal: terminaste_core::TerminalModel::new(80, 24, 1000),
            control_keys: false,
            query_only: false,
            output: Vec::new(),
        }
    }

    fn write(&self, bytes: &[u8]) {
        self.session
            .tx
            .send(PtyCommand::Write(bytes.to_vec()))
            .unwrap();
    }

    fn mock_shell_launchers(&self) {
        for name in ["sudo", "su", "ssh"] {
            let path = self.home.join(name);
            std::fs::write(&path, "#!/bin/sh\nif [ \"$1\" = -G ]; then printf 'remotecommand none\\nrequesttty auto\\n'; exit; fi\nprintf '%s\\0' \"$@\" >\"$HOME/launcher-args\"\nfor arg do body=$arg; done\nexec /bin/sh -c \"$body\"\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn input(&self, text: &str, revision: u64, key: &str) {
        if self.query_only {
            let kind = if key == "\x1b[94~" {
                "history"
            } else {
                "complete"
            };
            self.write(
                format!(
                    "__terminaste_query {} {revision} '{}' {kind}\r",
                    text.chars().count(),
                    BASE64_STANDARD.encode(text)
                )
                .as_bytes(),
            );
            return;
        }
        let payload = format!(
            "{};{revision};{}",
            text.chars().count(),
            BASE64_STANDARD.encode(text)
        );
        let bytes = format!("\x1b[99~{:08x}{payload}\x1b[98~{key}", payload.len());
        let bytes = if self.control_keys {
            bytes
                .replace("\x1b[99~", "\x18r")
                .replace("\x1b[98~", "\x18f")
                .replace("\x1b[97~", "\x18o")
                .replace("\x1b[94~", "\x18h")
        } else {
            bytes
        };
        self.write(bytes.as_bytes());
    }

    fn event(&mut self, expected: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            while let Some(start) = self.buffer.windows(2).position(|part| part == b"\x1bP") {
                let Some(end) = self.buffer[start + 2..]
                    .windows(2)
                    .position(|part| part == b"\x1b\\")
                    .map(|offset| start + 2 + offset)
                else {
                    break;
                };
                let frame = String::from_utf8_lossy(&self.buffer[start + 2..end]).to_string();
                self.buffer.drain(..end + 2);
                if let Some(frame) = frame.strip_prefix("terminaste;1;") {
                    let (name, payload) = frame.split_once(";json64;").unwrap();
                    let json = BASE64_URL_SAFE_NO_PAD.decode(payload).unwrap();
                    let envelope: Value = serde_json::from_slice(&json).unwrap_or_else(|error| {
                        panic!(
                            "invalid {name}: {error}: {}",
                            String::from_utf8_lossy(&json)
                        )
                    });
                    if name == expected {
                        return envelope;
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "missing {expected}: {}",
                String::from_utf8_lossy(&self.output[self.output.len().saturating_sub(6000)..])
            );
            match self.session.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(PtyEvent::Output(bytes)) => {
                    self.output.extend_from_slice(&bytes);
                    self.terminal.process_bytes(&bytes);
                    let responses = self.terminal.take_responses();
                    if !responses.is_empty() {
                        self.write(&responses);
                    }
                    self.buffer.extend(bytes);
                }
                Ok(event) => panic!("unexpected {event:?}"),
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("{error}"),
            }
        }
    }
}

impl Drop for ShellTest {
    fn drop(&mut self) {
        self.session.tx.send(PtyCommand::Shutdown).unwrap();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

#[test]
fn shell_bridges_stream_completion_and_history_batches() {
    for program in ["/bin/zsh", "/bin/bash", "/opt/homebrew/bin/fish"] {
        if !Path::new(program).exists() {
            continue;
        }
        let mut shell = ShellTest::new(program);
        for index in 0..45 {
            std::fs::create_dir(shell.home.join(format!("stream-{index:02}"))).unwrap();
        }
        let ready = shell.event("ready");
        shell.control_keys = ready["data"]["control_keys"] == true;
        shell.query_only = ready["data"]["input_bridge"] != true;
        let prompt = if shell.query_only {
            "prompt-start"
        } else {
            "editor-ready"
        };
        shell.event(prompt);
        if program == "/bin/zsh" {
            shell.input("autoload -Uz compinit; compinit -D", 0, "\r");
            shell.event("command-end");
            shell.event(prompt);
        }
        shell.input("cd stream-", 1, "\x1b[97~");
        let first = shell.event("completions");
        assert_eq!(
            first["data"]["items"].as_array().unwrap().len(),
            8,
            "{program}: {first}"
        );
        assert_eq!(first["data"]["append"], false);
        assert_eq!(first["data"]["more"], true);
        let mut count = 8;
        loop {
            let batch = shell.event("completions");
            assert_eq!(batch["data"]["append"], true);
            count += batch["data"]["items"].as_array().unwrap().len();
            if batch["data"]["more"] == false {
                break;
            }
        }
        assert_eq!(count, 45, "{program}");
        for index in 0..12 {
            let text = format!("echo stream-{index:02}");
            if shell.query_only {
                shell.write(format!("{text}\r").as_bytes());
            } else {
                shell.input(&text, index + 2, "\r");
            }
            shell.event("command-end");
            shell.event(prompt);
        }
        shell.input("echo stream-", 20, "\x1b[94~");
        let first = shell.event("completions");
        assert_eq!(
            first["data"]["items"].as_array().unwrap().len(),
            8,
            "{program}: {first}"
        );
        assert_eq!(first["data"]["more"], true);
        assert_eq!(first["data"]["items"][0]["text"], "echo stream-11");
        let last = shell.event("completions");
        assert_eq!(last["data"]["append"], true);
        assert_eq!(last["data"]["more"], false);
        assert_eq!(
            last["data"]["items"].as_array().unwrap().len(),
            4,
            "{program}: {last}"
        );
    }
}

#[test]
fn bash_and_fish_bridges_complete_directories_and_execute_exact_input() {
    for program in [
        "/bin/bash",
        "/opt/homebrew/bin/bash",
        "/usr/bin/bash",
        "/opt/homebrew/bin/fish",
        "/usr/bin/fish",
    ] {
        if !Path::new(program).exists() {
            continue;
        }
        let mut shell = ShellTest::new(program);
        let ready = shell.event("ready");
        shell.control_keys = ready["data"]["control_keys"] == true;
        let input_bridge = ready["data"]["input_bridge"] == true;
        shell.query_only = !input_bridge;
        shell.event(if input_bridge {
            "editor-ready"
        } else {
            "prompt-start"
        });
        shell.input("cd sub", 1, "\x1b[97~");
        let completions = shell.event("completions");
        assert!(
            completions["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["text"]
                    .as_str()
                    .is_some_and(|text| text.trim_end() == "cd subdir/")),
            "{program}: {completions}"
        );
        if input_bridge {
            shell.input("printf 'a  b 界\\n'", 2, "\r");
        } else {
            shell.write("printf 'a  b 界\\n'\r".as_bytes());
        }
        shell.event("command-start");
        assert_eq!(shell.event("command-end")["data"]["exit_code"], 0);
        shell.event(if input_bridge {
            "editor-ready"
        } else {
            "prompt-start"
        });
        shell.input("printf", 3, "\x1b[94~");
        let history = shell.event("completions");
        assert!(
            history["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["text"] == "printf 'a  b 界\\n'"),
            "{program}: {history}"
        );
    }
}

#[test]
fn powershell_bridge_completes_and_executes_unicode_input() {
    let Some(program) = env::var("TERMINASTE_TEST_PWSH")
        .ok()
        .or_else(|| command_exists("pwsh").then(|| "pwsh".to_owned()))
    else {
        return;
    };
    let mut shell = ShellTest::new(&program);
    let ready = shell.event("ready");
    assert_eq!(ready["data"]["input_bridge"], true);
    shell.control_keys = true;
    shell.event("editor-ready");
    shell.input("Get-Chil", 1, "\x1b[97~");
    let completions = shell.event("completions");
    assert!(
        completions["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["text"] == "Get-ChildItem"),
        "{completions}"
    );
    shell.input("Write-Output 'a  b 界'", 2, "\r");
    assert_eq!(
        shell.event("command-start")["data"]["command"],
        "Write-Output 'a  b 界'"
    );
    assert_eq!(shell.event("command-end")["data"]["exit_code"], 0);
    shell.event("editor-ready");
    shell.input("Write-Output", 3, "\x1b[94~");
    let history = shell.event("completions");
    assert!(
        history["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["text"] == "Write-Output 'a  b 界'"),
        "{history}"
    );
}

#[test]
fn elevated_and_remote_shells_bootstrap_and_return_to_the_parent() {
    if !Path::new("/bin/zsh").exists() {
        return;
    }
    let mut shell = ShellTest::new("/bin/zsh");
    shell.mock_shell_launchers();
    let parent = shell.event("ready")["shell_id"].clone();
    shell.event("editor-ready");
    shell.input("export PATH=\"$HOME:$PATH\"", 1, "\r");
    shell.event("command-end");
    shell.event("editor-ready");
    for (index, command) in [
        "sudo -s",
        "sudo -iu root",
        "su",
        "su - root",
        "ssh -p 2222 example",
    ]
    .into_iter()
    .enumerate()
    {
        shell.input(command, 2 + index as u64, "\r");
        shell.event("command-start");
        let child = shell.event("ready");
        assert_ne!(child["shell_id"], parent);
        assert_eq!(child["data"]["isolated"], true);
        assert_eq!(child["data"]["input_bridge"], true);
        shell.event("editor-ready");
        shell.input("cd sub", 1, "\x1b[97~");
        let completions = shell.event("completions");
        assert!(
            completions["data"]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["text"]
                    .as_str()
                    .is_some_and(|text| text.trim_end() == "cd subdir/")),
            "{command}: {completions}"
        );
        shell.input("exit", 2, "\r");
        let end = shell.event("command-end");
        assert_eq!(end["shell_id"], parent);
        assert_eq!(end["data"]["command"], command);
        shell.event("editor-ready");
    }
}

#[test]
fn shell_launchers_leave_commands_and_noninteractive_modes_untouched() {
    let wrappers = include_str!("shell_wrappers.sh");
    for shell in ["/bin/sh", "/bin/bash", "/bin/zsh"] {
        if !Path::new(shell).exists() {
            continue;
        }
        for (classifier, arguments, expected) in [
            ("__terminaste_sudo_shell", vec!["-iu", "root"], true),
            (
                "__terminaste_sudo_shell",
                vec!["-u", "some user", "-s"],
                true,
            ),
            (
                "__terminaste_sudo_shell",
                vec!["--login", "--user=root"],
                true,
            ),
            (
                "__terminaste_sudo_shell",
                vec!["-i", "printf", "%s", "a b"],
                false,
            ),
            ("__terminaste_sudo_shell", vec!["ls"], false),
            ("__terminaste_su_shell", vec!["-", "root"], true),
            ("__terminaste_su_shell", vec!["-m", "other"], true),
            (
                "__terminaste_su_shell",
                vec!["root", "-c", "printf 'a b'"],
                false,
            ),
            (
                "__terminaste_ssh_interactive",
                vec!["-vvJ", "jump", "-p2222", "host"],
                true,
            ),
            (
                "__terminaste_ssh_interactive",
                vec!["-o", "User=someone", "--", "host"],
                true,
            ),
            (
                "__terminaste_ssh_interactive",
                vec!["-NT", "-L8080:localhost:80", "host"],
                false,
            ),
            (
                "__terminaste_ssh_interactive",
                vec!["-oUser=someone", "host", "printf 'a b'"],
                false,
            ),
            (
                "__terminaste_ssh_interactive",
                vec!["-W", "host:22", "jump"],
                false,
            ),
            ("__terminaste_ssh_interactive", vec!["-n", "host"], false),
        ] {
            let status = std::process::Command::new(shell)
                .args([
                    "-c",
                    &format!("{wrappers}\n{classifier} \"$@\""),
                    "terminaste",
                ])
                .args(&arguments)
                .status()
                .unwrap();
            assert_eq!(
                status.success(),
                expected,
                "{shell}: {classifier} {arguments:?}"
            );
        }
    }
}
