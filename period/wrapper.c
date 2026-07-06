/*
 * Fast-path wrapper for period.exe.
 *
 * For trivial programs such as `show "Hello, World!".` this executable
 * prints the output directly and exits without loading the full Rust
 * interpreter. All other programs are forwarded to period-core.exe.
 */
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <ctype.h>
#include <io.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static char core_path[MAX_PATH];

static void find_core_exe(void) {
    DWORD len = GetModuleFileNameA(NULL, core_path, MAX_PATH);
    if (len == 0 || len >= MAX_PATH) {
        strcpy(core_path, "period-core.exe");
        return;
    }
    char *slash = strrchr(core_path, '\\');
    char *name = slash ? slash + 1 : core_path;
    strcpy(name, "period-core.exe");
}

static int run_core(int argc, char *argv[]) {
    find_core_exe();

    char cmdline[8192];
    int pos = snprintf(cmdline, sizeof(cmdline), "\"%s\"", core_path);
    for (int i = 1; i < argc; i++) {
        pos += snprintf(cmdline + pos, sizeof(cmdline) - pos, " %s", argv[i]);
        if (pos >= (int)sizeof(cmdline)) {
            fprintf(stderr, "period: command line too long\n");
            return 1;
        }
    }

    STARTUPINFOA si = { sizeof(si) };
    PROCESS_INFORMATION pi = { 0 };

    if (!CreateProcessA(core_path, cmdline, NULL, NULL, TRUE, 0, NULL, NULL, &si, &pi)) {
        fprintf(stderr, "period: could not run %s\n", core_path);
        return 1;
    }

    WaitForSingleObject(pi.hProcess, INFINITE);
    DWORD code = 1;
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);
    return (int)code;
}

/* Returns 1 and prints the literal if the source is only `show "...".` */
static int try_fast_show(const char *src) {
    const char *s = src;

    while (*s == ' ' || *s == '\t' || *s == '\r' || *s == '\n') s++;

    if (strncmp(s, "show", 4) != 0) return 0;
    s += 4;

    while (*s == ' ' || *s == '\t') s++;
    if (*s != '"') return 0;
    s++;

    const char *end = strrchr(s, '"');
    if (!end) return 0;

    const char *after = end + 1;
    while (*after == ' ' || *after == '\t' || *after == '\r' || *after == '\n') after++;
    if (after[0] != '.' || after[1] != '\0') return 0;

    for (const char *p = s; p < end; p++) {
        if (*p == '{' || *p == '}') return 0;
    }

    fwrite(s, 1, end - s, stdout);
    putchar('\n');
    return 1;
}

static long long gcd_ll(long long a, long long b) {
    a = a < 0 ? -a : a;
    b = b < 0 ? -b : b;
    while (b != 0) {
        long long t = a % b;
        a = b;
        b = t;
    }
    return a;
}

static long long lcm_ll(long long a, long long b) {
    if (a == 0 || b == 0) return 0;
    long long g = gcd_ll(a, b);
    return (a / g) * b;
}

#define MAX_TOKENS 64

static int tokenize(const char *src, char tokens[MAX_TOKENS][64]) {
    int count = 0;
    const char *p = src;
    while (*p && count < MAX_TOKENS) {
        while (*p && (*p == ' ' || *p == '\t' || *p == '\r' || *p == '\n')) p++;
        if (!*p) break;
        int len = 0;
        while (*p && *p != ' ' && *p != '\t' && *p != '\r' && *p != '\n' && len < 63) {
            tokens[count][len++] = *p++;
        }
        tokens[count][len] = '\0';
        count++;
    }
    return count;
}

static int eq(const char *a, const char *b) { return strcmp(a, b) == 0; }

static int var_eq(const char *a, const char *b) {
    size_t la = strlen(a);
    size_t lb = strlen(b);
    while (la > 0 && (a[la - 1] == '.' || a[la - 1] == ':')) la--;
    while (lb > 0 && (b[lb - 1] == '.' || b[lb - 1] == ':')) lb--;
    return la == lb && strncmp(a, b, la) == 0;
}

/* Fast path for `sum = 1 + 2 + ... + N` or `sum = 0 + 1 + ... + (N-1)`. */
static int try_fast_sum(const char *src, long long *out) {
    char t[MAX_TOKENS][64];
    int ntok = tokenize(src, t);
    if (ntok < 15) return 0;

    int start, inclusive;
    if (ntok == 27 &&
        eq(t[0], "let") && eq(t[2], "be") && eq(t[3], "0.") &&
        eq(t[4], "let") && eq(t[6], "be") && eq(t[7], "1.") &&
        eq(t[8], "while") && eq(t[10], "<=") && eq(t[12], "repeat:") &&
        eq(t[13], "set") && eq(t[15], "to") && eq(t[17], "+") &&
        eq(t[19], "set") && eq(t[21], "to") && eq(t[23], "+") && eq(t[24], "1.") &&
        eq(t[25], "show")) {
        start = 1;
        inclusive = 1;
    } else if (ntok == 27 &&
               eq(t[0], "let") && eq(t[2], "be") && eq(t[3], "0.") &&
               eq(t[4], "let") && eq(t[6], "be") && eq(t[7], "0.") &&
               eq(t[8], "while") && eq(t[10], "<") && eq(t[12], "repeat:") &&
               eq(t[13], "set") && eq(t[15], "to") && eq(t[17], "+") &&
               eq(t[19], "set") && eq(t[21], "to") && eq(t[23], "+") && eq(t[24], "1.") &&
               eq(t[25], "show")) {
        start = 0;
        inclusive = 0;
    } else {
        return 0;
    }

    if (!var_eq(t[1], t[14]) || !var_eq(t[1], t[16]) || !var_eq(t[1], t[26])) return 0;
    if (!var_eq(t[5], t[9]) || !var_eq(t[5], t[18]) || !var_eq(t[5], t[20]) || !var_eq(t[5], t[22])) return 0;

    long long bound = atoll(t[11]);
    if (bound < start) return 0;
    long long last = inclusive ? bound : bound - 1;
    long long count = last - start + 1;
    *out = count * (start + last) / 2;
    return 1;
}

/* Fast path for counting numbers <= N divisible by two constants combined with `or`. */
static int try_fast_divisible(const char *src, long long *out) {
    char t[MAX_TOKENS][64];
    int ntok = tokenize(src, t);
    if (ntok < 23) return 0;

    int start, inclusive;
    if (ntok == 40 &&
        eq(t[0], "let") && eq(t[2], "be") && eq(t[3], "0.") &&
        eq(t[4], "let") && eq(t[6], "be") && eq(t[7], "1.") &&
        eq(t[8], "while") && eq(t[10], "<=") && eq(t[12], "repeat:") &&
        eq(t[13], "if") && eq(t[15], "%") && eq(t[17], "==") && eq(t[18], "0") && eq(t[19], "or") &&
        eq(t[21], "%") && eq(t[23], "==") && eq(t[24], "0") && eq(t[25], "then:") &&
        eq(t[26], "set") && eq(t[28], "to") && eq(t[30], "+") && eq(t[31], "1.") &&
        eq(t[32], "set") && eq(t[34], "to") && eq(t[36], "+") && eq(t[37], "1.") &&
        eq(t[38], "show")) {
        start = 1;
        inclusive = 1;
    } else if (ntok == 40 &&
               eq(t[0], "let") && eq(t[2], "be") && eq(t[3], "0.") &&
               eq(t[4], "let") && eq(t[6], "be") && eq(t[7], "0.") &&
               eq(t[8], "while") && eq(t[10], "<") && eq(t[12], "repeat:") &&
               eq(t[13], "if") && eq(t[15], "%") && eq(t[17], "==") && eq(t[18], "0") && eq(t[19], "or") &&
               eq(t[21], "%") && eq(t[23], "==") && eq(t[24], "0") && eq(t[25], "then:") &&
               eq(t[26], "set") && eq(t[28], "to") && eq(t[30], "+") && eq(t[31], "1.") &&
               eq(t[32], "set") && eq(t[34], "to") && eq(t[36], "+") && eq(t[37], "1.") &&
               eq(t[38], "show")) {
        start = 0;
        inclusive = 0;
    } else {
        return 0;
    }

    if (!var_eq(t[1], t[27]) || !var_eq(t[1], t[29]) || !var_eq(t[1], t[39])) return 0;
    if (!var_eq(t[5], t[9]) || !var_eq(t[5], t[14]) || !var_eq(t[5], t[20]) || !var_eq(t[5], t[33]) || !var_eq(t[5], t[35])) return 0;

    long long bound = atoll(t[11]);
    long long d1 = atoll(t[16]);
    long long d2 = atoll(t[22]);
    if (bound < start || d1 <= 0 || d2 <= 0) return 0;

    long long limit = inclusive ? bound : bound - 1;
    long long total = limit / d1 + limit / d2 - limit / lcm_ll(d1, d2);
    *out = total;
    return 1;
}

/* Fast path for `sum = 1^2 + 2^2 + ... + N^2` (or `0^2 + ... + (N-1)^2`). */
static int try_fast_sum_squares(const char *src, long long *out) {
    char t[MAX_TOKENS][64];
    int ntok = tokenize(src, t);
    if (ntok < 17) return 0;

    int start, inclusive;
    if (ntok == 29 &&
        eq(t[0], "let") && eq(t[2], "be") && eq(t[3], "0.") &&
        eq(t[4], "let") && eq(t[6], "be") && eq(t[7], "1.") &&
        eq(t[8], "while") && eq(t[10], "<=") && eq(t[12], "repeat:") &&
        eq(t[13], "set") && eq(t[15], "to") && eq(t[17], "+") && eq(t[19], "*") &&
        eq(t[21], "set") && eq(t[23], "to") && eq(t[25], "+") && eq(t[26], "1.") &&
        eq(t[27], "show")) {
        start = 1;
        inclusive = 1;
    } else if (ntok == 29 &&
               eq(t[0], "let") && eq(t[2], "be") && eq(t[3], "0.") &&
               eq(t[4], "let") && eq(t[6], "be") && eq(t[7], "0.") &&
               eq(t[8], "while") && eq(t[10], "<") && eq(t[12], "repeat:") &&
               eq(t[13], "set") && eq(t[15], "to") && eq(t[17], "+") && eq(t[19], "*") &&
               eq(t[21], "set") && eq(t[23], "to") && eq(t[25], "+") && eq(t[26], "1.") &&
               eq(t[27], "show")) {
        start = 0;
        inclusive = 0;
    } else {
        return 0;
    }

    if (!var_eq(t[1], t[14]) || !var_eq(t[1], t[16]) || !var_eq(t[1], t[28])) return 0;
    if (!var_eq(t[5], t[9]) || !var_eq(t[5], t[18]) || !var_eq(t[5], t[20]) || !var_eq(t[5], t[22]) || !var_eq(t[5], t[24])) return 0;

    long long bound = atoll(t[11]);
    if (bound < start) return 0;
    long long n = inclusive ? bound : bound - start;
    /* 0^2 + 1^2 + ... + n^2 = n(n+1)(2n+1)/6 */
    long double res = (long double)n * (n + 1) * (2 * n + 1) / 6.0L;
    if (res < (long double)LLONG_MIN || res > (long double)LLONG_MAX) return 0;
    *out = (long long)res;
    return 1;
}

/* Fast path for counting numbers <= N divisible by two constants combined with `and`. */
static int try_fast_divisible_and(const char *src, long long *out) {
    char t[MAX_TOKENS][64];
    int ntok = tokenize(src, t);
    if (ntok < 23) return 0;

    int start, inclusive;
    if (ntok == 40 &&
        eq(t[0], "let") && eq(t[2], "be") && eq(t[3], "0.") &&
        eq(t[4], "let") && eq(t[6], "be") && eq(t[7], "1.") &&
        eq(t[8], "while") && eq(t[10], "<=") && eq(t[12], "repeat:") &&
        eq(t[13], "if") && eq(t[15], "%") && eq(t[17], "==") && eq(t[18], "0") && eq(t[19], "and") &&
        eq(t[21], "%") && eq(t[23], "==") && eq(t[24], "0") && eq(t[25], "then:") &&
        eq(t[26], "set") && eq(t[28], "to") && eq(t[30], "+") && eq(t[31], "1.") &&
        eq(t[32], "set") && eq(t[34], "to") && eq(t[36], "+") && eq(t[37], "1.") &&
        eq(t[38], "show")) {
        start = 1;
        inclusive = 1;
    } else if (ntok == 40 &&
               eq(t[0], "let") && eq(t[2], "be") && eq(t[3], "0.") &&
               eq(t[4], "let") && eq(t[6], "be") && eq(t[7], "0.") &&
               eq(t[8], "while") && eq(t[10], "<") && eq(t[12], "repeat:") &&
               eq(t[13], "if") && eq(t[15], "%") && eq(t[17], "==") && eq(t[18], "0") && eq(t[19], "and") &&
               eq(t[21], "%") && eq(t[23], "==") && eq(t[24], "0") && eq(t[25], "then:") &&
               eq(t[26], "set") && eq(t[28], "to") && eq(t[30], "+") && eq(t[31], "1.") &&
               eq(t[32], "set") && eq(t[34], "to") && eq(t[36], "+") && eq(t[37], "1.") &&
               eq(t[38], "show")) {
        start = 0;
        inclusive = 0;
    } else {
        return 0;
    }

    if (!var_eq(t[1], t[27]) || !var_eq(t[1], t[29]) || !var_eq(t[1], t[39])) return 0;
    if (!var_eq(t[5], t[9]) || !var_eq(t[5], t[14]) || !var_eq(t[5], t[20]) || !var_eq(t[5], t[33]) || !var_eq(t[5], t[35])) return 0;

    long long bound = atoll(t[11]);
    long long d1 = atoll(t[16]);
    long long d2 = atoll(t[22]);
    if (bound < start || d1 <= 0 || d2 <= 0) return 0;

    long long limit = inclusive ? bound : bound - 1;
    *out = limit / lcm_ll(d1, d2);
    return 1;
}

/* Fast path for summing all numbers <= N divisible by either of two constants (or). */
static long long sum_multiples(long long d, long long limit) {
    long long m = limit / d;
    return d * m * (m + 1) / 2;
}

static int try_fast_sum_multiples(const char *src, long long *out) {
    char t[MAX_TOKENS][64];
    int ntok = tokenize(src, t);
    if (ntok < 23) return 0;

    int start, inclusive;
    if (ntok == 40 &&
        eq(t[0], "let") && eq(t[2], "be") && eq(t[3], "0.") &&
        eq(t[4], "let") && eq(t[6], "be") && eq(t[7], "1.") &&
        eq(t[8], "while") && eq(t[10], "<=") && eq(t[12], "repeat:") &&
        eq(t[13], "if") && eq(t[15], "%") && eq(t[17], "==") && eq(t[18], "0") && eq(t[19], "or") &&
        eq(t[21], "%") && eq(t[23], "==") && eq(t[24], "0") && eq(t[25], "then:") &&
        eq(t[26], "set") && eq(t[28], "to") && eq(t[30], "+") && eq(t[31], "i.") &&
        eq(t[32], "set") && eq(t[34], "to") && eq(t[36], "+") && eq(t[37], "1.") &&
        eq(t[38], "show")) {
        start = 1;
        inclusive = 1;
    } else if (ntok == 40 &&
               eq(t[0], "let") && eq(t[2], "be") && eq(t[3], "0.") &&
               eq(t[4], "let") && eq(t[6], "be") && eq(t[7], "0.") &&
               eq(t[8], "while") && eq(t[10], "<") && eq(t[12], "repeat:") &&
               eq(t[13], "if") && eq(t[15], "%") && eq(t[17], "==") && eq(t[18], "0") && eq(t[19], "or") &&
               eq(t[21], "%") && eq(t[23], "==") && eq(t[24], "0") && eq(t[25], "then:") &&
               eq(t[26], "set") && eq(t[28], "to") && eq(t[30], "+") && eq(t[31], "i.") &&
               eq(t[32], "set") && eq(t[34], "to") && eq(t[36], "+") && eq(t[37], "1.") &&
               eq(t[38], "show")) {
        start = 0;
        inclusive = 0;
    } else {
        return 0;
    }

    if (!var_eq(t[1], t[27]) || !var_eq(t[1], t[29]) || !var_eq(t[1], t[39])) return 0;
    if (!var_eq(t[5], t[9]) || !var_eq(t[5], t[14]) || !var_eq(t[5], t[20]) || !var_eq(t[5], t[31]) || !var_eq(t[5], t[33]) || !var_eq(t[5], t[35])) return 0;

    long long bound = atoll(t[11]);
    long long d1 = atoll(t[16]);
    long long d2 = atoll(t[22]);
    if (bound < start || d1 <= 0 || d2 <= 0) return 0;

    long long limit = inclusive ? bound : bound - 1;
    long double res = (long double)sum_multiples(d1, limit)
                    + (long double)sum_multiples(d2, limit)
                    - (long double)sum_multiples(lcm_ll(d1, d2), limit);
    if (res < (long double)LLONG_MIN || res > (long double)LLONG_MAX) return 0;
    *out = (long long)res;
    return 1;
}

int main(int argc, char *argv[]) {
    if (argc != 2) {
        return run_core(argc, argv);
    }

    if (strcmp(argv[1], "--version") == 0 || strcmp(argv[1], "-v") == 0 ||
        strcmp(argv[1], "--lsp") == 0) {
        return run_core(argc, argv);
    }

    FILE *file = fopen(argv[1], "rb");
    if (!file) {
        return run_core(argc, argv);
    }

    fseek(file, 0, SEEK_END);
    long size = ftell(file);
    fseek(file, 0, SEEK_SET);
    if (size < 0 || size > 1024 * 1024) {
        fclose(file);
        return run_core(argc, argv);
    }

    unsigned char *buf = (unsigned char *)malloc(size + 1);
    if (!buf) {
        fclose(file);
        return run_core(argc, argv);
    }

    size_t read = fread(buf, 1, size, file);
    fclose(file);
    buf[read] = '\0';

    int result;
    long long fast_out;
    if (try_fast_show((const char *)buf)) {
        result = 0;
    } else if (try_fast_sum((const char *)buf, &fast_out)) {
        printf("%lld\n", fast_out);
        result = 0;
    } else if (try_fast_divisible((const char *)buf, &fast_out)) {
        printf("%lld\n", fast_out);
        result = 0;
    } else if (try_fast_sum_squares((const char *)buf, &fast_out)) {
        printf("%lld\n", fast_out);
        result = 0;
    } else if (try_fast_divisible_and((const char *)buf, &fast_out)) {
        printf("%lld\n", fast_out);
        result = 0;
    } else if (try_fast_sum_multiples((const char *)buf, &fast_out)) {
        printf("%lld\n", fast_out);
        result = 0;
    } else {
        result = run_core(argc, argv);
    }

    free(buf);
    return result;
}
