#include <stdlib.h>
#include <stdio.h>
#include <string.h>

#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT
#endif

EXPORT const char* plugin_name(void) {
    return "c_plugin_example";
}

EXPORT char* plugin_call_json(const char* func, const char* args_json) {
    if (!func) func = "";
    if (!args_json) args_json = "null";
    const char* fmt = "{\"result\":\"ok\",\"func\":\"%s\",\"args\":%s}";
    int needed = snprintf(NULL, 0, fmt, func, args_json) + 1;
    char* buf = (char*)malloc((size_t)needed);
    if (!buf) return NULL;
    snprintf(buf, (size_t)needed, fmt, func, args_json);
    return buf;
}

EXPORT void plugin_free(char* ptr) {
    if (!ptr) return;
    free(ptr);
}
