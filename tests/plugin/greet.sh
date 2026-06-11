#!/bin/sh
# A minimal Mainstage subprocess plugin used by the Phase 15 example/integration
# tests. It speaks the newline-delimited JSON protocol over stdio: one request per
# line in, one response line out.
#
# Methods:
#   hello(name: string) -> string   returns "hello, <name>"
#   echo_num(n: int)     -> int      returns <n> unchanged (exercises the int wire type)

while IFS= read -r line; do
  case "$line" in
    *'"op":"describe"'*)
      printf '%s\n' '{"name":"greet","methods":[{"name":"hello","params":[{"name":"name","type":"string","required":true}],"returns":"string"},{"name":"echo_num","params":[{"name":"n","type":"int","required":true}],"returns":"int"}]}'
      ;;
    *'"method":"hello"'*)
      # Extract the string value from the first argument and prefix a greeting.
      name=$(printf '%s' "$line" | sed 's/.*"value":"\([^"]*\)"}.*/\1/')
      printf '%s\n' "{\"ok\":{\"type\":\"string\",\"value\":\"hello, $name\"}}"
      ;;
    *'"method":"echo_num"'*)
      # Extract the (unquoted) integer value and echo it back as an int.
      n=$(printf '%s' "$line" | sed 's/.*"value":\(-\{0,1\}[0-9]*\)}.*/\1/')
      printf '%s\n' "{\"ok\":{\"type\":\"int\",\"value\":$n}}"
      ;;
    *)
      printf '%s\n' '{"err":"unknown method"}'
      ;;
  esac
done
