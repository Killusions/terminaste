#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use terminaste_pty::{PtyCommand, PtyEvent};
    use terminaste_settings::{load_settings_from_path, Settings};
    use terminaste_ui::{integration_frame_for_tests, TerminasteApp};

    #[test]
    fn fake_pty_harness_covers_headless_app_flow() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        assert_eq!(app.tab_count(), 1);
        assert_eq!(app.active_pane_count(), 1);

        let (events, commands) = app.active_pane_fake_pty_for_tests().unwrap();
        app.submit_active_input_for_tests("echo ok");
        assert_eq!(
            recv_command(&commands),
            PtyCommand::Write(b"echo ok\r".to_vec())
        );

        events.send(PtyEvent::Output(b"ok\r\n".to_vec())).unwrap();
        app.drain_pty_events_for_tests();
        let snapshot = app.active_pane_snapshot_for_tests().unwrap();
        assert_eq!(snapshot.blocks.last().unwrap().output, "ok");

        app.resize_active_pane_for_tests(100, 30);
        assert_eq!(
            recv_command(&commands),
            PtyCommand::Resize {
                cols: 100,
                rows: 30
            }
        );
        assert_eq!(app.active_pane_snapshot_for_tests().unwrap().cols, 100);

        app.new_tab_for_tests();
        assert_eq!(app.tab_count(), 2);
        app.close_active_tab_for_tests();
        assert_eq!(app.tab_count(), 1);
        app.reopen_closed_tab_for_tests();
        assert_eq!(app.tab_count(), 2);

        app.split_active_for_tests();
        assert_eq!(app.active_pane_count(), 2);

        app.resize_active_pane_for_tests(80, 40);
        let before_rows = app.active_pane_snapshot_for_tests().unwrap().rows;
        app.set_font_size_for_tests(24.0);
        let after_rows = app.active_pane_snapshot_for_tests().unwrap().rows;
        assert_ne!(before_rows, after_rows);
    }

    #[test]
    fn invalid_settings_still_create_usable_headless_state() {
        let path = temp_settings_path();
        std::fs::write(
            &path,
            "[font]\nsize = 99\nline_height = 9\n[terminal]\nmax_grid_rows = 1\n",
        )
        .unwrap();
        let loaded = load_settings_from_path(path.clone());
        std::fs::remove_file(path).unwrap();

        assert!(!loaded.issues.is_empty());
        let app = TerminasteApp::headless_for_tests(loaded.settings);
        assert_eq!(app.tab_count(), 1);
        assert_eq!(app.active_pane_count(), 1);
        assert_eq!(app.active_pane_snapshot_for_tests().unwrap().rows, 24);
    }

    #[test]
    fn echo_temp_creates_command_block_and_separate_output_without_hanging() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let (_, commands) = app.active_pane_fake_pty_for_tests().unwrap();

        app.submit_active_input_for_tests("echo temp");
        assert_eq!(
            recv_command(&commands),
            PtyCommand::Write(b"echo temp\r".to_vec())
        );
        integrated_output(&mut app, "echo temp", b"temp\r\n");
        app.finish_active_command_for_tests(0);

        let snapshot = app.active_pane_snapshot_for_tests().unwrap();
        let block = snapshot.blocks.last().unwrap();
        assert_eq!(block.command, "echo temp");
        assert_eq!(block.output, "temp");
        assert_eq!(block.exit_code, Some(0));
        assert!(!block.running);
        assert_eq!(app.active_pane_editor_text_for_tests().unwrap(), "");
    }

    #[test]
    fn ls_output_does_not_show_previous_command_echo() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let (_, commands) = app.active_pane_fake_pty_for_tests().unwrap();

        app.submit_active_input_for_tests("echo temp");
        assert_eq!(
            recv_command(&commands),
            PtyCommand::Write(b"echo temp\r".to_vec())
        );
        integrated_output(&mut app, "echo temp", b"temp\r\n");
        app.finish_active_command_for_tests(0);

        app.submit_active_input_for_tests("ls");
        assert_eq!(recv_command(&commands), PtyCommand::Write(b"ls\r".to_vec()));
        integrated_output(&mut app, "ls", b"Cargo.toml\r\ncrates\r\n");
        app.finish_active_command_for_tests(0);

        let snapshot = app.active_pane_snapshot_for_tests().unwrap();
        assert_eq!(snapshot.blocks.len(), 2);
        let block = snapshot.blocks.last().unwrap();
        assert_eq!(block.command, "ls");
        assert_eq!(block.output, "Cargo.toml\ncrates");
        assert!(!block.output.contains("echo temp"));
        assert!(!block.output.contains("ls"));
    }

    #[test]
    fn oh_my_zsh_prompt_frame_bytes_never_appear_in_blocks() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        let mut bytes = Vec::new();
        bytes.extend(integration_frame_for_tests("prompt-start", "{}"));
        bytes.extend_from_slice(b"\x1b[7m%\x1b[27m \x1b[36m~/work\x1b[0m ");
        bytes.extend(integration_frame_for_tests("prompt-end", "{}"));
        bytes.extend(integration_frame_for_tests(
            "command-start",
            r#"{"command":"echo temp"}"#,
        ));
        bytes.extend_from_slice(b"temp\r\n");
        bytes.extend(integration_frame_for_tests(
            "command-end",
            r#"{"exit_code":0}"#,
        ));

        app.send_active_pane_output_for_tests(&bytes);

        let snapshot = app.active_pane_snapshot_for_tests().unwrap();
        assert_eq!(snapshot.blocks.len(), 1);
        let block = snapshot.blocks.last().unwrap();
        assert_eq!(block.command, "echo temp");
        assert_eq!(block.output, "temp");
        let block_text = format!("{}\n{}", block.command, block.output);
        assert!(!block_text.contains("~/work"));
        assert!(!block_text.contains('%'));
        assert!(!block_text.contains("\u{1b}"));
    }

    #[test]
    fn input_remains_fixed_bottom_state() {
        let mut app = TerminasteApp::headless_for_tests(Settings::default());
        app.submit_active_input_for_tests("echo temp");
        app.send_active_pane_output_for_tests(b"echo temp\r\ntemp\r\n");
        app.finish_active_command_for_tests(0);

        let layout = app.active_pane_layout_snapshot_for_tests().unwrap();
        assert!(layout.blocks_stick_to_bottom);
        assert!(layout.input_below_blocks);
        assert!(layout.input_fixed_to_bottom);
    }

    fn recv_command(commands: &crossbeam_channel::Receiver<PtyCommand>) -> PtyCommand {
        commands.recv_timeout(Duration::from_secs(1)).unwrap()
    }

    fn integrated_output(app: &mut TerminasteApp, command: &str, output: &[u8]) {
        app.send_active_pane_output_for_tests(format!("{command}\r\n").as_bytes());
        app.send_active_pane_output_for_tests(&integration_frame_for_tests(
            "command-start",
            &format!("{{\"command\":\"{command}\"}}"),
        ));
        app.send_active_pane_output_for_tests(output);
    }

    fn temp_settings_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "terminaste-invalid-settings-{}.toml",
            std::process::id()
        ))
    }
}
