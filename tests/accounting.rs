#![allow(dead_code)]

mod task {
    #[derive(Debug, Clone, Copy)]
    pub enum TaskMode {
        User,
        Kernel,
    }
}

#[path = "../kernel/src/process/accounting.rs"]
mod accounting;

use accounting::{CpuAccounting, CpuUsage, NS_PER_USER_TICK};
use task::TaskMode::{Kernel, User};

#[test]
fn nested_transitions_charge_each_interval_once() {
    let mut cpu = CpuAccounting::new();
    cpu.resume(100);
    cpu.set_mode(User, 110);
    cpu.set_mode(Kernel, 130);
    cpu.set_mode(Kernel, 135);
    cpu.set_mode(Kernel, 140);
    cpu.set_mode(User, 150);
    assert_eq!(
        cpu.snapshot(160),
        CpuUsage {
            user_ns: 30,
            kernel_ns: 30
        }
    );
    assert_eq!(cpu.snapshot(160), cpu.snapshot(160));
    cpu.pause(160);
    assert_eq!(
        cpu.snapshot(1000),
        CpuUsage {
            user_ns: 30,
            kernel_ns: 30
        }
    );
}

#[test]
fn idle_interrupts_and_switched_out_time_are_not_charged() {
    let mut cpu = CpuAccounting::new();
    cpu.resume(0);
    cpu.pause(10);
    cpu.set_mode(Kernel, 100);
    cpu.set_mode(Kernel, 110);
    cpu.resume(1000);
    cpu.resume(1010);
    cpu.pause(1020);
    assert_eq!(
        cpu.snapshot(9000),
        CpuUsage {
            user_ns: 0,
            kernel_ns: 30
        }
    );
}

#[test]
fn sub_tick_intervals_accumulate_before_conversion() {
    let mut cpu = CpuAccounting::new();
    for now in 0..100 {
        cpu.resume(now * 1_000_000);
        cpu.pause(now * 1_000_000 + 500_000);
    }
    assert_eq!(cpu.snapshot(1_000_000_000).kernel_ns / NS_PER_USER_TICK, 5);
}

#[test]
fn reaping_includes_descendants_without_charging_parent_cpu() {
    let mut parent = CpuAccounting::new();
    parent.resume(0);
    parent.pause(10);
    let mut child = CpuAccounting::new();
    assert_eq!(child.snapshot(100), CpuUsage::default());
    child.resume(20);
    child.set_mode(User, 30);
    child.pause(60);
    child.children = CpuUsage {
        user_ns: 7,
        kernel_ns: 9,
    };
    parent.reap(&child, 1000);
    assert_eq!(
        parent.children,
        CpuUsage {
            user_ns: 37,
            kernel_ns: 19
        }
    );
    assert_eq!(
        parent.snapshot(1000),
        CpuUsage {
            user_ns: 0,
            kernel_ns: 10
        }
    );
}

#[test]
fn usage_saturates_instead_of_wrapping() {
    let mut usage = CpuUsage {
        user_ns: u64::MAX - 1,
        kernel_ns: u64::MAX,
    };
    usage.add(CpuUsage {
        user_ns: 2,
        kernel_ns: 1,
    });
    assert_eq!(
        usage,
        CpuUsage {
            user_ns: u64::MAX,
            kernel_ns: u64::MAX
        }
    );
}
