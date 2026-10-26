/* No libc or startup runtime: exercise the kernel's real syscall entry. */
typedef unsigned long usize;

static long syscall6(long nr, usize a, usize b, usize c, usize d, usize e, usize f)
{
    register usize r10 __asm__("r10") = d;
    register usize r8 __asm__("r8") = e;
    register usize r9 __asm__("r9") = f;
    long result;
    __asm__ volatile("syscall" : "=a"(result)
                     : "a"(nr), "D"(a), "S"(b), "d"(c),
                       "r"(r10), "r"(r8), "r"(r9)
                     : "rcx", "r11", "memory");
    return result;
}

__attribute__((noreturn)) static void finish(int ok)
{
    const char *message = ok ? "KRUST_MMAP_OK\n" : "KRUST_MMAP_FAIL\n";
    syscall6(1, 1, (usize)message, ok ? 14 : 16, 0, 0, 0);
    syscall6(60, ok ? 0 : 1, 0, 0, 0, 0, 0);
    for (;;) {}
}

static void check(int condition)
{
    if (!condition)
        finish(0);
}

static long mmap(usize addr, usize len, usize flags, usize fd)
{
    return syscall6(9, addr, len, 3, flags, fd, 0);
}

static usize next;
static volatile usize *previous;
static volatile usize *boundary;
static const usize sentinel = 0x123456789abcdefUL;
static volatile usize protected_sentinel = 0x123456789abcdefUL;

static void healthy(void)
{
    long result = mmap(0, 4096, 0x22, ~0UL);
    check(result > 0 && (usize)result == next);
    check(protected_sentinel == sentinel);
    if (previous)
        check(*previous == sentinel);
    if (boundary)
        check(*boundary == sentinel);
    previous = (volatile usize *)result;
    check(*previous == 0 && previous[511] == 0);
    *previous = sentinel;
    next += 4096;
}

static void reject(usize addr, usize len, usize flags, usize fd, long error)
{
    check(mmap(addr, len, flags, fd) == error);
    healthy(); /* Also proves failed calls did not advance default placement. */
}

static usize hex_address(const char *text)
{
    check(text[0] == '0' && text[1] == 'x');
    usize value = 0;
    for (usize i = 2; text[i]; ++i) {
        check(i < 14);
        usize digit = text[i] >= '0' && text[i] <= '9' ? text[i] - '0' :
                      text[i] >= 'a' && text[i] <= 'f' ? text[i] - 'a' + 10 : 16;
        check(digit < 16);
        value = (value << 4) | digit;
    }
    return value;
}

void test_main(const usize *initial_stack)
{
    const usize end = 1UL << 47; /* Four-level x86_64 lower canonical half. */
    check(initial_stack[0] == 2);
    usize kernel = hex_address((const char *)initial_stack[2]);
    check(kernel > 4096 && kernel < end && (kernel & 4095) == 0);
    next = 0x600000000000UL;
    healthy();
    reject(kernel, 4096, 0x22, ~0UL, -12);
    /* Start at a live ELF data page, then cross into the actual kernel image.
       Preflight must reject the whole request without zeroing this sentinel. */
    usize data_page = (usize)&protected_sentinel & ~4095UL;
    check(data_page < kernel);
    reject(data_page, kernel - data_page + 4096, 0x22, ~0UL, -12);
    static const char kernel_ok[] = "KRUST_MMAP_KERNEL_OK\n";
    syscall6(1, 1, (usize)kernel_ok, sizeof(kernel_ok) - 1, 0, 0, 0);
    /* This reserved stack-top page is far above the live initial stack. The
       existing mapper replaces it; this fixture does not test collision policy. */
    check(mmap(end - 4096, 4096, 0x22, ~0UL) == (long)(end - 4096));
    boundary = (volatile usize *)(end - 4096);
    check(*boundary == 0 && boundary[511] == 0);
    *boundary = sentinel;
    healthy();

    reject(0, 0, 0x22, ~0UL, -22);
    reject(0, ~0UL, 0x22, ~0UL, -12);
    reject(0, ~0UL - 1, 0x22, ~0UL, -12);
    reject(0, ~0UL - 4094, 0x22, ~0UL, -12);
    reject(0, ~0UL - 4095, 0x22, ~0UL, -12);
    reject(0, end, 0x22, ~0UL, -12);
    reject(~0UL, 4096, 0x22, ~0UL, -12);
    reject(~0UL - 4095, 8192, 0x22, ~0UL, -12);
    reject(end, 4096, 0x22, ~0UL, -12);
    reject(end + 4096, 4096, 0x22, ~0UL, -12);
    reject(0xffff800000000000UL, 4096, 0x22, ~0UL, -12);
    reject(end - 1, 1, 0x22, ~0UL, -12);
    reject(end - 4096, 4097, 0x22, ~0UL, -12);
    reject(end - 8192, 8193, 0x22, ~0UL, -12);
    reject(4096, ~0UL - 4095, 0x22, ~0UL, -12);
    /* Unsupported modes retain their existing error, even for zero length. */
    reject(0, 0, 0x32, ~0UL, -12);
    reject(0, 4096, 0x2, ~0UL, -12);
    reject(0, 4096, 0x22, 0, -12);

    long hinted = mmap(0x500000000001UL, 4097, 0x22, 0xffffffffUL);
    check(hinted == 0x500000001000L);
    volatile usize *p = (volatile usize *)hinted;
    check(p[0] == 0 && p[1023] == 0);
    p[0] = sentinel;
    p[1023] = sentinel;
    healthy(); /* Hints must not change the zero-hint cursor. */
    long rounded = mmap(0, 1, 0x22, ~0UL);
    check(rounded == (long)next);
    next += 4096;
    healthy();
    finish(1);
}

__asm__(".global _start\n"
        "_start:\n"
        "movq %rsp, %rdi\n"
        "andq $-16, %rsp\n"
        "call test_main\n"
        "ud2\n");
