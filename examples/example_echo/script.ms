import "example_rust" as example;

workspace echo_ws {
    val = example.util.echo(["hello", "123"]);
    say(val);
}
