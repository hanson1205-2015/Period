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

static int run_via_dll(int argc, char *argv[]) {
    (void)argc;
    (void)argv;

    char dll_path[MAX_PATH];
    DWORD len = GetModuleFileNameA(NULL, dll_path, MAX_PATH);
    if (len == 0 || len >= MAX_PATH) {
        return -1;
    }
    char *slash = strrchr(dll_path, '\\');
    char *name = slash ? slash + 1 : dll_path;
    strcpy(name, "period-core.dll");

    HMODULE dll = LoadLibraryA(dll_path);
    if (!dll) {
        return -1;
    }

    typedef int (*run_fn)(void);
    run_fn fn = (run_fn)GetProcAddress(dll, "period_run");
    if (!fn) {
        FreeLibrary(dll);
        return -1;
    }

    int result = fn();
    FreeLibrary(dll);
    return result;
}

static int run_core(int argc, char *argv[]) {
    int dll_result = run_via_dll(argc, argv);
    if (dll_result >= 0) {
        return dll_result;
    }

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
    if (try_fast_show((const char *)buf)) {
        result = 0;
    } else {
        result = run_core(argc, argv);
    }

    free(buf);
    return result;
}
