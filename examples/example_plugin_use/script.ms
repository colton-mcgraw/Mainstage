import "stdlib" as *;
import "cpp" as cpp_plugin;

workspace hello_ws {
    // Read source files from the example_plugin_use project and store as an array.
    say(example_plugin_use.srcs);
    glb = glob(example_plugin_use.srcs);
    say("Globbed source files:");
    say(glb);
    sources = read(glb[0]);
    // Debug output
    say("Compiling C++ sources:");
    say(sources);
    // Compile using the cpp plugin
    val = cpp_plugin.compile(sources, example_plugin_use.flags);
    if val.ok {
        say("Compilation succeeded. Output binary at: " + val.path);
    } else {
        say("Compilation failed with error: " + val.error);
    }
    // Debug
    say(val);
}

project example_plugin_use {
    srcs = [ "./src/*.cpp" ];
    flags = ["-O2"];
}