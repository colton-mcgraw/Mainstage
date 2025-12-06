import "stdlib" as *;

workspace typed_array_ops {
    a = [];
    say("start len " + a.length);
    b = append(a, "x");
    say("b len " + b.length);
    say("a len " + a.length);
    c = extend(b, ["y", "z"]);
    say("c len " + c.length);
    say("b len " + b.length);
    d = append(a, "w");
    say("d len " + d.length);
    say("a len " + a.length);
}
