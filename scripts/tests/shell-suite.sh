#!/usr/bin/env bash
# Built-in command cases for the shared prompt-driven QEMU harness.
shell_case() {
    shell_commands+=("$1")
    shell_checks+=("$2")
    shell_expected+=("${3-}")
}

shell_negative() {
    shell_case "$1" exact "$2"
    shell_case "echo KRUST_SHELL_RECOVERY_OK" exact "KRUST_SHELL_RECOVERY_OK"
    shell_case "pwd" exact "$shell_cwd"
}

shell_suite() {
    shell_commands=()
    shell_checks=()
    shell_expected=()
    shell_cwd=/
    shell_case "cd /" quiet
    shell_case "pwd" exact "/"
    if [[ "$1" == disk ]]; then
    shell_case "" quiet
    shell_case "help" help
    shell_case "echo KRUST_SHELL_ECHO_OK" exact "KRUST_SHELL_ECHO_OK"
    shell_case "echo" quiet
    shell_case "set KRUST_SHELL_VALUE=first value" quiet
    shell_case "env" environment "KRUST_SHELL_VALUE=first value"
    shell_case "set KRUST_SHELL_VALUE second value" quiet
    shell_case "env" environment "KRUST_SHELL_VALUE=second value"
    shell_case "set KRUST_SHELL_EMPTY=" quiet
    shell_case "env" environment "KRUST_SHELL_EMPTY="
    shell_case "clear" clear
    shell_case "echo KRUST_SHELL_CLEAR_OK" exact "KRUST_SHELL_CLEAR_OK"
    shell_case "mem" memory
    shell_case "ps" processes
    shell_case "lsdev" lines $'Registered devices:\n  console\n  null\n  sda\n  ttyS0'
    shell_case "lspci" pci
    shell_case "lsmod" lines $'Loaded modules:\n  procfs\n  devfs\n  pci-bus\n  sata-dev'
    shell_case "uptime" uptime
    shell_negative "cd missing" "cd: errno -2"
    shell_negative "cat missing" "cat: missing: errno -2"
    shell_negative "krust-no-such-command" "krust-no-such-command: command not found"
    shell_case "env" environment "KRUST_SHELL_VALUE=second value"
    shell_case "uptime" uptime
    shell_case "/bin/busybox echo KRUST_FORK_EXEC_OK" exact "KRUST_FORK_EXEC_OK"
    shell_case "exit" quiet
    return
    fi

    shell_case "mkdir /krust-shell-test" exact "created directory: /krust-shell-test"
    shell_case "cd /krust-shell-test" quiet
    shell_case "pwd" exact "/krust-shell-test"
    shell_case "mkdir nested" exact "created directory: nested"
    shell_case "cd ./nested" quiet
    shell_case "pwd" exact "/krust-shell-test/nested"
    shell_cwd=/krust-shell-test/nested
    shell_case "write note.txt shell round trip one" exact "wrote 20 bytes to note.txt"
    shell_case "cat note.txt" exact "shell round trip one"
    shell_case "write note.txt shell round trip two" exact "wrote 20 bytes to note.txt"
    shell_case "cat ./note.txt" exact "shell round trip two"
    shell_case "cat /krust-shell-test/nested/note.txt" exact "shell round trip two"
    shell_case "write .hidden hidden-data" exact "wrote 11 bytes to .hidden"
    shell_case "ls" listing
    shell_case "ls -a" all-listing
    shell_case "ls -l" long-listing
    shell_case "ls note.txt" exact "note.txt (file, 20 bytes)"
    shell_case "stat note.txt" stat
    shell_case "stat ." directory-stat
    shell_case "cd .." quiet
    shell_case "pwd" exact "/krust-shell-test"
    shell_case "ls" exact "  nested/"
    shell_case "cd nested/." quiet
    shell_case "pwd" exact "/krust-shell-test/nested"
    shell_negative "set" "usage: set <key=value> | set <key> <value>"
    shell_negative "set =bad" "usage: set <key=value> | set <key> <value>"
    shell_negative "cd" "usage: cd <path>"
    shell_negative "cat" "usage: cat <path>"
    shell_negative "stat" "usage: stat <path>"
    shell_negative "mkdir" "usage: mkdir <path>"
    shell_negative "write" "usage: write <path> <data>"
    shell_negative "write note.txt" "usage: write <path> <data>"
    shell_negative "cd missing" "cd: errno -2"
    shell_negative "cd note.txt" "cd: errno -20"
    shell_negative "cat missing" "cat: missing: errno -2"
    shell_negative "cat ." "cat: .: Is a directory"
    shell_negative "stat missing" "stat: errno -2"
    shell_negative "ls missing" "ls: errno -2"
    shell_negative "write missing/note.txt text" "write: errno -2"
    shell_negative "krust-no-such-command" "krust-no-such-command: command not found"
    shell_case "cd /" quiet
    shell_case "pwd" exact "/"
    shell_case "echo KRUST_SHELL_RAMFS_OK" exact "KRUST_SHELL_RAMFS_OK"
    shell_case "exit" quiet
}

shell_check_response() {
    local index=$1 command=${commands[$1]} response="$work/response"
    local check=${shell_checks[$1]} expected=${shell_expected[$1]} line ticks
    # Remove terminal controls, the next prompt, and at most one echoed input
    # line. Never let input echo stand in for the command's output.
    tr -d '\r' < "$work/raw-response" |
        sed -E $'s/\033\\[[0-9;]*[mHJ]//g; s/krust> $//' |
        awk -v command="$command" 'NR == 1 && $0 == command { next } { print }' > "$response"
    if [[ "$check" != exact ]] && grep -Eq "$errors|usage:|Is a directory|not enabled|failed to" "$response"; then
        return 1
    fi
    case "$check" in
        exact) [[ "$(< "$response")" == "$expected" ]] ;;
        quiet) ! grep -q '[^[:space:]]' "$response" ;;
        clear) grep -Fq $'\033[2J\033[H' "$work/raw-response" &&
            ! grep -q '[^[:space:]]' "$response" ;;
        help)
            grep -qx 'Available commands:' "$response" || return 1
            for line in help echo set env clear mem ps ls stat pwd cat cd mkdir write lsdev lspci lsmod uptime shutdown exit; do
                grep -Eq "^  $line +.* - " "$response" || return 1
            done ;;
        environment)
            grep -Fxq "$expected" "$response" &&
                [[ $(grep -c "^${expected%%=*}=" "$response") == 1 ]] ;;
        memory)
            grep -Eq '^MemTotal: +[1-9][0-9]* kB$' "$response" &&
                grep -Eq '^MemFree: +[0-9]+ kB$' "$response" ;;
        processes)
            grep -qx 'PID  PPID   STATE   NAME' "$response" &&
                grep -Eq '^[0-9]+ +[0-9]+ +[^ ]+ +.*shell' "$response" ;;
        lines)
            while IFS= read -r line; do
                grep -Fxq "$line" "$response" || return 1
            done <<< "$expected" ;;
        pci)
            grep -qx 'bus: pci0' "$response" &&
                grep -Eq '^  sda +[0-9a-f]{2}:[0-9a-f]{2}\.[0-7] class=01 subclass=06 prog_if=01 bar5=0x[0-9a-f]+$' "$response" ;;
        uptime)
            [[ "$(< "$response")" =~ ^ticks\ since\ boot:\ ([0-9]+)$ ]] || return 1
            ticks=${BASH_REMATCH[1]}
            (( ticks > 0 && ticks >= shell_last_ticks )) || return 1
            shell_last_ticks=$ticks ;;
        listing)
            grep -qx '  note.txt' "$response" &&
                grep -qx '  .hidden' "$response" &&
                ! grep -Eq '^  (\./|\.\./)$' "$response" ;;
        all-listing)
            for line in '  note.txt' '  .hidden'; do
                grep -Fxq "$line" "$response" || return 1
            done ;;
        long-listing)
            grep -Eq '^note.txt +perms=[rwx-]{9} ino=[1-9][0-9]* +size=20 +type=file$' "$response" &&
                grep -Eq '^\.hidden +perms=[rwx-]{9} ino=[1-9][0-9]* +size=11 +type=file$' "$response" ;;
        stat)
            grep -Eq '^note.txt +perms=[rwx-]{9} ino=[1-9][0-9]* +size=20 +type=file$' "$response" ;;
        directory-stat)
            grep -Eq '^\. +perms=[rwx-]{9} ino=[1-9][0-9]* +size=[0-9]+ +type=dir$' "$response" ;;
        *) echo "Unknown shell assertion: $check (case $index)" >&2; return 1 ;;
    esac
}
