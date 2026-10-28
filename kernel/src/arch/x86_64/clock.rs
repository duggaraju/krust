use core::arch::x86_64::{__cpuid, _mm_lfence, _rdtsc};
use x86_64::instructions::port::Port;

pub fn counter() -> u64 {
    unsafe {
        _mm_lfence();
        let value = _rdtsc();
        _mm_lfence();
        value
    }
}

pub fn frequency() -> u64 {
    if __cpuid(0x8000_0000).eax < 0x8000_0007 || __cpuid(0x8000_0007).edx & (1 << 8) == 0 {
        log::warn!("CPU accounting assumes a constant-rate TSC; invariant TSC is not advertised");
    }
    if __cpuid(0).eax >= 0x15 {
        let ratio = __cpuid(0x15);
        if ratio.eax != 0 && ratio.ebx != 0 && ratio.ecx != 0 {
            return u64::from(ratio.ecx) * u64::from(ratio.ebx) / u64::from(ratio.eax);
        }
    }

    // PIT channel 2 is independent of the IRQ0 uptime timer.
    let mut samples = [0u64; 3];
    for sample in &mut samples {
        let mut speaker = Port::<u8>::new(0x61);
        let saved = unsafe { speaker.read() };
        unsafe {
            speaker.write(saved & !3);
            Port::<u8>::new(0x43).write(0xb0);
            Port::<u8>::new(0x42).write(11932u16 as u8);
            Port::<u8>::new(0x42).write((11932u16 >> 8) as u8);
        }
        let start = counter();
        unsafe { speaker.write((saved & !2) | 1) };
        let mut elapsed = None;
        for _ in 0..10_000_000 {
            if unsafe { speaker.read() } & 0x20 != 0 {
                elapsed = Some(counter().saturating_sub(start));
                break;
            }
            core::hint::spin_loop();
        }
        unsafe { speaker.write(saved) };
        let ticks = elapsed.expect("CPU clock calibration: PIT channel 2 timed out");
        *sample = (u128::from(ticks) * 1_193_182 / 11932) as u64;
        assert!(
            *sample != 0,
            "CPU clock calibration produced zero frequency"
        );
    }
    samples.sort_unstable();
    samples[1]
}
