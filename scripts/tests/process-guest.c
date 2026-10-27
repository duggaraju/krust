typedef unsigned long usize;

static long call(long nr, usize a, usize b, usize c, usize d, usize e, usize f)
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

static void message(const char *text)
{
    usize length = 0;
    while (text[length]) ++length;
    call(1, 1, (usize)text, length, 0, 0, 0);
}

__attribute__((noreturn)) static void finish(int status)
{
    call(60, status, 0, 0, 0, 0, 0);
    for (;;) {}
}

static void check_line(int condition, int line)
{
    if (!condition) {
        char digits[] = "000\n";
        digits[0] += line / 100;
        digits[1] += line / 10 % 10;
        digits[2] += line % 10;
        message("KRUST_PROCESS_FAIL line=");
        message(digits);
        finish(1);
    }
}
#define check(condition) check_line((condition), __LINE__)

static int equal(const char *a, const char *b)
{
    while (*a && *a == *b) { ++a; ++b; }
    return *a == *b;
}

static long fork_registers(void)
{
    register usize r12 __asm__("r12") = 0x12345678;
    register usize r13 __asm__("r13") = 0x23456789;
    register usize r14 __asm__("r14") = 0x34567890;
    register usize r15 __asm__("r15") = 0x45678901;
    long result;
    __asm__ volatile("syscall" : "=a"(result), "+r"(r12), "+r"(r13), "+r"(r14), "+r"(r15)
                     : "a"(57) : "rcx", "r11", "memory");
    check(r12 == 0x12345678 && r13 == 0x23456789 &&
          r14 == 0x34567890 && r15 == 0x45678901);
    return result;
}

static volatile int global_value = 19;

__attribute__((noreturn)) void process_main(usize *stack)
{
    usize argc = stack[0];
    char **argv = (char **)(stack + 1);
    if (argc && (equal(argv[0], "replacement") || equal(argv[0], "replacement-empty"))) {
        check(argc == 3 && equal(argv[1], "") && equal(argv[2], "argument"));
        check(argv[3] == 0);
        char **env = argv + argc + 1;
        if (equal(argv[0], "replacement-empty"))
            check(env[0] == 0);
        else
            check(env[0] && equal(env[0], "TOKEN=kept") && env[1] == 0);
        finish(42);
    }
    const char *path = "/bin/proctest";
    char *bad_string[] = {(char *)1, 0};
    char *args[] = {"replacement", "", "argument", 0};
    char *env[] = {"TOKEN=kept", 0};
    check(call(59, 1, 0, 0, 0, 0, 0) == -14);
    check(call(59, (usize)path, 1, 0, 0, 0, 0) == -14);
    check(call(59, (usize)path, (usize)bad_string, 0, 0, 0, 0) == -14);
    check(call(59, (usize)path, (usize)args, 1, 0, 0, 0) == -14);
    check(call(59, (usize)"/bin/missing", (usize)args, (usize)env, 0, 0, 0) == -2);
    char *memory = (char *)call(9, 0, 128 * 1024, 3, 0x22, ~0UL, 0);
    check((long)memory > 0);
    for (usize index = 0; index < 128 * 1024; ++index) memory[index] = 'x';
    char *huge[] = {memory, 0};
    check(call(59, (usize)path, (usize)huge, 0, 0, 0, 0) == -7);
    /* A NUL-terminated vector need not have natural alignment or stay in one page. */
    char **cross = (char **)(memory + 4092);
    cross[0] = args[0]; cross[1] = args[1]; cross[2] = args[2]; cross[3] = 0;
    struct { int status; unsigned guard; } result = {-1, 0xfeedcafe};
    check(call(61, ~0UL, (usize)&result.status, 1, 0, 0, 0) == -10);
    check(call(61, ~0UL, (usize)&result.status, 2, 0, 0, 0) == -22);
    for (int iteration = 0; iteration < 3; ++iteration) {
        volatile int local_value = 23;
        global_value = 19;
        memory[12000] = 31;
        cross[0] = iteration == 2 ? "replacement-empty" : args[0];
        long child = fork_registers();
        check(child >= 0);
        if (child == 0) {
            check(local_value == 23 && global_value == 19 && memory[12000] == 31);
            local_value = 99; global_value = 98; memory[12000] = 97;
            call(59, (usize)path, (usize)cross, iteration == 2 ? 0 : (usize)env, 0, 0, 0);
            check(0);
        }
        check(local_value == 23 && global_value == 19 && memory[12000] == 31);
        check(call(61, child, 1, 0, 0, 0, 0) == -14);
        check(call(61, child, (usize)&result.status, 0, 0, 0, 0) == child);
        check(result.status == (42 << 8) && result.guard == 0xfeedcafe);
        check(call(61, child, (usize)&result.status, 1, 0, 0, 0) == -10);
    }
    long child = fork_registers();
    check(child >= 0);
    if (child == 0) {
        __asm__ volatile("movq $1, (%0)" : : "r"(0UL) : "memory");
        check(0);
    }
    check(call(61, child, (usize)&result.status, 0, 0, 0, 0) == child);
    check(result.status == (139 << 8) && result.guard == 0xfeedcafe);
    check(call(39, 0, 0, 0, 0, 0, 0) > 0);
    message("KRUST_PROCESS_OK\n");
    finish(0);
}

__asm__(".global _start\n_start:\nmov %rsp,%rdi\nand $-16,%rsp\ncall process_main\n");
