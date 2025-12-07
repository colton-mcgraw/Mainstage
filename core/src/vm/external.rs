//! file: core/src/vm/external.rs
//! description: external (out-of-process) plugin adapter.
//!
//! `ExternalPlugin` implements the `Plugin` trait by invoking an external
//! executable and exchanging JSON on stdin/stdout. This adapter is used when
//! a manifest indicates an external plugin entrypoint.
//!
use crate::vm::plugin::Plugin;
use crate::vm::value::Value;
use async_trait::async_trait;
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub struct ExternalPlugin {
    name: String,
    exe: PathBuf,
}

impl ExternalPlugin {
    pub fn new(name: String, exe: PathBuf) -> Self {
        Self { name, exe }
    }

    fn value_to_json(v: &Value) -> serde_json::Value {
        match v {
            Value::Int(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
            Value::Float(f) => serde_json::Value::Number(serde_json::Number::from_f64(*f).unwrap_or(serde_json::Number::from(0))),
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Str(s) => serde_json::Value::String(s.clone()),
            Value::Symbol(s) => serde_json::Value::String(s.clone()),
            Value::Array(a) => serde_json::Value::Array(a.iter().map(Self::value_to_json).collect()),
            Value::Object(m) => {
                let mut map = serde_json::Map::new();
                for (k, v) in m.iter() {
                    map.insert(k.clone(), Self::value_to_json(v));
                }
                serde_json::Value::Object(map)
            }
            Value::Null => serde_json::Value::Null,
        }
    }

    fn json_to_value(j: &serde_json::Value) -> Value {
        match j {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Number(n) => {
                if n.is_i64() {
                    Value::Int(n.as_i64().unwrap_or(0))
                } else {
                    Value::Float(n.as_f64().unwrap_or(0.0))
                }
            }
            serde_json::Value::String(s) => Value::Str(s.clone()),
            serde_json::Value::Array(a) => Value::Array(a.iter().map(Self::json_to_value).collect()),
            serde_json::Value::Object(o) => {
                let mut map = std::collections::HashMap::new();
                for (k, v) in o.iter() {
                    map.insert(k.clone(), Self::json_to_value(v));
                }
                Value::Object(map)
            }
        }
    }
}

#[async_trait]
impl Plugin for ExternalPlugin {
    fn name(&self) -> &str { &self.name }

    async fn call(&self, func: &str, args: Vec<Value>) -> Result<Value, String> {
        // Prepare request JSON: { "func": "<func>", "args": [ ... ] }
        let req = serde_json::json!({
            "func": func,
            "args": args.iter().map(Self::value_to_json).collect::<Vec<_>>()
        });

        let mut cmd = Command::new(&self.exe);
        cmd.arg("call");
        cmd.arg(func);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        // Capture stderr so we can include plugin diagnostic output in
        // error messages instead of inheriting it to the parent console.
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| format!("spawn plugin '{}': {}", self.exe.display(), e))?;

        if let Some(mut stdin) = child.stdin.take() {
            let body = serde_json::to_vec(&req).map_err(|e| format!("serialize req: {}", e))?;
            use std::io::Write;
            stdin.write_all(&body).map_err(|e| format!("write stdin: {}", e))?;
            // close stdin so child sees EOF
            drop(stdin);
        }

        let output = child.wait_with_output().map_err(|e| format!("wait plugin: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            // If the plugin failed, prefer any structured error in stdout
            // but always include stderr for diagnostics.
            let maybe_json = serde_json::from_str::<serde_json::Value>(&stdout).ok();
            if let Some(j) = maybe_json {
                if let Some(err) = j.get("error").and_then(|v| v.as_str()) {
                    return Err(format!("{} (plugin stderr: {})", err, stderr));
                }
            }
            return Err(format!("plugin '{}' exit code: {}\nstdout: {}\nstderr: {}", self.name, output.status, stdout, stderr));
        }

        // Parse stdout as JSON and interpret plugin-level errors encoded
        // in the returned object (common pattern: { ok: bool, error: "..." }).
        let json: serde_json::Value = serde_json::from_str(&stdout)
            .map_err(|e| format!("parse plugin output: {} \nstdout: {}\nstderr: {}", e, stdout, stderr))?;

        if let serde_json::Value::Object(map) = &json {
            if let Some(okv) = map.get("ok") {
                if okv.as_bool() == Some(false) {
                    if let Some(errv) = map.get("error") {
                        if let Some(s) = errv.as_str() {
                            return Err(format!("{}", s));
                        } else {
                            return Err(format!("plugin '{}' reported an error: {:?}", self.name, errv));
                        }
                    } else {
                        return Err(format!("plugin '{}' reported failure: {:?}", self.name, map));
                    }
                }
                // ok == true: prefer returning `path` or `result` fields when present
                if okv.as_bool() == Some(true) {
                    if let Some(p) = map.get("path") {
                        return Ok(Self::json_to_value(p));
                    }
                    if let Some(r) = map.get("result") {
                        return Ok(Self::json_to_value(r));
                    }
                }
            }
        }

        Ok(Self::json_to_value(&json))
    }

    fn as_any(&self) -> &dyn std::any::Any { self }
}
