import "cpp" as cpp_plugin;

workspace ws {
    val = cpp_plugin.list_compilers();
    say(val);
}
