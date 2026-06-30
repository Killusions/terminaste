__terminaste_launch_command() {
  __terminaste_body="export TERMINASTE_SESSION='$TERMINASTE_SESSION' TERMINASTE_BOOTSTRAP='$TERMINASTE_BOOTSTRAP'; /bin/sh -c \"\$(printf %s '$TERMINASTE_BOOTSTRAP' | base64 -d)\""
  __terminaste_command="/bin/sh -c $(__terminaste_quote "$__terminaste_body")"
}

__terminaste_quote() {
  printf "'%s'" "$(printf %s "$1" | sed "s/'/'\\\\''/g")"
}

__terminaste_ssh_interactive() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --) shift; [ "$#" -eq 1 ]; return ;;
      -*)
        local flags="${1#-}" option
        shift
        while [ -n "$flags" ]; do
          option="${flags%"${flags#?}"}"
          flags="${flags#?}"
          case "$option" in
            N|T|G|V|s|W|O|Q|f|n) return 1 ;;
            b|c|D|E|e|F|I|i|J|L|l|m|o|p|R|S|w|B)
              if [ -z "$flags" ]; then
                [ "$#" -gt 0 ] || return 1
                shift
              fi
              break ;;
            4|6|A|a|C|g|K|k|q|t|v|X|x|Y|y) ;;
            *) return 1 ;;
          esac
        done ;;
      *) [ "$#" -eq 1 ]; return ;;
    esac
  done
  return 1
}

ssh() {
  if [ -z "${TERMINASTE_BOOTSTRAP:-}" ] || ! __terminaste_ssh_interactive "$@"; then
    command ssh "$@"
    return
  fi
  local config
  config=$(command ssh -G "$@") || return
  case "$config" in
    *'remotecommand none'*) ;;
    *remotecommand*) command ssh "$@"; return ;;
  esac
  case "$config" in
    *'requesttty false'*) command ssh "$@"; return ;;
  esac
  local __terminaste_body __terminaste_command
  __terminaste_launch_command
  command ssh -t "$@" "$__terminaste_command"
}

__terminaste_sudo_shell() {
  local interactive=1 flags option
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --) shift; [ "$#" -eq 0 ] && return "$interactive"; return 1 ;;
      --shell|--login) interactive=0; shift ;;
      --user|--group|--host|--prompt|--chdir|--chroot|--close-from|--command-timeout)
        [ "$#" -ge 2 ] || return 1; shift 2 ;;
      --user=*|--group=*|--host=*|--prompt=*|--chdir=*|--chroot=*|--preserve-env=*) shift ;;
      --non-interactive|--stdin|--preserve-env|--reset-timestamp) shift ;;
      --*) return 1 ;;
      -*)
        flags="${1#-}"
        shift
        while [ -n "$flags" ]; do
          option="${flags%"${flags#?}"}"
          flags="${flags#?}"
          case "$option" in
            i|s) interactive=0 ;;
            u|g|h|p|D|R|C|T)
              if [ -z "$flags" ]; then
                [ "$#" -gt 0 ] || return 1
                shift
              fi
              break ;;
            E|H|n|S|k|P) ;;
            *) return 1 ;;
          esac
        done ;;
      *=*) shift ;;
      *) return 1 ;;
    esac
  done
  return "$interactive"
}

sudo() {
  if [ -n "${TERMINASTE_BOOTSTRAP:-}" ] && __terminaste_sudo_shell "$@"; then
    local __terminaste_body __terminaste_command
    __terminaste_launch_command
    command sudo "$@" /bin/sh -c "$__terminaste_body"
  else
    command sudo "$@"
  fi
}

__terminaste_su_shell() {
  local user=0
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -|-l|-m|-p|--login|--preserve-environment) ;;
      -s|--shell|--group|-g|--supp-group|-G)
        [ "$#" -ge 2 ] || return 1; shift ;;
      --shell=*|--group=*|--supp-group=*) ;;
      -*) return 1 ;;
      *) [ "$user" -eq 0 ] || return 1; user=1 ;;
    esac
    shift
  done
  __terminaste_su_has_user=$user
}

su() {
  local __terminaste_su_has_user=0
  if [ -n "${TERMINASTE_BOOTSTRAP:-}" ] && __terminaste_su_shell "$@"; then
    local __terminaste_body __terminaste_command
    __terminaste_launch_command
    if [ "$__terminaste_su_has_user" -eq 0 ]; then
      command su "$@" root -c "$__terminaste_command"
    else
      command su "$@" -c "$__terminaste_command"
    fi
  else
    command su "$@"
  fi
}

pwsh() {
  if [ "$#" -eq 0 ] && [ -r "$TERMINASTE_INTEGRATION_DIR/powershell.ps1" ]; then
    command pwsh -NoExit -File "$TERMINASTE_INTEGRATION_DIR/powershell.ps1"
  else
    command pwsh "$@"
  fi
}
