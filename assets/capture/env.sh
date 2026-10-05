# Sourced by the VHS tapes: fixed shell environment for README captures.
export PATH="$REVIA_BIN_DIR:$PATH"
export PS1='$ '
export PAGER=cat
export LANG=en_US.UTF-8
export TERM=xterm-256color
export COLORTERM=truecolor
unset NO_COLOR
export REVIA_DEMO="${TMPDIR:-/tmp}/revia-demo"
"$(dirname "${BASH_SOURCE[0]}")/setup-fixture.sh" "$REVIA_DEMO" >/dev/null
cd "$REVIA_DEMO"
clear
