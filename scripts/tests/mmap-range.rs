#[path = "../../kernel/src/arch/aarch64/user_layout.rs"]
mod aarch64_layout;
#[path = "../../kernel/src/syscall/mmap.rs"]
mod mmap;
#[path = "../../kernel/src/arch/x86_64/supervisor_overlap.rs"]
mod supervisor_overlap;
#[path = "../../kernel/src/arch/x86_64/user_layout.rs"]
mod user_layout;
#[path = "../../kernel/src/arch/x86_64/user_protection.rs"]
mod user_protection;
#[path = "../../kernel/src/arch/aarch64/user_protection.rs"]
mod aarch64_protection;

#[test]
fn aarch64_protections_are_explicitly_unsupported() {
    for prot in 0..8 {
        assert_eq!(aarch64_protection::mmap_page_flags(prot), Err(-38));
    }
    assert_eq!(aarch64_protection::mmap_page_flags(8), Err(-22));
}

#[test]
fn aarch64_has_arch_owned_mmap_bounds() {
    assert_eq!(aarch64_layout::PAGE_SIZE, 4096);
    assert_eq!(aarch64_layout::USER_ADDRESS_END, 1 << 47);
    assert_eq!(
        user_layout::USER_STACK_TOP + user_layout::PAGE_SIZE,
        user_layout::USER_ADDRESS_END
    );
}
