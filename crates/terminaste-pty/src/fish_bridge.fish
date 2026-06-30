set -g __TERMINASTE_INPUT_REVISION 0

function __terminaste_report_input
  if set -q __TERMINASTE_COLLECTING
    set -e __TERMINASTE_COLLECTING
    set -l framed (commandline | string collect)
    set -l header (string sub --length 8 -- "$framed")
    set -l payload (string sub --start 9 -- "$framed")
    string match -qr '^[0-9a-f]{8}$' -- "$header"; or return
    set -l length (math "0x$header")
    test "$length" -gt 0; and test "$length" -le 65536; and test "$length" -eq (string length -- "$payload"); or return
    set -l parts (string split --max 2 ';' -- "$payload")
    test (count $parts) -eq 3; or return
    string match -qr '^[0-9]+$' -- "$parts[1]"; or return
    string match -qr '^[0-9]+$' -- "$parts[2]"; or return
    set -l text (printf %s "$parts[3]" | base64 -d | string collect --allow-empty --no-trim-newlines)
    commandline --replace -- "$text"
    commandline --cursor "$parts[1]"
    set -g __TERMINASTE_INPUT_REVISION "$parts[2]"
  end
  set -l text (commandline | string collect)
  set -l cursor (commandline --cursor)
  set -l prompt (fish_prompt | string collect)
  __terminaste_emit input-buffer "\"text\":\""(__terminaste_json_escape "$text")"\",\"cursor\":$cursor,\"revision\":$__TERMINASTE_INPUT_REVISION,\"prompt\":\""(__terminaste_json_escape "$prompt")"\""
end

function __terminaste_replace_input
  set -g __TERMINASTE_COLLECTING 1
  commandline --replace -- ''
end

function __terminaste_complete
  set -l original (commandline | string collect)
  set -l before (commandline --cut-at-cursor | string collect)
  set -l token (commandline --current-token --cut-at-cursor | string collect)
  set -l cursor (commandline --cursor)
  set -l start (math $cursor - (string length -- "$token"))
  set -l prefix (string sub --length $start -- "$original" | string collect --allow-empty)
  set -l suffix (string sub --start (math $cursor + 1) -- "$original" | string collect --allow-empty)
  set -l items
  for candidate in (complete --do-complete "$before")
    set -l text (string split --max 1 \t -- "$candidate")[1]
    set -l replacement "$prefix$text$suffix"
    set -l point (math $start + (string length -- "$text"))
    set -a items "{\"text\":\""(__terminaste_json_escape "$replacement")"\",\"cursor\":$point}"
  end
  __terminaste_emit completions "\"revision\":$__TERMINASTE_INPUT_REVISION,\"text\":\""(__terminaste_json_escape "$original")"\",\"items\":["(string join , -- $items)"]"
end

function __terminaste_history_options
  set -l original (commandline | string collect)
  set -l items
  for text in (history search --null --prefix --max 2000 -- "$original" | string split0)
    set -a items "{\"text\":\""(__terminaste_json_escape "$text")"\",\"cursor\":"(string length -- "$text")"}"
  end
  __terminaste_emit completions "\"revision\":$__TERMINASTE_INPUT_REVISION,\"text\":\""(__terminaste_json_escape "$original")"\",\"items\":["(string join , -- $items)"]"
end

function __terminaste_editor_ready --on-event fish_prompt
  for keymap in default insert
    bind --mode $keymap \cxr __terminaste_replace_input
    bind --mode $keymap \cxf __terminaste_report_input
    bind --mode $keymap \cxo __terminaste_complete
    bind --mode $keymap \cxh __terminaste_history_options
  end
  __terminaste_emit editor-ready "\"revision\":$__TERMINASTE_INPUT_REVISION"
  set -l prompt (fish_prompt | string collect)
  __terminaste_emit input-buffer "\"revision\":$__TERMINASTE_INPUT_REVISION,\"text\":\"\",\"cursor\":0,\"prompt\":\""(__terminaste_json_escape "$prompt")"\""
end

for program in ssh sudo su pwsh
  function $program --inherit-variable program
    /bin/sh -c "$TERMINASTE_WRAPPERS
$program \"\$@\"" terminaste $argv
  end
end
