#[path = "../../kernel/src/arch/aarch64/user_layout.rs"]
mod aarch64_layout;
#[path = "../../kernel/src/syscall/mmap.rs"]
mod mmap;
#[path = "../../kernel/src/arch/x86_64/supervisor_overlap.rs"]
mod supervisor_overlap;
#[path = "../../kernel/src/arch/x86_64/user_layout.rs"]
mod user_layout;

#[test]
fn aarch64_has_arch_owned_mmap_bounds() {
    assert_eq!(aarch64_layout::PAGE_SIZE, 4096);
    assert_eq!(aarch64_layout::USER_ADDRESS_END, 1 << 47);
    assert_eq!(
        user_layout::USER_STACK_TOP + user_layout::PAGE_SIZE,
        user_layout::USER_ADDRESS_END
    );
}
