/*
 * Fast-path wrapper for period.exe.
 *
 * For trivial programs such as `show "Hello, World!".` this executable
 * prints the output directly and exits without loading the full Rust
 * interpreter. All other programs are forwarded to a persistent
 * period-core worker process, avoiding the cost of loading the DLL and
 * re-JITting on every invocation.
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

static int worker_port(void) {
    const char *env = getenv("PERIOD_WORKER_PORT");
    if (env) {
        int p = atoi(env);
        if (p > 0 && p <= 65535) return p;
    }
    return 52691;
}

/* Minimal Winsock types loaded dynamically so we do not depend on the
 * platform SDK headers shipped with tiny compilers. */
typedef UINT_PTR SOCKET;
#define INVALID_SOCKET ((SOCKET)(~0))
#define SOCKET_ERROR (-1)
#define AF_INET 2
#define SOCK_STREAM 1
#define IPPROTO_TCP 6

struct ws2_in_addr {
    unsigned long s_addr;
};

struct ws2_sockaddr_in {
    short sin_family;
    unsigned short sin_port;
    struct ws2_in_addr sin_addr;
    char sin_zero[8];
};

struct ws2_WSAData {
    WORD wVersion;
    WORD wHighVersion;
    char szDescription[257];
    char szSystemStatus[129];
    unsigned short iMaxSockets;
    unsigned short iMaxUdpDg;
    char *lpVendorInfo;
};

typedef SOCKET (WINAPI *fn_socket)(int af, int type, int protocol);
typedef int (WINAPI *fn_connect)(SOCKET s, const void *name, int namelen);
typedef int (WINAPI *fn_send)(SOCKET s, const char *buf, int len, int flags);
typedef int (WINAPI *fn_recv)(SOCKET s, char *buf, int len, int flags);
typedef int (WINAPI *fn_closesocket)(SOCKET s);
typedef int (WINAPI *fn_WSAStartup)(WORD wVersionRequested, struct ws2_WSAData *lpWSAData);

static struct {
    HMODULE mod;
    fn_socket socket;
    fn_connect connect;
    fn_send send;
    fn_recv recv;
    fn_closesocket closesocket;
    fn_WSAStartup WSAStartup;
} ws = { 0 };

static unsigned short my_htons(unsigned short v) {
    return (unsigned short)(((v & 0xffu) << 8) | ((v >> 8) & 0xffu));
}

static int load_winsock(void) {
    if (ws.mod) return 0;
    ws.mod = LoadLibraryA("ws2_32.dll");
    if (!ws.mod) return -1;
    ws.socket = (fn_socket)GetProcAddress(ws.mod, "socket");
    ws.connect = (fn_connect)GetProcAddress(ws.mod, "connect");
    ws.send = (fn_send)GetProcAddress(ws.mod, "send");
    ws.recv = (fn_recv)GetProcAddress(ws.mod, "recv");
    ws.closesocket = (fn_closesocket)GetProcAddress(ws.mod, "closesocket");
    ws.WSAStartup = (fn_WSAStartup)GetProcAddress(ws.mod, "WSAStartup");
    if (!ws.socket || !ws.connect || !ws.send || !ws.recv || !ws.closesocket || !ws.WSAStartup) {
        FreeLibrary(ws.mod);
        ws.mod = NULL;
        return -1;
    }
    struct ws2_WSAData wsa;
    if (ws.WSAStartup(MAKEWORD(2, 2), &wsa) != 0) {
        FreeLibrary(ws.mod);
        ws.mod = NULL;
        return -1;
    }
    return 0;
}

static int send_all(SOCKET s, const char *buf, size_t len) {
    size_t sent = 0;
    while (sent < len) {
        int n = ws.send(s, buf + sent, (int)(len - sent), 0);
        if (n <= 0) return -1;
        sent += (size_t)n;
    }
    return 0;
}

static int recv_all(SOCKET s, char *buf, size_t len) {
    size_t got = 0;
    while (got < len) {
        int n = ws.recv(s, buf + got, (int)(len - got), 0);
        if (n <= 0) return -1;
        got += (size_t)n;
    }
    return 0;
}

static int start_worker(void) {
    find_core_exe();

    char cmdline[512];
    snprintf(cmdline, sizeof(cmdline), "\"%s\" --server", core_path);

    STARTUPINFOA si = { sizeof(si) };
    PROCESS_INFORMATION pi = { 0 };

    if (!CreateProcessA(NULL, cmdline, NULL, NULL, FALSE,
                        CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP,
                        NULL, NULL, &si, &pi)) {
        return -1;
    }
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);
    return 0;
}

/* Try to run a single file through the persistent worker process.
 * Returns the worker's exit code, or -1 if the worker could not be used.
 */
static int run_via_worker(const char *path) {
    if (load_winsock() != 0) return -1;

    char abs_path[MAX_PATH];
    if (!GetFullPathNameA(path, MAX_PATH, abs_path, NULL)) {
        strcpy(abs_path, path);
    }

    SOCKET sock = INVALID_SOCKET;
    int attempts = 0;
    while (attempts < 25) {
        sock = ws.socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
        if (sock == INVALID_SOCKET) return -1;

        struct ws2_sockaddr_in addr = { 0 };
        addr.sin_family = (unsigned short)AF_INET;
        addr.sin_port = my_htons((unsigned short)worker_port());
        addr.sin_addr.s_addr = 0x0100007f; /* 127.0.0.1 in network order */

        if (ws.connect(sock, &addr, sizeof(addr)) == 0) {
            break;
        }
        ws.closesocket(sock);
        sock = INVALID_SOCKET;

        if (attempts == 0) {
            if (start_worker() != 0) return -1;
        }
        Sleep(40);
        attempts++;
    }
    if (sock == INVALID_SOCKET) return -1;

    uint64_t path_len = (uint64_t)strlen(abs_path);
    char len_buf[8];
    memcpy(len_buf, &path_len, sizeof(len_buf));
    if (send_all(sock, len_buf, 8) != 0 || send_all(sock, abs_path, path_len) != 0) {
        ws.closesocket(sock);
        return -1;
    }

    char resp[12];
    if (recv_all(sock, resp, 12) != 0) {
        ws.closesocket(sock);
        return -1;
    }
    int32_t exit_code;
    memcpy(&exit_code, resp, sizeof(exit_code));
    uint64_t out_len;
    memcpy(&out_len, resp + 4, sizeof(out_len));

    char *output = NULL;
    if (out_len > 0 && out_len < 64 * 1024 * 1024) {
        output = (char *)malloc((size_t)out_len + 1);
        if (output) {
            if (recv_all(sock, output, (size_t)out_len) != 0) {
                free(output);
                ws.closesocket(sock);
                return -1;
            }
            output[out_len] = '\0';
        }
    }
    ws.closesocket(sock);

    if (output) {
        fwrite(output, 1, (size_t)out_len, stdout);
        free(output);
    }
    return (int)exit_code;
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
        strcmp(argv[1], "--lsp") == 0 || strcmp(argv[1], "--server") == 0) {
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
        result = run_via_worker(argv[1]);
        if (result < 0) {
            result = run_core(argc, argv);
        } else if (result != 0) {
            /* On error run locally so the user sees diagnostics. */
            int local = run_core(argc, argv);
            if (local != 0) result = local;
        }
    }

    free(buf);
    return result;
}
