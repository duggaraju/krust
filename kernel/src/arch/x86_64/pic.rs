use x86_64::instructions::port::Port;

pub const IRQ_BASE_VECTOR: u8 = 32;

unsafe fn write(port: u16, value: u8) {
    unsafe {
        Port::<u8>::new(port).write(value);
        // The legacy PIC needs time to process each initialization command.
        Port::<u8>::new(0x80).write(0);
    }
}

pub fn init() {
    unsafe {
        write(0x20, 0x11);
        write(0xa0, 0x11);
        write(0x21, IRQ_BASE_VECTOR);
        write(0xa1, IRQ_BASE_VECTOR + 8);
        write(0x21, 4);
        write(0xa1, 2);
        write(0x21, 1);
        write(0xa1, 1);
        // Only the PIT has an active IRQ consumer; other devices are polled.
        write(0x21, 0xfe);
        write(0xa1, 0xff);
    }
}

pub fn end_of_interrupt(vector: u8) {
    unsafe {
        // Spurious IRQ7/IRQ15 do not have an in-service bit to acknowledge.
        if vector == IRQ_BASE_VECTOR + 7 || vector == IRQ_BASE_VECTOR + 15 {
            let port = if vector == IRQ_BASE_VECTOR + 7 {
                0x20
            } else {
                0xa0
            };
            Port::<u8>::new(port).write(0x0b);
            if Port::<u8>::new(port).read() & 0x80 == 0 {
                if port == 0xa0 {
                    Port::<u8>::new(0x20).write(0x20);
                }
                return;
            }
        }
        if vector >= IRQ_BASE_VECTOR + 8 {
            Port::<u8>::new(0xa0).write(0x20);
        }
        Port::<u8>::new(0x20).write(0x20);
    }
}
