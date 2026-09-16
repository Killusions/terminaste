use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use base64::prelude::{Engine, BASE64_STANDARD, BASE64_URL_SAFE_NO_PAD};
use portable_pty::CommandBuilder;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellFamily {
    Zsh,
    Bash,
    Fish,
    PowerShell,
    Unknown,
}

impl ShellFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::Fish => "fish",
            Self::PowerShell => "powershell",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellStartup {
    pub family: ShellFamily,
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IntegrationEnvelope {
    pub session: String,
    pub sequence: u64,
    pub time_ms: u64,
    pub data: Value,
}

pub fn detect_shell_family(shell: &str) -> ShellFamily {
    let name = Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(shell)
        .to_ascii_lowercase();
    let name = name.strip_suffix(".exe").unwrap_or(&name);

    match name {
        "zsh" => ShellFamily::Zsh,
        "bash" => ShellFamily::Bash,
        "fish" => ShellFamily::Fish,
        "pwsh" | "powershell" => ShellFamily::PowerShell,
        _ => ShellFamily::Unknown,
    }
}

pub fn encode_integration_frame(
    session: &str,
    sequence: u64,
    event_name: &str,
    data: Value,
) -> anyhow::Result<String> {
    let envelope = IntegrationEnvelope {
        session: session.to_owned(),
        sequence,
        time_ms: current_time_ms(),
        data,
    };
    let json = serde_json::to_vec(&envelope)?;
    let payload = BASE64_URL_SAFE_NO_PAD.encode(json);
    Ok(format!(
        "\x1bPterminaste;1;{event_name};json64;{payload}\x1b\\"
    ))
}

pub fn shell_startup(shell: &str, session: &str) -> anyhow::Result<ShellStartup> {
    let family = detect_shell_family(shell);
    let program = shell.to_owned();
    let temp_dir = integration_temp_dir(session);
    fs::create_dir_all(&temp_dir).with_context(|| {
        format!(
            "failed to create shell integration directory {}",
            temp_dir.display()
        )
    })?;
    write_zsh_startup_files(&temp_dir)?;
    fs::write(temp_dir.join("bashrc"), bash_script())?;
    fs::write(temp_dir.join("fish"), fish_script())?;
    fs::write(temp_dir.join("powershell.ps1"), powershell_script())?;

    let mut startup = match family {
        ShellFamily::Zsh => ShellStartup {
            family,
            program,
            args: vec!["-i".to_owned(), "-l".to_owned()],
            env: vec![
                ("ZDOTDIR".to_owned(), temp_dir.display().to_string()),
                (
                    "TERMINASTE_INTEGRATION_DIR".to_owned(),
                    temp_dir.display().to_string(),
                ),
                (
                    "TERMINASTE_ORIGINAL_ZDOTDIR".to_owned(),
                    std::env::var("ZDOTDIR")
                        .ok()
                        .filter(|path| !path.contains("/terminaste-shell-integration"))
                        .unwrap_or_default(),
                ),
                (
                    "TERMINASTE_SHELL_FAMILY".to_owned(),
                    family.as_str().to_owned(),
                ),
            ],
        },
        ShellFamily::Bash => {
            let rcfile = temp_dir.join("bashrc");
            ShellStartup {
                family,
                program,
                args: vec![
                    "--rcfile".to_owned(),
                    rcfile.display().to_string(),
                    "-i".to_owned(),
                ],
                env: vec![
                    (
                        "TERMINASTE_SHELL_FAMILY".to_owned(),
                        family.as_str().to_owned(),
                    ),
                    (
                        "TERMINASTE_INTEGRATION_DIR".to_owned(),
                        temp_dir.display().to_string(),
                    ),
                ],
            }
        }
        ShellFamily::Fish => ShellStartup {
            family,
            program,
            args: vec!["-C".to_owned(), fish_script()],
            env: vec![
                (
                    "TERMINASTE_SHELL_FAMILY".to_owned(),
                    family.as_str().to_owned(),
                ),
                (
                    "TERMINASTE_INTEGRATION_DIR".to_owned(),
                    temp_dir.display().to_string(),
                ),
            ],
        },
        ShellFamily::PowerShell => ShellStartup {
            family,
            program,
            args: vec![
                "-NoLogo".to_owned(),
                "-NoExit".to_owned(),
                "-Command".to_owned(),
                powershell_script(),
            ],
            env: vec![(
                "TERMINASTE_SHELL_FAMILY".to_owned(),
                family.as_str().to_owned(),
            )],
        },
        ShellFamily::Unknown => ShellStartup {
            family,
            program,
            args: Vec::new(),
            env: vec![(
                "TERMINASTE_SHELL_FAMILY".to_owned(),
                family.as_str().to_owned(),
            )],
        },
    };

    startup.env.push((
        "TERMINASTE_BOOTSTRAP".to_owned(),
        BASE64_STANDARD.encode(shell_bootstrap()),
    ));
    startup.env.push((
        "TERMINASTE_WRAPPERS".to_owned(),
        include_str!("shell_wrappers.sh").to_owned(),
    ));
    Ok(startup)
}

pub fn command_builder_for_shell(shell: &str, session: &str) -> anyhow::Result<CommandBuilder> {
    let startup = shell_startup(shell, session)?;
    let mut command = CommandBuilder::from_argv(
        std::iter::once(OsString::from(&startup.program))
            .chain(startup.args.iter().map(OsString::from))
            .collect(),
    );
    for (key, value) in startup.env {
        command.env(key, value);
    }
    Ok(command)
}

pub fn generated_script(family: ShellFamily) -> String {
    match family {
        ShellFamily::Zsh => zsh_script(),
        ShellFamily::Bash => bash_script(),
        ShellFamily::Fish => fish_script(),
        ShellFamily::PowerShell => powershell_script(),
        ShellFamily::Unknown => String::new(),
    }
}

fn write_zsh_startup_files(temp_dir: &Path) -> anyhow::Result<()> {
    for (name, contents) in zsh_startup_files() {
        fs::write(temp_dir.join(name), contents)?;
    }
    Ok(())
}

fn zsh_startup_files() -> Vec<(&'static str, String)> {
    let mut files = vec![(
        ".zshenv",
        r#"typeset -g __TERMINASTE_USER_ZDOTDIR="${TERMINASTE_ORIGINAL_ZDOTDIR:-$HOME}"
if [[ "$__TERMINASTE_USER_ZDOTDIR" == "$TERMINASTE_INTEGRATION_DIR" ]]; then
  __TERMINASTE_USER_ZDOTDIR="$HOME"
fi
if [[ "$__TERMINASTE_USER_ZDOTDIR" == *"/terminaste-shell-integration"* ]]; then
  __TERMINASTE_USER_ZDOTDIR="$HOME"
fi
ZDOTDIR="$__TERMINASTE_USER_ZDOTDIR"
if [[ -r "$__TERMINASTE_USER_ZDOTDIR/.zshenv" && "$__TERMINASTE_USER_ZDOTDIR/.zshenv" != "$TERMINASTE_INTEGRATION_DIR/.zshenv" ]]; then
  source "$__TERMINASTE_USER_ZDOTDIR/.zshenv"
fi
if [[ -n "${ZDOTDIR:-}" && "$ZDOTDIR" != "$TERMINASTE_INTEGRATION_DIR" ]]; then
  __TERMINASTE_USER_ZDOTDIR="$ZDOTDIR"
fi
ZDOTDIR="$TERMINASTE_INTEGRATION_DIR"
"#
        .to_owned(),
    )];
    for file in [".zprofile", ".zshrc"] {
        let history_setup = if file == ".zshrc" {
            r#"if [[ "$HISTFILE" == "$TERMINASTE_INTEGRATION_DIR/.zsh_history" ]]; then
  HISTFILE="$__TERMINASTE_USER_ZDOTDIR/.zsh_history"
fi
"#
        } else {
            ""
        };
        let integration = if file == ".zshrc" {
            zsh_script()
        } else {
            String::new()
        };
        files.push((
            file,
            format!(
                r#"ZDOTDIR="$__TERMINASTE_USER_ZDOTDIR"
{history_setup}if [[ -r "$ZDOTDIR/{file}" ]]; then
  source "$ZDOTDIR/{file}"
fi
__TERMINASTE_USER_ZDOTDIR="${{ZDOTDIR:-$HOME}}"
ZDOTDIR="$TERMINASTE_INTEGRATION_DIR"
{integration}
"#
            ),
        ));
    }
    files.push((
        ".zlogin",
        format!(
            r#"ZDOTDIR="$__TERMINASTE_USER_ZDOTDIR"
if [[ -r "$ZDOTDIR/.zlogin" ]]; then
  source "$ZDOTDIR/.zlogin"
fi
ZDOTDIR="$TERMINASTE_INTEGRATION_DIR"

{}
"#,
            zsh_script()
        ),
    ));
    files
}

fn zsh_script() -> String {
    format!(
        "{}\n{}",
        zsh_integration_script(),
        include_str!("shell_wrappers.sh")
    )
}

fn zsh_integration_script() -> &'static str {
    r#"if [[ -n "${TERMINASTE_SESSION:-}" && -z "${__TERMINASTE_INTEGRATION_LOADED:-}" ]]; then
  typeset -g __TERMINASTE_INTEGRATION_LOADED=1
  typeset -g __TERMINASTE_SEQUENCE=0
  typeset -g __TERMINASTE_SHELL_ID="$$-$RANDOM-$RANDOM"
  typeset -g __TERMINASTE_ACTIVE_COMMAND=""
  typeset -g __TERMINASTE_LAST_PWD="$PWD"
  typeset -g __TERMINASTE_COMMAND_START_MS=0
  PROMPT_EOL_MARK=''

  zmodload zsh/datetime 2>/dev/null || true

  __terminaste_now_ms() {
    if [[ -n "${EPOCHREALTIME:-}" ]]; then
      printf '%.0f' $(( EPOCHREALTIME * 1000 ))
    else
      printf '%s000' "${EPOCHSECONDS:-0}"
    fi
  }

  __terminaste_escape_json_value() {
    local value="$1"
    value=${value//\\/\\\\}
    value=${value//\"/\\\"}
    value=${value//$'\n'/\\n}
    value=${value//$'\r'/\\r}
    value=${value//$'\t'/\\t}
    value=${value//$'\e'/\\u001b}
    value=${value//$'\a'/\\u0007}
    value=${value//$'\b'/\\b}
    value=${value//$'\f'/\\f}
    value=${value//$'\v'/\\u000b}
    REPLY="$value"
  }

  __terminaste_json_escape() {
    local REPLY
    __terminaste_escape_json_value "$1"
    print -r -- "$REPLY"
  }

  __terminaste_b64url() {
    command base64 | command tr -d '\n=' | command tr '+/' '-_'
  }

  __terminaste_emit() {
    local event="$1"
    local data="$2"
    local session="$(__terminaste_json_escape "$TERMINASTE_SESSION")"
    local now="$(__terminaste_now_ms)"
    __TERMINASTE_SEQUENCE=$(( __TERMINASTE_SEQUENCE + 1 ))
    local json="{\"session\":\"$session\",\"shell_id\":\"$__TERMINASTE_SHELL_ID\",\"sequence\":$__TERMINASTE_SEQUENCE,\"time_ms\":$now,\"data\":{$data}}"
    local payload="$(printf '%s' "$json" | __terminaste_b64url)"
    printf '\033Pterminaste;1;%s;json64;%s\033\\' "$event" "$payload"
  }

  __terminaste_data_pwd() {
    printf '"current_directory":"%s"' "$(__terminaste_json_escape "$PWD")"
  }

  __terminaste_preexec() {
    local command_text="$1"
    __TERMINASTE_ACTIVE_COMMAND="$command_text"
    __TERMINASTE_COMMAND_START_MS="$(__terminaste_now_ms)"
    __terminaste_emit prompt-end "$(__terminaste_data_pwd)"
    __terminaste_emit command-start "\"command\":\"$(__terminaste_json_escape "$command_text")\",$(__terminaste_data_pwd)"
  }

  __terminaste_precmd() {
    local exit_code="$?"
    if [[ -n "$__TERMINASTE_ACTIVE_COMMAND" ]]; then
      local finished_ms="$(__terminaste_now_ms)"
      local duration_ms=$(( finished_ms - __TERMINASTE_COMMAND_START_MS ))
      (( duration_ms < 0 )) && duration_ms=0
      __terminaste_emit command-end "\"command\":\"$(__terminaste_json_escape "$__TERMINASTE_ACTIVE_COMMAND")\",\"exit_code\":$exit_code,\"duration_ms\":$duration_ms,$(__terminaste_data_pwd)"
      __TERMINASTE_ACTIVE_COMMAND=""
    fi
    if [[ "$PWD" != "$__TERMINASTE_LAST_PWD" ]]; then
      __TERMINASTE_LAST_PWD="$PWD"
      __terminaste_emit directory-change "$(__terminaste_data_pwd)"
    fi
    __terminaste_emit prompt-start "$(__terminaste_data_pwd)"
  }

  __terminaste_zle_line_init() {
    __terminaste_emit prompt-end "$(__terminaste_data_pwd)"
    __terminaste_emit editor-ready "\"revision\":$__TERMINASTE_INPUT_REVISION"
    __terminaste_report_input
  }

  typeset -g __TERMINASTE_INPUT_REVISION=0
  __terminaste_report_input() {
    (( ${__terminaste_collecting:-0} )) && return
    local rendered_prompt="$PROMPT" rendered_rprompt="$RPROMPT"
    if [[ -o promptsubst ]]; then
      rendered_prompt="${(e)rendered_prompt}"
      rendered_rprompt="${(e)rendered_rprompt}"
    fi
    rendered_prompt="${(%)rendered_prompt}"
    rendered_rprompt="${(%)rendered_rprompt}"
    __terminaste_emit input-buffer "\"text\":\"$(__terminaste_json_escape "$BUFFER")\",\"cursor\":$CURSOR,\"revision\":$__TERMINASTE_INPUT_REVISION,\"columns\":$COLUMNS,\"keymap\":\"$KEYMAP\",\"prompt\":\"$(__terminaste_json_escape "$rendered_prompt")\",\"right_prompt\":\"$(__terminaste_json_escape "$rendered_rprompt")\""
  }

  __terminaste_replace_input() {
    emulate -L zsh
    setopt extendedglob
    local header payload length encoded value
    read -r -k 8 -t 1 header || return
    [[ "$header" == [0-9a-f]## ]] || return
    length=$(( 16#$header ))
    (( length > 0 && length <= 65536 )) || return
    read -r -k "$length" -t 1 payload || return
    local cursor="${payload%%;*}"
    payload="${payload#*;}"
    local revision="${payload%%;*}"
    encoded="${payload#*;}"
    [[ "$cursor" == <-> && "$revision" == <-> ]] || return
    value="$(printf '%s' "$encoded" | command base64 -d; printf '.')"
    BUFFER="${value%.}"
    CURSOR="$cursor"
    __TERMINASTE_INPUT_REVISION="$revision"
    zle -R
  }

  zle -N __terminaste_replace_input
  zle -N __terminaste_report_input
  __terminaste_completion_state() {
    unset MENUSELECT MENUMODE
    __terminaste_match_count=$compstate[nmatches]
    compstate[list]=''
    if (( __terminaste_match_count > 1 )); then
      compstate[insert]='menu:1'
    fi
  }

  __terminaste_complete() {
    local original="$BUFFER" original_cursor=$CURSOR
    local MENUSELECT MENUMODE
    local -i __terminaste_collecting=1 __terminaste_match_count=0 index
    local -a comppostfuncs=("${comppostfuncs[@]}" __terminaste_completion_state)
    local binding="$(bindkey -M "$KEYMAP" '^I')"
    local -a binding_parts=( ${(z)binding} )
    zle "${binding_parts[2]}"
    local items='' separator='' first="$BUFFER" REPLY
    if (( __terminaste_match_count > 1 )); then
      for (( index=1; index<=__terminaste_match_count; index++ )); do
        __terminaste_escape_json_value "$BUFFER"
        items+="$separator{\"text\":\"$REPLY\",\"cursor\":$CURSOR}"
        separator=','
        zle .menu-complete -n 1
        [[ "$BUFFER" == "$first" ]] && break
      done
      BUFFER="$original"
      CURSOR=$original_cursor
    elif [[ "$BUFFER" != "$original" ]]; then
      __terminaste_escape_json_value "$BUFFER"
      items="{\"text\":\"$REPLY\",\"cursor\":$CURSOR}"
      BUFFER="$original"
      CURSOR=$original_cursor
    fi
    __terminaste_emit completions "\"revision\":$__TERMINASTE_INPUT_REVISION,\"text\":\"$(__terminaste_json_escape "$original")\",\"items\":[$items]"
    zle -R
  }

  zle -N __terminaste_complete
  __terminaste_history_options() {
    emulate -L zsh
    zmodload zsh/parameter
    local original="$BUFFER" items='' separator='' entry text REPLY
    local prefix="$original"
    [[ "$prefix" == *$'\n'* ]] && prefix=''
    local -A seen
    local -i count=0
    for entry in ${(Onk)history}; do
      text="${history[$entry]}"
      [[ "$text" == "$prefix"* && -n "$text" && -z "${seen[$text]-}" ]] || continue
      seen[$text]=1
      __terminaste_escape_json_value "$text"
      items+="$separator{\"text\":\"$REPLY\",\"cursor\":${#text}}"
      separator=','
      (( ++count >= 2000 )) && break
    done
    __terminaste_emit completions "\"revision\":$__TERMINASTE_INPUT_REVISION,\"text\":\"$(__terminaste_json_escape "$original")\",\"items\":[$items]"
  }
  zle -N __terminaste_history_options
  __terminaste_history() {
    local original_cursor=$CURSOR
    if [[ "$LASTWIDGET" != __terminaste_history_* && "$LASTWIDGET" != __terminaste_report_input ]]; then
      typeset -g __TERMINASTE_HISTORY_CURSOR=$CURSOR
    fi
    CURSOR=${__TERMINASTE_HISTORY_CURSOR:-$CURSOR}
    if [[ "$WIDGET" == __terminaste_history_previous ]]; then
      zle .history-beginning-search-backward
    else
      zle .history-beginning-search-forward
    fi
    if (( $? == 0 )); then
      CURSOR=${#BUFFER}
    else
      CURSOR=$original_cursor
    fi
  }
  zle -N __terminaste_history_previous __terminaste_history
  zle -N __terminaste_history_next __terminaste_history
  for __terminaste_keymap in emacs viins vicmd; do
    bindkey -M "$__terminaste_keymap" $'\e[99~' __terminaste_replace_input
    bindkey -M "$__terminaste_keymap" $'\e[98~' __terminaste_report_input
    bindkey -M "$__terminaste_keymap" $'\e[97~' __terminaste_complete
    bindkey -M "$__terminaste_keymap" $'\e[96~' __terminaste_history_previous
    bindkey -M "$__terminaste_keymap" $'\e[95~' __terminaste_history_next
    bindkey -M "$__terminaste_keymap" $'\e[94~' __terminaste_history_options
  done

  if [[ -r "$TERMINASTE_INTEGRATION_DIR/bashrc" ]]; then
    bash() { command bash --rcfile "$TERMINASTE_INTEGRATION_DIR/bashrc" "$@"; }
  fi
  if [[ -r "$TERMINASTE_INTEGRATION_DIR/fish" ]]; then
    fish() { command fish -C "source $TERMINASTE_INTEGRATION_DIR/fish" "$@"; }
  fi

  autoload -Uz add-zsh-hook
  add-zsh-hook preexec __terminaste_preexec
  add-zsh-hook precmd __terminaste_precmd
  precmd_functions=(__terminaste_precmd ${precmd_functions:#__terminaste_precmd})
  autoload -Uz add-zle-hook-widget 2>/dev/null || true
  if whence add-zle-hook-widget >/dev/null 2>&1; then
    zle -N __terminaste_zle_line_init
    add-zle-hook-widget line-init __terminaste_zle_line_init
    add-zle-hook-widget line-pre-redraw __terminaste_report_input
    add-zle-hook-widget isearch-update __terminaste_report_input
  fi

  __terminaste_emit ready "\"shell_family\":\"zsh\",\"input_bridge\":true,\"isolated\":${TERMINASTE_ISOLATED:-false},$(__terminaste_data_pwd)"
  __terminaste_emit directory-change "$(__terminaste_data_pwd)"
fi
"#
}

fn bash_script() -> String {
    format!(
        "{}\n{}",
        bash_integration_script(),
        include_str!("shell_wrappers.sh")
    )
}

fn bash_integration_script() -> String {
    r#"if [[ -r "$HOME/.bashrc" && -z "${__TERMINASTE_BASHRC_SOURCED:-}" ]]; then
  __TERMINASTE_BASHRC_SOURCED=1
  source "$HOME/.bashrc"
fi

if [[ -n "${TERMINASTE_SESSION:-}" && -z "${__TERMINASTE_INTEGRATION_LOADED:-}" ]]; then
  __TERMINASTE_INTEGRATION_LOADED=1
  __TERMINASTE_SEQUENCE=0
  __TERMINASTE_SHELL_ID="$$-$RANDOM-$RANDOM"
  __TERMINASTE_ACTIVE_COMMAND=""
  __TERMINASTE_LAST_PWD="$PWD"
  __TERMINASTE_COMMAND_START_MS=0
  __TERMINASTE_INPUT_REVISION=0

  __terminaste_now_ms() { printf '%s000' "$(date +%s)"; }

  __terminaste_json_escape() {
    local value="$1"
    value=${value//\\/\\\\}
    value=${value//\"/\\\"}
    value=${value//$'\n'/\\n}
    value=${value//$'\r'/\\r}
    value=${value//$'\t'/\\t}
    printf '%s' "$value"
  }

  __terminaste_b64url() {
    command base64 | command tr -d '\n=' | command tr '+/' '-_'
  }

  __terminaste_emit() {
    local event="$1"
    local data="$2"
    local session="$(__terminaste_json_escape "$TERMINASTE_SESSION")"
    local now="$(__terminaste_now_ms)"
    __TERMINASTE_SEQUENCE=$(( __TERMINASTE_SEQUENCE + 1 ))
    local json="{\"session\":\"$session\",\"shell_id\":\"$__TERMINASTE_SHELL_ID\",\"sequence\":$__TERMINASTE_SEQUENCE,\"time_ms\":$now,\"data\":{$data}}"
    local payload
    payload="$(printf '%s' "$json" | __terminaste_b64url)"
    printf '\033Pterminaste;1;%s;json64;%s\033\\' "$event" "$payload"
  }

  __terminaste_data_pwd() { printf '"current_directory":"%s"' "$(__terminaste_json_escape "$PWD")"; }

  __terminaste_report_input() {
    local prefix="$(LC_ALL=C; printf %s "${READLINE_LINE:0:READLINE_POINT}")"
    __terminaste_emit input-buffer "\"text\":\"$(__terminaste_json_escape "${READLINE_LINE:-}")\",\"cursor\":${#prefix},\"revision\":$__TERMINASTE_INPUT_REVISION"
  }

  __terminaste_begin_input() {
    READLINE_LINE=''
    READLINE_POINT=0
  }

  __terminaste_query() {
    local value
    value="$(printf %s "$3" | base64 -d; printf '.')"
    READLINE_LINE="${value%.}"
    READLINE_POINT="$1"
    __TERMINASTE_INPUT_REVISION="$2"
    if [[ "$4" == history ]]; then __terminaste_history_options; else __terminaste_complete; fi
  }

  __terminaste_finish_input() {
    local framed="$READLINE_LINE" header payload length cursor revision encoded value
    header="${framed:0:8}"
    payload="${framed:8}"
    if [[ ! "$header" =~ ^[0-9a-f]+$ ]]; then
      __terminaste_report_input
      return
    fi
    length=$(( 16#$header ))
    if (( length <= 0 || length > 65536 || ${#payload} != length )); then
      __terminaste_report_input
      return
    fi
    cursor="${payload%%;*}"
    payload="${payload#*;}"
    revision="${payload%%;*}"
    encoded="${payload#*;}"
    [[ "$cursor" =~ ^[0-9]+$ && "$revision" =~ ^[0-9]+$ ]] || return
    value="$(printf '%s' "$encoded" | command base64 -d; printf '.')"
    READLINE_LINE="${value%.}"
    local prefix="${READLINE_LINE:0:cursor}"
    READLINE_POINT="$(LC_ALL=C; printf %s "${#prefix}")"
    __TERMINASTE_INPUT_REVISION="$revision"
    __terminaste_report_input
  }

  __terminaste_complete() {
    local original="$READLINE_LINE" before token command_name candidate escaped replacement REPLY
    local -i original_point start cursor
    local items='' separator='' suffix=''
    before="$(LC_ALL=C; printf %s "${READLINE_LINE:0:READLINE_POINT}")"
    original_point=${#before}
    token="${before##*[[:space:]]}"
    start=$(( original_point - ${#token} ))
    command_name="${before%%[[:space:]]*}"
    local -a candidates=()
    if [[ "$command_name" == cd ]]; then
      while IFS= read -r candidate; do candidates+=("$candidate"); done < <(compgen -d -- "$token")
    elif (( start == 0 )); then
      while IFS= read -r candidate; do candidates+=("$candidate"); done < <(compgen -c -- "$token")
    else
      while IFS= read -r candidate; do candidates+=("$candidate"); done < <(compgen -f -- "$token")
    fi
    for candidate in "${candidates[@]:0:200}"; do
      printf -v escaped '%q' "$candidate"
      suffix=''
      [[ -d "$candidate" ]] && suffix='/'
      replacement="${original:0:start}${escaped}${suffix}${original:original_point}"
      cursor=$(( start + ${#escaped} + ${#suffix} ))
      REPLY="$(__terminaste_json_escape "$replacement")"
      items+="$separator{\"text\":\"$REPLY\",\"cursor\":$cursor}"
      separator=','
    done
    __terminaste_emit completions "\"revision\":$__TERMINASTE_INPUT_REVISION,\"text\":\"$(__terminaste_json_escape "$original")\",\"items\":[$items]"
  }

  __terminaste_history_options() {
    local original="$READLINE_LINE" line text REPLY items='' separator=''
    local -a entries=()
    local -i index count=0
    while IFS= read -r line; do entries+=("$line"); done < <(HISTTIMEFORMAT= builtin history)
    for (( index=${#entries[@]} - 1; index >= 0; index-- )); do
      line="${entries[index]}"
      [[ "$line" =~ ^[[:space:]]*[0-9]+[[:space:]]+(.*)$ ]] || continue
      text="${BASH_REMATCH[1]}"
      [[ "$text" == "$original"* && -n "$text" ]] || continue
      REPLY="$(__terminaste_json_escape "$text")"
      items+="$separator{\"text\":\"$REPLY\",\"cursor\":${#text}}"
      separator=','
      (( ++count >= 2000 )) && break
    done
    __terminaste_emit completions "\"revision\":$__TERMINASTE_INPUT_REVISION,\"text\":\"$(__terminaste_json_escape "$original")\",\"items\":[$items]"
  }

  __terminaste_debug_trap() {
    local command_text="$BASH_COMMAND"
    [[ -n "$__TERMINASTE_ACTIVE_COMMAND" ]] && return
    [[ "$command_text" == __terminaste_* ]] && return
    [[ "$command_text" == "$PROMPT_COMMAND" ]] && return
    [[ "$command_text" == trap*DEBUG* ]] && return
    __TERMINASTE_ACTIVE_COMMAND="$command_text"
    __TERMINASTE_COMMAND_START_MS="$(__terminaste_now_ms)"
    __terminaste_emit prompt-end "$(__terminaste_data_pwd)"
    __terminaste_emit command-start "\"command\":\"$(__terminaste_json_escape "$command_text")\",$(__terminaste_data_pwd)"
  }

  __terminaste_prompt_command() {
    local exit_code="$?"
    if [[ -n "$__TERMINASTE_ACTIVE_COMMAND" ]]; then
      local finished_ms="$(__terminaste_now_ms)"
      local duration_ms=$(( finished_ms - __TERMINASTE_COMMAND_START_MS ))
      (( duration_ms < 0 )) && duration_ms=0
      __terminaste_emit command-end "\"command\":\"$(__terminaste_json_escape "$__TERMINASTE_ACTIVE_COMMAND")\",\"exit_code\":$exit_code,\"duration_ms\":$duration_ms,$(__terminaste_data_pwd)"
      __TERMINASTE_ACTIVE_COMMAND=""
    fi
    if [[ "$PWD" != "$__TERMINASTE_LAST_PWD" ]]; then
      __TERMINASTE_LAST_PWD="$PWD"
      __terminaste_emit directory-change "$(__terminaste_data_pwd)"
    fi
    __terminaste_emit prompt-start "$(__terminaste_data_pwd)"
    if [[ "$__TERMINASTE_INPUT_BRIDGE" == true ]]; then
      __terminaste_emit editor-ready "\"revision\":$__TERMINASTE_INPUT_REVISION"
      READLINE_LINE=''
      READLINE_POINT=0
      __terminaste_report_input
    fi
  }

  if declare -p PROMPT_COMMAND 2>/dev/null | command grep -q '^declare \-a'; then
    PROMPT_COMMAND=(__terminaste_prompt_command "${PROMPT_COMMAND[@]}")
  else
    PROMPT_COMMAND="__terminaste_prompt_command${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
  fi

  if [[ -r "$TERMINASTE_INTEGRATION_DIR/.zshrc" ]]; then
    zsh() { ZDOTDIR="$TERMINASTE_INTEGRATION_DIR" command zsh "$@"; }
  fi
  if [[ -r "$TERMINASTE_INTEGRATION_DIR/fish" ]]; then
    fish() { command fish -C "source '$TERMINASTE_INTEGRATION_DIR/fish'" "$@"; }
  fi

  __TERMINASTE_INPUT_BRIDGE=false
  if (( BASH_VERSINFO[0] >= 4 )); then
    bind -x '"\e[99~":__terminaste_begin_input'
    bind -x '"\e[98~":__terminaste_finish_input'
    bind -x '"\e[97~":__terminaste_complete'
    bind -x '"\e[94~":__terminaste_history_options'
    bind '"\e[96~":history-search-backward'
    bind '"\e[95~":history-search-forward'
    __TERMINASTE_INPUT_BRIDGE=true
  fi

  if [[ "$__TERMINASTE_INPUT_BRIDGE" == false ]]; then
    HISTIGNORE="${HISTIGNORE:+$HISTIGNORE:}__terminaste_query *"
  fi
  __terminaste_emit ready "\"shell_family\":\"bash\",\"input_bridge\":$__TERMINASTE_INPUT_BRIDGE,\"query_bridge\":true,\"isolated\":${TERMINASTE_ISOLATED:-false},$(__terminaste_data_pwd)"
  __terminaste_emit directory-change "$(__terminaste_data_pwd)"
  trap '__terminaste_debug_trap' DEBUG
fi
"#
    .to_owned()
}

fn fish_script() -> String {
    r#"if status is-interactive; and test -n "$TERMINASTE_SESSION"; and not set -q __TERMINASTE_INTEGRATION_LOADED
  set -g __TERMINASTE_INTEGRATION_LOADED 1
  set -g __TERMINASTE_SEQUENCE 0
  set -g __TERMINASTE_SHELL_ID "$fish_pid-"(random)"-"(random)
  set -g __TERMINASTE_ACTIVE_COMMAND ""
  set -g __TERMINASTE_LAST_PWD "$PWD"
  set -g __TERMINASTE_COMMAND_START_MS 0

  function __terminaste_now_ms
    math (date +%s) \* 1000
  end

  function __terminaste_json_escape
    set -l value (string replace -a -- \\ \\\\ "$argv[1]" | string collect --allow-empty)
    set value (string replace -a -- '"' '\\"' "$value" | string collect --allow-empty)
    set value (string replace -a -- \n '\\n' "$value" | string collect --allow-empty)
    set value (string replace -a -- \r '\\r' "$value" | string collect --allow-empty)
    set value (string replace -a -- \t '\\t' "$value" | string collect --allow-empty)
    set value (string replace -a -- \e '\\u001b' "$value" | string collect --allow-empty)
    printf %s "$value"
  end

  function __terminaste_b64url
    if base64 --help 2>&1 | string match -q '*-w*'
      base64 -w 0
    else
      base64
    end | string replace -a '\n' '' | string replace -a '=' '' | string replace -a '+' '-' | string replace -a '/' '_'
  end

  function __terminaste_emit
    set event $argv[1]
    set data $argv[2]
    set session (__terminaste_json_escape "$TERMINASTE_SESSION")
    set now (__terminaste_now_ms)
    set -g __TERMINASTE_SEQUENCE (math $__TERMINASTE_SEQUENCE + 1)
    set json "{\"session\":\"$session\",\"shell_id\":\"$__TERMINASTE_SHELL_ID\",\"sequence\":$__TERMINASTE_SEQUENCE,\"time_ms\":$now,\"data\":{$data}}"
    set payload (printf '%s' "$json" | __terminaste_b64url)
    printf '\033Pterminaste;1;%s;json64;%s\033\\' "$event" "$payload"
  end

  function __terminaste_data_pwd
    printf '"current_directory":"%s"' (__terminaste_json_escape "$PWD")
  end

  function __terminaste_preexec --on-event fish_preexec
    set -g __TERMINASTE_ACTIVE_COMMAND "$argv"
    set -g __TERMINASTE_COMMAND_START_MS (__terminaste_now_ms)
    set escaped_command (__terminaste_json_escape "$argv")
    __terminaste_emit prompt-end (__terminaste_data_pwd)
    __terminaste_emit command-start "\"command\":\"$escaped_command\","(__terminaste_data_pwd)
  end

  function __terminaste_postexec --on-event fish_postexec
    set exit_code $status
    if test -n "$__TERMINASTE_ACTIVE_COMMAND"
      set finished_ms (__terminaste_now_ms)
      set duration_ms (math max 0, $finished_ms - $__TERMINASTE_COMMAND_START_MS)
      set escaped_command (__terminaste_json_escape "$__TERMINASTE_ACTIVE_COMMAND")
      __terminaste_emit command-end "\"command\":\"$escaped_command\",\"exit_code\":$exit_code,\"duration_ms\":$duration_ms,"(__terminaste_data_pwd)
      set -g __TERMINASTE_ACTIVE_COMMAND ""
    end
  end

  function __terminaste_pwd_changed --on-variable PWD
    set -g __TERMINASTE_LAST_PWD "$PWD"
    __terminaste_emit directory-change (__terminaste_data_pwd)
  end

  function __terminaste_prompt --on-event fish_prompt
    __terminaste_emit prompt-start (__terminaste_data_pwd)
    __terminaste_emit prompt-end (__terminaste_data_pwd)
  end

  if test -r "$TERMINASTE_INTEGRATION_DIR/.zshrc"
    function zsh
      env ZDOTDIR="$TERMINASTE_INTEGRATION_DIR" zsh $argv
    end
  end
  if test -r "$TERMINASTE_INTEGRATION_DIR/bashrc"
    function bash
      command bash --rcfile "$TERMINASTE_INTEGRATION_DIR/bashrc" $argv
    end
  end

  __FISH_BRIDGE__
  set -l isolated false
  if set -q TERMINASTE_ISOLATED
    set isolated $TERMINASTE_ISOLATED
  end
  __terminaste_emit ready "\"shell_family\":\"fish\",\"input_bridge\":true,\"control_keys\":true,\"isolated\":$isolated,"(__terminaste_data_pwd)
  __terminaste_emit directory-change (__terminaste_data_pwd)
end
"#
    .replace("__FISH_BRIDGE__", include_str!("fish_bridge.fish"))
}

fn powershell_script() -> String {
    r#"if ($env:TERMINASTE_SESSION -and -not $global:__TerminasteIntegrationLoaded) {
  $global:__TerminasteIntegrationLoaded = $true
  $global:__TerminasteSequence = 0
  $global:__TerminasteShellId = [guid]::NewGuid().ToString()
  $global:__TerminasteLastPath = (Get-Location).Path
  $global:__TerminasteLastHistoryId = 0
  $global:__TerminasteCommandStartMs = 0
  $global:__TerminasteOriginalPrompt = if (Test-Path Function:\prompt) { (Get-Command prompt).ScriptBlock } else { { "PS $($executionContext.SessionState.Path.CurrentLocation)> " } }

  function global:__TerminasteNowMs { [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() }
  function global:__TerminasteBase64Url([string]$Json) {
    [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Json)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
  }
  function global:__TerminasteEmit([string]$EventName, [hashtable]$Data) {
    $global:__TerminasteSequence += 1
    $Envelope = [ordered]@{
      session = $env:TERMINASTE_SESSION
      shell_id = $global:__TerminasteShellId
      sequence = $global:__TerminasteSequence
      time_ms = (__TerminasteNowMs)
      data = $Data
    }
    $Payload = __TerminasteBase64Url ($Envelope | ConvertTo-Json -Compress -Depth 8)
    [Console]::Write("$([char]27)Pterminaste;1;$EventName;json64;$Payload$([char]27)\")
  }
  function global:__TerminastePwdData { @{ current_directory = (Get-Location).Path } }

  Import-Module PSReadLine
  if ('Microsoft.PowerShell.PSConsoleReadLine' -as [type]) {
    __POWERSHELL_BRIDGE__
    Set-PSReadLineKeyHandler -Key Enter -ScriptBlock {
      $Line = $null
      $Cursor = $null
      [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$Line, [ref]$Cursor)
      $global:__TerminasteCommandStartMs = __TerminasteNowMs
      __TerminasteEmit 'prompt-end' (__TerminastePwdData)
      __TerminasteEmit 'command-start' @{ command = $Line; current_directory = (Get-Location).Path }
      [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
    }
  }

  function global:prompt {
    $ExitCode = if ($global:LASTEXITCODE -is [int]) { $global:LASTEXITCODE } elseif ($?) { 0 } else { 1 }
    $History = Get-History -Count 1
    if ($History -and $History.Id -ne $global:__TerminasteLastHistoryId) {
      $global:__TerminasteLastHistoryId = $History.Id
      if ($global:__TerminasteCommandStartMs -gt 0) {
        $DurationMs = [Math]::Max([long]0, (__TerminasteNowMs) - $global:__TerminasteCommandStartMs)
        __TerminasteEmit 'command-end' @{ command = $History.CommandLine; exit_code = $ExitCode; duration_ms = $DurationMs; current_directory = (Get-Location).Path }
        $global:__TerminasteCommandStartMs = 0
      }
    }
    $Path = (Get-Location).Path
    if ($Path -ne $global:__TerminasteLastPath) {
      $global:__TerminasteLastPath = $Path
      __TerminasteEmit 'directory-change' (__TerminastePwdData)
    }
    __TerminasteEmit 'prompt-start' (__TerminastePwdData)
    $PromptText = & $global:__TerminasteOriginalPrompt
    __TerminasteEmit 'prompt-end' (__TerminastePwdData)
    __TerminasteEmit 'editor-ready' @{ revision = $global:__TerminasteInputRevision }
    __TerminasteEmit 'input-buffer' @{ revision = $global:__TerminasteInputRevision; text = ''; cursor = 0; prompt = ($PromptText -join "`n") }
    $PromptText
  }

  __TerminasteEmit 'ready' @{ shell_family = 'powershell'; input_bridge = $true; control_keys = $true; isolated = ($env:TERMINASTE_ISOLATED -eq 'true'); current_directory = (Get-Location).Path }
  __TerminasteEmit 'directory-change' (__TerminastePwdData)
}
"#
    .replace("__POWERSHELL_BRIDGE__", include_str!("powershell_bridge.ps1"))
}

fn integration_temp_dir(session: &str) -> PathBuf {
    std::env::temp_dir()
        .join("terminaste-shell-integration")
        .join(session)
}

fn shell_bootstrap() -> String {
    let mut script = String::from("d=$(mktemp -d) || exit\ntrap 'rm -rf \"$d\"' EXIT\nexport TERMINASTE_INTEGRATION_DIR=\"$d\" TERMINASTE_ISOLATED=true\nunset TERMINASTE_ORIGINAL_ZDOTDIR ZDOTDIR\n");
    let mut files = zsh_startup_files();
    files.extend([
        ("bashrc", bash_script()),
        ("fish", fish_script()),
        ("powershell.ps1", powershell_script()),
        ("wrappers", include_str!("shell_wrappers.sh").to_owned()),
    ]);
    for (name, contents) in files {
        script.push_str(&format!(
            "printf %s '{}' | base64 -d >\"$d/{name}\" || exit\n",
            BASE64_STANDARD.encode(contents)
        ));
    }
    script.push_str(
        r#"export TERMINASTE_WRAPPERS="$(cat "$d/wrappers")"
case "${SHELL##*/}" in
  zsh) ZDOTDIR="$d" "$SHELL" -i ;;
  bash) "$SHELL" --rcfile "$d/bashrc" -i ;;
  fish) "$SHELL" -C "source '$d/fish'" ;;
  pwsh|powershell) "$SHELL" -NoExit -File "$d/powershell.ps1" ;;
  *) "${SHELL:-/bin/sh}" -i ;;
esac
"#,
    );
    script
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| {
            duration.as_secs().saturating_mul(1000) + u64::from(duration.subsec_millis())
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    #[test]
    fn detects_shell_family_from_paths() {
        assert_eq!(detect_shell_family("/bin/zsh"), ShellFamily::Zsh);
        assert_eq!(detect_shell_family("bash"), ShellFamily::Bash);
        assert_eq!(
            detect_shell_family("/usr/local/bin/fish"),
            ShellFamily::Fish
        );
        assert_eq!(detect_shell_family("pwsh.exe"), ShellFamily::PowerShell);
        assert_eq!(detect_shell_family("/bin/sh"), ShellFamily::Unknown);
    }

    #[test]
    fn encodes_private_dcs_frame_with_envelope() {
        let frame =
            encode_integration_frame("session-1", 7, "ready", json!({"shell_family":"zsh"}))
                .unwrap();
        assert!(frame.starts_with("\x1bPterminaste;1;ready;json64;"));
        assert!(frame.ends_with("\x1b\\"));
        let payload = frame
            .trim_start_matches("\x1bPterminaste;1;ready;json64;")
            .trim_end_matches("\x1b\\");
        let decoded = BASE64_URL_SAFE_NO_PAD.decode(payload).unwrap();
        let envelope: Value = serde_json::from_slice(&decoded).unwrap();
        assert_eq!(envelope["session"], "session-1");
        assert_eq!(envelope["sequence"], 7);
        assert_eq!(envelope["data"]["shell_family"], "zsh");
    }

    #[test]
    fn generated_scripts_contain_expected_hooks_and_session() {
        let zsh = generated_script(ShellFamily::Zsh);
        assert!(zsh.contains("TERMINASTE_SESSION"));
        assert!(zsh.contains("add-zsh-hook preexec"));
        assert!(zsh.contains("add-zsh-hook precmd"));

        let bash = generated_script(ShellFamily::Bash);
        assert!(bash.contains("PROMPT_COMMAND"));
        assert!(bash.contains("trap '__terminaste_debug_trap' DEBUG"));

        let fish = generated_script(ShellFamily::Fish);
        assert!(fish.contains("--on-event fish_preexec"));
        assert!(fish.contains("--on-event fish_postexec"));

        let powershell = generated_script(ShellFamily::PowerShell);
        assert!(powershell.contains("Set-PSReadLineKeyHandler"));
        assert!(powershell.contains("function global:prompt"));
    }

    #[test]
    fn shell_startup_uses_safe_per_shell_args() {
        let zsh = shell_startup("/bin/zsh", "test-zsh").unwrap();
        assert_eq!(zsh.family, ShellFamily::Zsh);
        assert_eq!(zsh.args, vec!["-i", "-l"]);
        assert!(zsh.env.iter().any(|(key, _)| key == "ZDOTDIR"));

        let bash = shell_startup("bash", "test-bash").unwrap();
        assert_eq!(bash.family, ShellFamily::Bash);
        assert_eq!(bash.args[0], "--rcfile");
        assert_eq!(bash.args[2], "-i");

        let fish = shell_startup("fish", "test-fish").unwrap();
        assert_eq!(fish.args[0], "-C");
        assert!(fish.args[1].contains("fish_preexec"));

        let powershell = shell_startup("pwsh.exe", "test-pwsh").unwrap();
        assert!(powershell.args.contains(&"-NoExit".to_owned()));
        assert!(powershell.args.contains(&"-Command".to_owned()));
    }
}
