mod support;

fn exercise(format: &str, firmware: &str, utility: &str) {
    let output = format!("target/cargo-tests/busybox/{format}/{firmware}/{utility}");
    support::run("scripts/test-busybox-utilities.sh", |command| {
        command
            .arg(support::fixture(
                "KRUST_TEST_BUSYBOX",
                "target/rootfs/bin/busybox",
            ))
            .arg(env!("CARGO_BIN_EXE_krust"))
            .args([format, "--firmware", firmware, "--utility", utility])
            .args(["--output-dir", &output]);
    });
}

macro_rules! utility_tests {
    ($format:literal, $firmware:literal; $($name:ident),+ $(,)?) => {
        $(
            #[test]
            #[ignore = "requires QEMU and an existing BusyBox binary; compatibility gaps fail"]
            fn $name() {
                crate::exercise($format, $firmware, stringify!($name));
            }
        )+

        #[test]
        #[ignore = "requires QEMU and an existing BusyBox binary"]
        fn false_exit_status() {
            crate::exercise($format, $firmware, "false");
        }

        #[test]
        #[ignore = "requires QEMU and an existing BusyBox binary"]
        fn true_exit_status() {
            crate::exercise($format, $firmware, "true");
        }
    };
}

macro_rules! applets {
    ($format:literal, $firmware:literal) => {
        utility_tests!(
            $format, $firmware;
            ash, awk, cat, chmod, chown, cp, cut, date, echo, env, find, grep,
            gunzip, gzip, head, hexdump, kill, ln, ls, mkdir, mv, od, printenv,
            ps, rm, rmdir, sed, sh, sleep, sort, stat, strings, tail, tar,
            touch, uniq, wc, xargs
        );
    };
}

mod fat {
    mod bios {
        applets!("fat", "bios");
    }
    mod uefi {
        applets!("fat", "uefi");
    }
}

mod ext4 {
    mod bios {
        applets!("ext4", "bios");
    }
    mod uefi {
        applets!("ext4", "uefi");
    }
}
