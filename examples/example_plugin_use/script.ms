import "stdlib" as *;
import "cpp" as cpp_plugin;

workspace hello_ws {
    glb = glob(example_plugin_use.srcs);            // Array<String> of paths
    contents = [];                                  // Array<String or Null>
    for p in glb {
        say("Reading file: " + p);
        c = read(p);                                // String or Null
        contents = append(contents, c);             // accumulate contents
    }
    
    // Use contents as needed, e.g., compile the first
    val = cpp_plugin.compile(contents, example_plugin_use.flags);
    if val.ok {
        say("Compilation succeeded. Output binary at: " + val.path);
    } else {
        say("Compilation failed with error: " + val.error);
    }
    say(val);
}

project example_plugin_use {
    srcs = [ "./src/*.cpp" ];
    flags = ["-O2"];
}