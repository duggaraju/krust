//! Hardware-independent legacy PCI scan; the reader supplies aligned config dwords.

#[derive(Debug)]
pub(crate) struct PciFunction {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub header_type: u8,
    pub bar5: Option<u32>,
}

impl PciFunction {
    pub fn is_ahci(&self) -> bool {
        self.header_type == 0
            && self.class_code == 0x01
            && self.subclass == 0x06
            && self.prog_if == 0x01
    }
}

pub(crate) fn enumerate<E>(
    mut read: impl FnMut(u8, u8, u8, u8) -> Result<u32, E>,
    mut found: impl FnMut(PciFunction),
) -> Result<(), E> {
    for bus in 0..=255 {
        for device in 0..32 {
            let mut function_count = 1;
            let mut function = 0;
            while function < function_count {
                let identity = read(bus, device, function, 0x00)?;
                if identity as u16 != 0xffff {
                    let header = (read(bus, device, function, 0x0c)? >> 16) as u8;
                    if function == 0 && header & 0x80 != 0 {
                        function_count = 8;
                    }
                    let class = read(bus, device, function, 0x08)?;
                    let mut info = PciFunction {
                        bus,
                        device,
                        function,
                        vendor_id: identity as u16,
                        device_id: (identity >> 16) as u16,
                        class_code: (class >> 24) as u8,
                        subclass: (class >> 16) as u8,
                        prog_if: (class >> 8) as u8,
                        header_type: header & 0x7f,
                        bar5: None,
                    };
                    // BAR5 exists only in type-0 headers; retain the AHCI ABAR path.
                    if info.is_ahci() {
                        info.bar5 = Some(read(bus, device, function, 0x24)? & 0xffff_fff0);
                    }
                    found(info);
                }
                function += 1;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    type Address = (u8, u8, u8, u8);

    #[derive(Default)]
    struct Config {
        data: BTreeMap<Address, u32>,
        reads: BTreeMap<Address, usize>,
    }

    impl Config {
        fn add(&mut self, bdf: (u8, u8, u8), identity: u32, class: u32, header: u8) {
            let (b, d, f) = bdf;
            self.data.insert((b, d, f, 0), identity);
            self.data.insert((b, d, f, 8), class);
            self.data.insert((b, d, f, 12), u32::from(header) << 16);
        }

        fn scan(&mut self) -> Vec<PciFunction> {
            let mut devices = Vec::new();
            enumerate(
                |b, d, f, offset| {
                    let address = (b, d, f, offset);
                    *self.reads.entry(address).or_default() += 1;
                    Ok::<_, ()>(*self.data.get(&address).unwrap_or_else(|| {
                        assert_eq!(offset, 0, "read past an absent function");
                        &0xffff_ffff
                    }))
                },
                |info| devices.push(info),
            )
            .unwrap();
            assert!(self.reads.values().all(|count| *count == 1));
            devices
        }
    }

    #[test]
    fn absent_slots_only_read_identity_and_do_not_probe_other_functions() {
        let mut config = Config::default();
        // Vendor ID alone determines absence, not the entire dword.
        config.data.insert((0, 0, 0, 0), 0x1234_ffff);
        assert!(config.scan().is_empty());
        assert_eq!(config.reads.len(), 256 * 32);
        assert!(
            config
                .reads
                .keys()
                .all(|(_, _, f, offset)| *f == 0 && *offset == 0)
        );
    }

    #[test]
    fn single_function_includes_non_ahci_identity_and_class_without_reading_bars() {
        let mut config = Config::default();
        config.add((0, 3, 0), 0x1044_1af4, 0x00ff_0001, 0);
        // A nonzero function must be ignored without the multifunction bit.
        config.add((0, 3, 1), 0x1000_1234, 0x0200_0000, 0);
        let devices = config.scan();
        assert_eq!(devices.len(), 1);
        let info = &devices[0];
        assert_eq!((info.bus, info.device, info.function), (0, 3, 0));
        assert_eq!((info.vendor_id, info.device_id), (0x1af4, 0x1044));
        assert_eq!((info.class_code, info.subclass, info.prog_if), (0, 0xff, 0));
        assert_eq!(info.header_type, 0);
        assert!(!info.is_ahci());
        assert_eq!(info.bar5, None);
        assert!(!config.reads.contains_key(&(0, 3, 1, 0)));
    }

    #[test]
    fn multifunction_scan_keeps_sparse_functions_and_reuses_function_zero_header() {
        let mut config = Config::default();
        config.add((255, 31, 0), 0x1234_8086, 0x0604_0000, 0x81);
        config.add((255, 31, 2), 0x5678_1234, 0x0200_0000, 0);
        config.add((255, 31, 7), 0xabcd_1234, 0x0300_0000, 0);
        let devices = config.scan();
        assert_eq!(
            devices.iter().map(|d| d.function).collect::<Vec<_>>(),
            [0, 2, 7]
        );
        assert_eq!(devices[0].header_type, 1);
        for function in 0..8 {
            assert_eq!(config.reads.get(&(255, 31, function, 0)), Some(&1));
        }
    }

    #[test]
    fn absent_function_zero_skips_even_populated_higher_functions() {
        let mut config = Config::default();
        config.add((0, 1, 3), 0x1044_1af4, 0x00ff_0000, 0);
        assert!(config.scan().is_empty());
        assert!(!config.reads.contains_key(&(0, 1, 3, 0)));
    }

    #[test]
    fn ahci_compatibility_retains_masked_abar_and_scan_order_among_other_devices() {
        let mut config = Config::default();
        config.add((0, 0, 0), 0x1044_1af4, 0x00ff_0000, 0);
        config.add((0, 1, 0), 0x2922_8086, 0x0106_0100, 0x80);
        config.data.insert((0, 1, 0, 0x24), 0xfebf_f008);
        config.add((0, 1, 5), 0x2922_8086, 0x0106_0100, 0);
        config.data.insert((0, 1, 5, 0x24), 0xfebf_e000);
        let devices = config.scan();
        let ahci: Vec<_> = devices.iter().filter(|d| d.is_ahci()).collect();
        assert_eq!(ahci.len(), 2);
        assert_eq!((ahci[0].bus, ahci[0].device, ahci[0].function), (0, 1, 0));
        assert_eq!(ahci[0].bar5, Some(0xfebf_f000));
        assert_eq!(ahci[1].function, 5);
        assert_eq!(ahci[1].bar5, Some(0xfebf_e000));
        assert_eq!(
            config
                .reads
                .keys()
                .filter(|(_, _, _, off)| *off == 0x24)
                .count(),
            2
        );
    }

    #[test]
    fn other_header_layouts_and_non_ahci_sata_do_not_read_abar() {
        let mut config = Config::default();
        config.add((0, 1, 0), 0x1234_8086, 0x0106_0100, 1);
        config.add((0, 2, 0), 0x1234_8086, 0x0106_0100, 2);
        config.add((0, 3, 0), 0x1234_8086, 0x0106_0000, 0);
        let devices = config.scan();
        assert_eq!(devices.len(), 3);
        assert!(devices.iter().all(|d| !d.is_ahci() && d.bar5.is_none()));
    }

    #[test]
    fn configuration_errors_are_propagated() {
        let result = enumerate(
            |_, _, _, _| Err::<u32, _>("config error"),
            |_| panic!("unexpected function"),
        );
        assert_eq!(result, Err("config error"));
    }
}
