use x86_64::instructions::port::Port;

/// PIT (Programmable Interval Timer) — 8253/8254 chip
/// IRQ0 fires at configured frequency
const PIT_DATA_0: u16 = 0x40;
const PIT_CONTROL: u16 = 0x43;

/// Frequency divider: 1193182 Hz / 11932 ≈ 100 Hz (10ms interval)
/// For 100 Hz: divider = 11932
/// For 1000 Hz: divider = 1193
const PIT_DIVISOR: u16 = 11932;

pub fn init() {
    unsafe {
        let mut control: Port<u8> = Port::new(PIT_CONTROL);
        let mut data: Port<u8> = Port::new(PIT_DATA_0);

        // Set control byte:
        // Bit 6-7: 00 = Counter 0
        // Bit 4-5: 11 = Load both bytes (low, then high)
        // Bit 1-3: 010 = Mode 2 (Rate Generator)
        // Bit 0:   0 = Binary (not BCD)
        // Control byte: 0x34 = 0011_0100
        control.write(0x34);

        // Load divisor (low byte first, then high byte)
        data.write((PIT_DIVISOR & 0xFF) as u8);
        data.write(((PIT_DIVISOR >> 8) & 0xFF) as u8);
    }
    log::info!("PIT initialized for ~100 Hz (10ms quantum)");
}
