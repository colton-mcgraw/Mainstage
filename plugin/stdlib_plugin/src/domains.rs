use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use rand::Rng;
use serde::Serialize;

// --- IO ---
pub fn ask(question: String) -> String {
    print!("{} ", question);
    let _ = io::stdout().flush();
    let mut buf = String::new();
    let _ = io::stdin().read_line(&mut buf);
    buf.trim_end().to_string()
}

pub fn say(message: String) {
    println!("{}", message);
}

// --- Env ---
pub fn get_env(key: String) -> Option<String> {
    std::env::var(&key).ok()
}

pub fn set_env(key: String, value: String) {
    unsafe { std::env::set_var(key, value); }
}

#[derive(Serialize)]
pub struct EnvKV { pub key: String, pub value: String }

pub fn list_env() -> Vec<EnvKV> {
    std::env::vars().map(|(k, v)| EnvKV { key: k, value: v }).collect()
}

// --- FS ---
pub fn read(path: String) -> Option<String> { fs::read_to_string(path).ok() }

pub fn write(path: String, content: String) -> bool { fs::write(path, content).is_ok() }

pub fn delete(path: String) -> bool { fs::remove_file(path).is_ok() }

pub fn exists(path: String) -> bool { Path::new(&path).exists() }

pub fn list_dir(path: String) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(path) {
        for e in rd.flatten() { out.push(e.path().to_string_lossy().to_string()); }
    }
    out
}

pub fn make_dir(path: String) -> bool { fs::create_dir_all(path).is_ok() }

pub fn remove_dir(path: String) -> bool { fs::remove_dir_all(path).is_ok() }

pub fn glob(pattern: String, recursive: Option<bool>) -> Vec<String> {
    let recursive = recursive.unwrap_or(false);
    let pattern_path = PathBuf::from(&pattern);
    let (base, pat) = if let Some(parent) = pattern_path.parent() {
        ( parent.to_path_buf(), pattern_path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or("*".into()) )
    } else { (PathBuf::from("."), pattern.clone()) };
    let mut out = Vec::new();
    let walker: Box<dyn Iterator<Item = fs::DirEntry>> = if recursive {
        Box::new(walk_dir_recursive(base))
    } else {
        Box::new( fs::read_dir(base).unwrap_or_else(|_| fs::read_dir(".").unwrap()).flatten() )
    };
    for e in walker {
        let p = e.path();
        if let Some(name) = p.file_name().map(|s| s.to_string_lossy().to_string()) 
        && glob_match(&name, &pat) { out.push(p.to_string_lossy().to_string()); }
    }
    out
}

fn walk_dir_recursive(base: PathBuf) -> impl Iterator<Item = fs::DirEntry> {
    let mut stack = vec![base];
    let mut items: Vec<fs::DirEntry> = Vec::new();
    while let Some(dir) = stack.pop() {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() { let p = e.path(); if p.is_dir() { stack.push(p); } items.push(e); }
        }
    }
    items.into_iter()
}

fn glob_match(name: &str, pat: &str) -> bool {
    let n: Vec<char> = name.chars().collect();
    let p: Vec<char> = pat.chars().collect();
    let mut i = 0usize; let mut j = 0usize; let mut si = 0usize; let mut sj = usize::MAX;
    while i < n.len() {
        if j < p.len() && (p[j] == '?' || n[i] == p[j]) { i += 1; j += 1; }
        else if j < p.len() && p[j] == '*' { sj = j; si = i; j += 1; }
        else if sj != usize::MAX { j = sj + 1; si += 1; i = si; }
        else { return false; }
    }
    while j < p.len() && p[j] == '*' { j += 1; }
    j == p.len()
}

pub fn copy(from: String, to: String) -> bool {
    let src = Path::new(&from);
    if src.is_dir() { copy_dir_recursive(src, Path::new(&to)).is_ok() } else { fs::copy(src, to).is_ok() }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?; let p = entry.path(); let target = dst.join(entry.file_name());
        if p.is_dir() { copy_dir_recursive(&p, &target)?; } else { fs::copy(&p, &target)?; }
    }
    Ok(())
}

pub fn move_path(from: String, to: String) -> bool { fs::rename(from, to).is_ok() }

#[derive(Serialize)]
pub struct Stat { pub size: i64, pub modified: String, pub is_dir: bool }

pub fn stat(path: String) -> Stat {
    match fs::metadata(&path) {
        Ok(m) => {
            let size = m.len() as i64; let is_dir = m.is_dir();
            let modified = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis().to_string()).unwrap_or_default();
            Stat { size, modified, is_dir }
        }
        Err(_) => Stat { size: -1, modified: String::new(), is_dir: false }
    }
}

// --- Time/Random ---
pub fn sleep(milliseconds: i64) { if milliseconds > 0 { thread::sleep(Duration::from_millis(milliseconds as u64)); } }

pub fn current_time_millis() -> i64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0) }

pub fn random_int(min: i64, max: i64) -> i64 { if max <= min { return min; } let mut rng = rand::thread_rng(); rng.gen_range(min..max) }

pub fn random_float() -> f64 { let mut rng = rand::thread_rng(); rng.r#gen::<f64>() }

// --- Process ---
pub fn which(name: String) -> Option<String> { which::which(name).ok().map(|p| p.to_string_lossy().to_string()) }

#[derive(Serialize)]
pub struct ExecResult { pub code: i32, pub stdout: String, pub stderr: String }

pub fn exec(command: String, args: Option<Vec<String>>, cwd: Option<String>, env: Option<serde_json::Value>, _timeout: Option<i64>) -> ExecResult {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(command);
    if let Some(a) = args { cmd.args(a); }
    if let Some(c) = cwd { cmd.current_dir(c); }
    if let Some(e) = env && let Some(obj) = e.as_object() { 
        for (k, v) in obj.iter() { 
            if let Some(s) = v.as_str() { cmd.env(k, s); } 
        }
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = match cmd.output() { Ok(o) => o, Err(e) => { return ExecResult { code: -1, stdout: String::new(), stderr: e.to_string() } } };
    ExecResult { code: out.status.code().unwrap_or(-1), stdout: String::from_utf8_lossy(&out.stdout).to_string(), stderr: String::from_utf8_lossy(&out.stderr).to_string() }
}

// --- Exit ---
pub fn exit(code: Option<i64>) { std::process::exit(code.unwrap_or(0) as i32) }
