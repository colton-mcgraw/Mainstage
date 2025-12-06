import "stdlib" as *;
import "cpp" as cpp_plugin;

workspace hello_ws {
    contents = gather_sources(example_plugin_use);
    say("Gathered " + contents.length + " source files.");
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

stage gather_sources(prj) {
    glb = glob(prj.srcs);            // Array<String> of paths
    say("Found " + glb.length + " files.");
    contents = [];                  // Array<String or Null>
    for p in glb {
        c = read(p);                // String or Null
        contents = append(contents, c); // accumulate contents
    }
    return contents;
}