/* Separate shell-launched processes: no userspace fork or mprotect required. */
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

static void say(const char *text)
{
    usize len = 0;
    while (text[len])
        ++len;
    syscall6(1, 1, (usize)text, len, 0, 0, 0);
}

__attribute__((noreturn)) static void finish(int ok)
{
    say(ok ? "KRUST_PROTECTION_OK\n" : "KRUST_PROTECTION_FAIL\n");
    syscall6(60, ok ? 0 : 1, 0, 0, 0, 0, 0);
    for (;;) {}
}

static void check(int condition)
{
    if (!condition)
        finish(0);
}

static int equal(const char *a, const char *b)
{
    while (*a && *a == *b) {
        ++a;
        ++b;
    }
    return *a == *b;
}

static long raw_map(usize address, usize prot)
{
    return syscall6(9, address, 4096, prot, 0x22, ~0UL, 0);
}

static volatile unsigned char *map(usize prot)
{
    long result = raw_map(0, prot);
    check(result > 0);
    return (volatile unsigned char *)result;
}

static void execute(const volatile unsigned char *code)
{
    unsigned char scratch = 0;
    usize target = (usize)&scratch;
    /* Zero-filled executable-only pages end in ADD [RAX],AL. Give that
       instruction valid writable storage, followed by a RET on the next page. */
    __asm__ volatile("call *%1" : "+a"(target) : "r"(code) : "memory", "cc");
}

void test_main(const usize *stack)
{
    check(stack[0] == 2);
    const char *mode = (const char *)stack[2];
    if (equal(mode, "ok")) {
        usize next = 0x600000000000UL;
        for (usize prot = 0; prot < 8; ++prot) {
            volatile unsigned char *p = map(prot);
            check((usize)p == next);
            next += 4096;
            if (prot)
                check(p[0] == 0 && p[4095] == 0);
            if (prot & 2) {
                p[0] = 0xc3; /* RET */
                check(p[0] == 0xc3); /* x86 writes imply reads */
            }
            if (prot & 4) {
                if (prot & 2) {
                    execute(p);
                } else {
                    volatile unsigned char *tail = map(7);
                    check((usize)tail == next);
                    next += 4096;
                    tail[0] = 0xc3;
                    execute(p + 4094);
                }
            }
        }
        for (usize bit = 3; bit < 64; ++bit)
            check(raw_map(0, 3 | (1UL << bit)) == -22);
        check((usize)map(3) == next); /* Rejections do not advance placement. */
        finish(1);
    }

    usize prot;
    int access;
    if (equal(mode, "ro-write")) { prot = 1; access = 1; }
    else if (equal(mode, "exec-write")) { prot = 4; access = 1; }
    else if (equal(mode, "rx-write")) { prot = 5; access = 1; }
    else if (equal(mode, "nx-read")) { prot = 1; access = 2; }
    else if (equal(mode, "nx-write")) { prot = 2; access = 2; }
    else if (equal(mode, "nx-rw")) { prot = 3; access = 2; }
    else if (equal(mode, "none-read")) { prot = 0; access = 0; }
    else if (equal(mode, "none-write")) { prot = 0; access = 1; }
    else if (equal(mode, "none-exec")) { prot = 0; access = 2; }
    else { finish(0); }
    volatile unsigned char *p = map(prot);
    if (prot & 2)
        p[0] = 0xc3;
    say("KRUST_PROTECTION_ARMED ");
    say(mode);
    say("\n");
    if (access == 0) {
        volatile unsigned char value = *p;
        (void)value;
    } else if (access == 1) {
        *p = 0xc3;
    } else {
        execute(p);
    }
    finish(0); /* Any forbidden access returning is a regression. */
}

__asm__(".global _start\n"
        "_start:\n"
        "movq %rsp, %rdi\n"
        "andq $-16, %rsp\n"
        "call test_main\n"
        "ud2\n");
