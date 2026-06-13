// Reads a JSON config and turns it into a small report. Demonstrates the
// read-only standard library: `fs` (read files), `json` (extract dotted paths),
// `str` (text manipulation), and `env` (environment with a default), plus an
// `if/else` expression driven by an `env(...)` condition.
import "env" as env;
import "fs" as fs;
import "json" as json;
import "str" as str;

project {
    name: "data-report"
    version: "0.1.0"
}

let config = fs.read("config.json");
let app = json.get(config, "app.name");
let port = json.get(config, "app.port");
let title = str.upper(app);

let env_name = env.get("APP_ENV", default: "development");
let banner = if env("APP_ENV") == "production" { "PRODUCTION BUILD" } else { "dev build" };

default pipeline report {
    stages: [render]
}

stage render {
    inputs: ["config.json"]
    outputs: ["dist/report.txt"]

    steps {
        mkdir "dist/"
        write "dist/report.txt" content: "${banner}\n${title} on :${port} (${env_name})"
    }
}
