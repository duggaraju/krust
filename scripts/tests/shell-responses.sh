#!/usr/bin/env bash
# Independent fake outputs; the real guest suite is also run under both firmwares.
shell_fake_response() {
    local command=$1 name
    case "$command" in
        ""|"echo") ;;
        "help")
            printf 'Available commands:\n'
            for name in help echo set env clear mem ps ls stat pwd cat cd mkdir write lsdev lspci lsmod uptime shutdown exit; do
                printf '  %s args - mock help\n' "$name"
            done ;;
        "echo KRUST_SHELL_ECHO_OK")
            [[ "$SMOKE_TEST_CASE" == shell-echo-only ]] || printf 'KRUST_SHELL_ECHO_OK\n' ;;
        "echo KRUST_SHELL_RECOVERY_OK")
            [[ "$SMOKE_TEST_CASE" == shell-missing-recovery ]] || printf 'KRUST_SHELL_RECOVERY_OK\n' ;;
        "echo KRUST_SHELL_CLEAR_OK") printf 'KRUST_SHELL_CLEAR_OK\n' ;;
        "echo KRUST_SHELL_RAMFS_OK") printf 'KRUST_SHELL_RAMFS_OK\n' ;;
        "/bin/busybox echo KRUST_FORK_EXEC_OK") printf 'KRUST_FORK_EXEC_OK\n' ;;
        "set KRUST_SHELL_VALUE=first value") shell_fake_value="first value" ;;
        "set KRUST_SHELL_VALUE second value")
            [[ "$SMOKE_TEST_CASE" == shell-env-loss ]] || shell_fake_value="second value" ;;
        "set KRUST_SHELL_EMPTY=") shell_fake_empty=true ;;
        "env")
            printf 'HOME=/\nKRUST_SHELL_VALUE=%s\n' "$shell_fake_value"
            [[ "$shell_fake_empty" != true ]] || printf 'KRUST_SHELL_EMPTY=\n' ;;
        "clear")
            [[ "$SMOKE_TEST_CASE" == shell-no-clear ]] || printf '\033[2J\033[H' ;;
        "mem") printf 'MemTotal:       65536 kB\nMemFree:        1234 kB\n' ;;
        "ps") printf 'PID  PPID   STATE   NAME\n2    1      Running shell\n' ;;
        "lsdev") printf 'Registered devices:\n  console\n  null\n  sda\n  ttyS0\n' ;;
        "lspci") printf 'bus: pci0\n  sda        00:04.0 class=01 subclass=06 prog_if=01 bar5=0xfebb1000\n' ;;
        "lsmod") printf 'Loaded modules:\n  procfs\n  devfs\n  pci-bus\n  sata-dev\n' ;;
        "uptime")
            printf 'ticks since boot: %s\n' "$shell_fake_ticks"
            if [[ "$SMOKE_TEST_CASE" == shell-uptime-backwards ]]; then
                shell_fake_ticks=1
            else
                shell_fake_ticks=$((shell_fake_ticks + 1))
            fi ;;
        "cd /") shell_fake_cwd=/ ;;
        "cd /krust-shell-test"|"cd ..") shell_fake_cwd=/krust-shell-test ;;
        "cd ./nested"|"cd nested/.") shell_fake_cwd=/krust-shell-test/nested ;;
        "pwd") printf '%s\n' "$shell_fake_cwd" ;;
        "mkdir /krust-shell-test") printf 'created directory: /krust-shell-test\n' ;;
        "mkdir nested") printf 'created directory: nested\n' ;;
        "write note.txt shell round trip one"|"write note.txt shell round trip two")
            shell_fake_contents=${command#write note.txt }
            printf 'wrote 20 bytes to note.txt\n' ;;
        "write .hidden hidden-data") printf 'wrote 11 bytes to .hidden\n' ;;
        "cat note.txt"|"cat ./note.txt"|"cat /krust-shell-test/nested/note.txt")
            if [[ "$SMOKE_TEST_CASE" == shell-bad-content ]]; then
                printf 'corrupt contents\n'
            else
                printf '%s\n' "$shell_fake_contents"
            fi ;;
        "ls")
            if [[ "$shell_fake_cwd" == /krust-shell-test ]]; then
                printf '  nested/\n'
            else
                printf '  note.txt\n  .hidden\n'
            fi ;;
        "ls -a") printf '  note.txt\n  .hidden\n' ;;
        "ls -l") printf 'note.txt             perms=rw-r--r-- ino=3        size=20       type=file\n.hidden              perms=rw-r--r-- ino=4        size=11       type=file\n' ;;
        "ls note.txt") printf 'note.txt (file, 20 bytes)\n' ;;
        "stat note.txt") printf 'note.txt             perms=rw-r--r-- ino=3        size=20       type=file\n' ;;
        "stat .") printf '.                    perms=rwxr-xr-x ino=2        size=0        type=dir\n' ;;
        "set"|"set =bad") printf 'usage: set <key=value> | set <key> <value>\n' ;;
        "cd") printf 'usage: cd <path>\n' ;;
        "cat") printf 'usage: cat <path>\n' ;;
        "stat") printf 'usage: stat <path>\n' ;;
        "mkdir") printf 'usage: mkdir <path>\n' ;;
        "write"|"write note.txt") printf 'usage: write <path> <data>\n' ;;
        "cd missing")
            if [[ "$SMOKE_TEST_CASE" == shell-wrong-diagnostic ]]; then
                printf 'cd: errno -5\n'
            else
                printf 'cd: errno -2\n'
            fi
            [[ "$SMOKE_TEST_CASE" != shell-cwd-loss ]] || shell_fake_cwd=/ ;;
        "cd note.txt") printf 'cd: errno -20\n' ;;
        "cat missing") printf 'cat: missing: errno -2\n' ;;
        "cat .") printf 'cat: .: Is a directory\n' ;;
        "stat missing") printf 'stat: errno -2\n' ;;
        "ls missing") printf 'ls: errno -2\n' ;;
        "write missing/note.txt text") printf 'write: errno -2\n' ;;
        "krust-no-such-command") printf 'krust-no-such-command: command not found\n' ;;
        "exit"|"shutdown")
            [[ "$SMOKE_TEST_CASE" != shell-shutdown-error ]] || printf 'shutdown: errno -5\n' ;;
        *) printf 'Unexpected fake shell command: %s\n' "$command" >&2; return 1 ;;
    esac
    return 0
}
