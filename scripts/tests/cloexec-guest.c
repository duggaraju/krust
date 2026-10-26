/* Build twice: two static images use fixed paths/fd numbers, never argv/envp. */
typedef unsigned long usize;

static long call(long nr, usize a, usize b, usize c, usize d)
{
    register usize r10 __asm__("r10") = d;
    long result;
    __asm__ volatile("syscall" : "=a"(result)
                     : "a"(nr), "D"(a), "S"(b), "d"(c), "r"(r10)
                     : "rcx", "r11", "memory");
    return result;
}

static void message(const char *text)
{
    usize length = 0;
    while (text[length])
        ++length;
    call(1, 1, (usize)text, length, 0);
}

__attribute__((noreturn)) static void finish(int status)
{
    call(60, status, 0, 0, 0);
    for (;;) {}
}

static void check_line(int condition, int line)
{
    if (!condition) {
        char digits[] = "000\n";
        digits[0] += line / 100;
        digits[1] += line / 10 % 10;
        digits[2] += line % 10;
        message("KRUST_CLOEXEC_FAIL line=");
        message(digits);
        finish(1);
    }
}
#define check(condition) check_line((condition), __LINE__)

static long flags(usize fd) { return call(72, fd, 1, 0, 0); }
static long setflags(usize fd, usize value) { return call(72, fd, 2, value, 0); }
#ifndef AFTER_EXEC
static long duplicate(usize fd, usize min, int cloexec)
{
    return call(72, fd, cloexec ? 1030 : 0, min, 0);
}
#endif
static long open_data(usize flags)
{
    return call(2, (usize)"/bin/krust-cloexec-data", flags, 0, 0);
}
static void read_byte(usize fd, char expected)
{
    char byte = 0;
    check(call(0, fd, (usize)&byte, 1, 0) == 1 && byte == expected);
}
static void bad_fd(usize fd)
{
    check(flags(fd) == -9);
    check(setflags(fd, 0) == -9);
    check(setflags(fd, 1) == -9);
}

__attribute__((noreturn)) void _start(void)
{
    bad_fd(20); /* Closed by the shell child's first successful exec. */
    check(flags(21) == 0);
#ifndef AFTER_EXEC
    read_byte(21, 'a');
    bad_fd(~0UL);
    bad_fd(9999);
    check(open_data(0x80000) == 3);
    check(call(257, -100UL, (usize)"/bin/krust-cloexec-data", 0, 0) == 4);
    check(flags(3) == 1 && flags(4) == 0);
    check(call(72, 3, 3, 0, 0) == 0); /* O_CLOEXEC is not a status flag. */
    check(duplicate(3, 5, 0) == 5);
    check(duplicate(3, 6, 1) == 6);
    check(flags(3) == 1 && flags(5) == 0 && flags(6) == 1);
    check(duplicate(3, ~0UL, 0) == -22);
    check(duplicate(9999, ~0UL, 1) == -9);
    check(setflags(4, 0x101) == 0 && flags(4) == 1);
    read_byte(4, 'a'); /* I/O must not erase descriptor flags or access mode. */
    check(call(8, 4, 0, 0, 0) == 0);
    check(flags(4) == 1 && call(72, 4, 3, 0, 0) == 0);
    check(setflags(4, 0x80000) == 0 && flags(4) == 0);
    check(call(257, -100UL, (usize)"/bin/krust-cloexec-data", 0x80000, 0) == 7);
    check(flags(7) == 1);
    check(open_data(0) == 8);
    check(setflags(8, 1) == 0 && flags(8) == 1);
    check(setflags(8, 0) == 0 && flags(8) == 0);
    check(call(3, 7, 0, 0, 0) == 0);
    bad_fd(7);
    check(open_data(0) == 7 && flags(7) == 0);
    check(call(3, 7, 0, 0, 0) == 0);
    check(call(59, (usize)"/bin/krust-cloexec-missing", 0, 0, 0) == -2);
    check(flags(3) == 1 && flags(6) == 1 && flags(4) == 0);
    check(call(59, (usize)"/bin/krust-cloexec-data", 0, 0, 0) == -8);
    check(flags(3) == 1 && flags(6) == 1 && flags(5) == 0);
    read_byte(3, 'a');
    check(flags(3) == 1);
    check(call(3, 0, 0, 0, 0) == 0);
    check(duplicate(3, 0, 0) == 0 && flags(0) == 0);
    check(call(3, 0, 0, 0, 0) == 0);
    check(open_data(0x80000) == 0 && flags(0) == 1);
    message("KRUST_CLOEXEC_BEFORE_OK\n");
    call(59, (usize)"/bin/krust-cloexec-after", 0, 0, 0);
    check(0); /* A successful exec cannot return here. */
#else
    bad_fd(0);
    bad_fd(3);
    bad_fd(6);
    bad_fd(7);
    check(flags(4) == 0 && flags(5) == 0 && flags(8) == 0);
    read_byte(5, 'b'); /* Dup aliases share offsets across exec. */
    read_byte(21, 'b'); /* Fork aliases share offsets across both images. */
    check(setflags(21, 1) == 0 && flags(21) == 1);
    check(open_data(0) == 0 && flags(0) == 0);
    check(open_data(0) == 3 && flags(3) == 0);
    check(setflags(3, 1) == 0);
    check(call(3, 3, 0, 0, 0) == 0);
    check(open_data(0) == 3 && flags(3) == 0);
    message("KRUST_CLOEXEC_EXEC_OK\n");
    finish(0);
#endif
    finish(1);
}
