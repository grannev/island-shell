# island-shell for bash: a command that runs a while becomes a live
# activity in the island — a ring beside the clock from the tenth second,
# then a tick, or a cross with its exit code, when it ends.
#
# Source it from ~/.bashrc (scripts/install.sh puts it here):
#     . ~/.local/share/island-shell/island.bash
#
# ISLAND_JOB_AFTER   seconds before a command shows (default 10)
# ISLAND_JOB_IGNORE  first words that are never jobs (editors, pagers, …)
#
# The island hears of every command but shows only those still running
# after ISLAND_JOB_AFTER; the shell starts no background process. A closed
# terminal ends its job as interrupted.

[[ $- == *i* ]] || return 0
[[ -n ${WAYLAND_DISPLAY-} ]] || return 0
command -v island-shell >/dev/null 2>&1 || return 0
[[ -z ${__island_hooked-} ]] || return 0
__island_hooked=1

: "${ISLAND_JOB_AFTER:=10}"
: "${ISLAND_JOB_IGNORE:=nvim vim vi view nano less more most man info htop btop top ssh mosh tmux screen watch tail journalctl fzf mpv python python3 ipython node irb ghci bash sh zsh fish sudoedit claude island-shell}"

__island_armed=0
__island_id=
__island_n=0

# before a command line runs (the DEBUG trap fires for every simple
# command; only the first after a prompt counts)
__island_preexec() {
    [[ $__island_armed == 1 ]] || return 0
    __island_armed=0
    local command=$BASH_COMMAND word
    # the program, past variable assignments and sudo
    for word in $command; do
        [[ $word == *=* || $word == sudo || $word == command || $word == time ]] && continue
        break
    done
    word=${word##*/}
    [[ " $ISLAND_JOB_IGNORE " == *" $word "* ]] && return 0
    __island_id="sh-$$-$((++__island_n))"
    island-shell task --pid=$$ start --after="$ISLAND_JOB_AFTER" "$__island_id" "${command:0:80}" >/dev/null 2>&1
}

# first in PROMPT_COMMAND: the line has finished
__island_status() {
    local code=$?
    if [[ -n $__island_id ]]; then
        if (( code == 0 )); then
            island-shell task done "$__island_id" >/dev/null 2>&1
        elif (( code == 130 || code > 128 )); then
            island-shell task fail "$__island_id" прервано >/dev/null 2>&1
        else
            island-shell task fail "$__island_id" "код $code" >/dev/null 2>&1
        fi
        __island_id=
    fi
    return "$code"
}

# last in PROMPT_COMMAND: the next command line is the user's
__island_arm() {
    local code=$?
    __island_armed=1
    return "$code"
}

PROMPT_COMMAND="__island_status${PROMPT_COMMAND:+; $PROMPT_COMMAND}; __island_arm"
trap '__island_preexec' DEBUG
