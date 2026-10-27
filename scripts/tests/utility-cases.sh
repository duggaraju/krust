#!/usr/bin/env bash
# Every applet exposed by install-busybox-applets.sh needs a functional case.
utility_case() {
    utility_status=0
    utility_check=exact
    utility_expected=
    case "$1" in
        sh|ash|bash) utility_command="/bin/$1 -c pwd"; utility_expected=/ ;;
        ls) utility_command='/bin/ls /fixture'; utility_check=listing ;;
        cat) utility_command='/bin/cat /fixture/input'; utility_expected=$'alpha\nbeta\nalpha' ;;
        echo) utility_command='/bin/echo utility echo'; utility_expected='utility echo' ;;
        cp) utility_command='/bin/cp /fixture/input /scratch/copied && /bin/cat /scratch/copied'; utility_expected=$'alpha\nbeta\nalpha' ;;
        mv) utility_command='/bin/mv /fixture/move /scratch/moved && /bin/cat /scratch/moved'; utility_expected=move-data ;;
        rm) utility_command='/bin/rm /fixture/remove && test ! -e /fixture/remove' ;;
        mkdir) utility_command='/bin/mkdir /scratch/newdir && test -d /scratch/newdir' ;;
        rmdir) utility_command='/bin/rmdir /fixture/empty && test ! -d /fixture/empty' ;;
        ln) utility_command='/bin/ln -s /fixture/input /scratch/link && /bin/cat /scratch/link'; utility_expected=$'alpha\nbeta\nalpha' ;;
        touch) utility_command='/bin/touch /scratch/touched && test -f /scratch/touched' ;;
        grep) utility_command='/bin/grep beta /fixture/input'; utility_expected=beta ;;
        sed) utility_command="/bin/sed 's/alpha/gamma/g' /fixture/input"; utility_expected=$'gamma\nbeta\ngamma' ;;
        awk) utility_command="/bin/awk '{print NR}' /fixture/input"; utility_expected=$'1\n2\n3' ;;
        cut) utility_command='/bin/cut -c 1 /fixture/input'; utility_expected=$'a\nb\na' ;;
        sort) utility_command='/bin/sort /fixture/input'; utility_expected=$'alpha\nalpha\nbeta' ;;
        uniq) utility_command='/bin/uniq /fixture/repeat'; utility_expected=$'alpha\nbeta' ;;
        head) utility_command='/bin/head -n 1 /fixture/input'; utility_expected=alpha ;;
        tail) utility_command='/bin/tail -n 1 /fixture/input'; utility_expected=alpha ;;
        wc) utility_command='/bin/wc -l /fixture/input'; utility_check=wc ;;
        find) utility_command='/bin/find /fixture/empty -type d'; utility_expected=/fixture/empty ;;
        xargs) utility_command='/bin/xargs -a /fixture/one /bin/echo'; utility_expected='one two' ;;
        stat) utility_command="/bin/stat -c '%s' /fixture/input"; utility_expected=17 ;;
        chmod) utility_command="/bin/chmod 600 /fixture/mode && /bin/stat -c '%a' /fixture/mode"; utility_expected=600 ;;
        chown) utility_command="/bin/chown 123:456 /fixture/owner && /bin/stat -c '%u:%g' /fixture/owner"; utility_expected=123:456 ;;
        ps) utility_command='/bin/ps'; utility_check=ps ;;
        kill) utility_command='/bin/kill -0 $$' ;;
        env) utility_command='/bin/env UTILITY_ENV=kept /bin/printenv UTILITY_ENV'; utility_expected=kept ;;
        printenv) utility_command='UTILITY_ENV=kept /bin/printenv UTILITY_ENV'; utility_expected=kept ;;
        date) utility_command='/bin/date +%s'; utility_check=number ;;
        sleep) utility_command='/bin/sleep 0.01' ;;
        true) utility_command='/bin/true' ;;
        false) utility_command='/bin/false'; utility_status=1 ;;
        hexdump) utility_command="/bin/hexdump -v -e '1/1 \"%02x\"' /fixture/bytes"; utility_expected=4142 ;;
        od) utility_command='/bin/od -An -tx1 /fixture/bytes'; utility_check=od ;;
        strings) utility_command='/bin/strings /fixture/strings'; utility_expected=UTILITY_STRING ;;
        gzip) utility_command='/bin/gzip /fixture/zipin && /bin/gunzip -c /fixture/zipin.gz'; utility_expected=$'alpha\nbeta\nalpha' ;;
        gunzip) utility_command='/bin/gunzip -c /fixture/data.gz'; utility_expected=$'alpha\nbeta\nalpha' ;;
        tar) utility_command='/bin/tar -tf /fixture/data.tar'; utility_expected=input ;;
        wget) utility_command='/bin/wget -T 1 -t 1 -O /scratch/web http://127.0.0.1:9/'; utility_check=network ;;
        *) echo "No functional test defined for installed utility '$1'" >&2; return 1 ;;
    esac
}

utility_validate() {
    local response=$1 status=$2 name
    [[ "$status" == "$utility_status" ]] || return 1
    case "$utility_check" in
        exact) [[ "$response" == "$utility_expected" ]] ;;
        listing)
            for name in input repeat one bytes strings data.gz data.tar empty; do
                grep -Eq "(^|[[:space:]])${name//./\\.}([[:space:]]|$)" <<< "$response" || return 1
            done ;;
        wc) [[ "$response" =~ ^[[:space:]]*3[[:space:]]+/fixture/input$ ]] ;;
        ps) grep -Eq '(^|[[:space:]])PID([[:space:]]|$)' <<< "$response" &&
            grep -q 'busybox' <<< "$response" ;;
        number) [[ "$response" =~ ^[0-9]+$ ]] ;;
        od) [[ "$response" =~ ^[[:space:]]*41[[:space:]]+42[[:space:]]*$ ]] ;;
        network)
            echo "wget is installed but its networking test requires a guest-reachable HTTP fixture" >&2
            return 1 ;;
        *) return 1 ;;
    esac
}
