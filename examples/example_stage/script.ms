import "stdlib" as *;
workspace mainstage_example
{
    example_stage(example_project.test_value);
}

project example_project
{
    test_value = 1;
}

stage example_stage(content)
{
    say("-------------------------");
    say(content);
}