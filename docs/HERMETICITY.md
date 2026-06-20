# Hermeticity & Reproducibility

A build that "works on my machine" is one that quietly depends on whatever happens to be
installed, exported, or left lying around the working tree. Mainstage offers four features
that move a build toward being **declared**, **isolated**, and **verifiable**:

- **`requires { … }`** — declare the external tools a stage needs.
- **`hermetic: true`** — run a stage's commands with a cleared environment.
- **`mainstage --check-reproducible`** — verify a pipeline produces identical outputs twice.
- **`mainstage --audit-inputs`** — warn about files a stage reads but did not declare.

For the field grammar see [`GRAMMAR.md`](GRAMMAR.md).

---

## Table of Contents

1. [Declared tool requirements](#declared-tool-requirements)
2. [Hermetic stages](#hermetic-stages)
3. [Reproducibility checks](#reproducibility-checks)
4. [Input-completeness audit](#input-completeness-audit)

---

## Declared tool requirements

A `requires { … }` block on a stage lists the external programs it needs, each with an
optional version constraint. The check runs **before the stage's steps** (and only when the
stage actually runs — a cached stage is not checked):

```mainstage
stage compile {
    requires {
        "cargo" >= "1.70"   // must be on PATH and at least version 1.70
        "git"               // must be on PATH (any version)
    }
    steps {
        $ cargo build --release
    }
}
```

Each entry is a program name (a **string**, so paths and names containing `-`/`+` like
`g++` work) and an optional constraint: an operator (`>=`, `>`, `<=`, `<`, `==`) and a
dotted version string. Presence is checked by resolving the name on `PATH`; a version
constraint is checked against the number parsed from `<tool> --version` (the first
dotted-numeric run in its output, so banners like `cargo 1.70.0 (…)` work). Versions compare
component-wise with missing components treated as zero, so `1.70` and `1.70.0` are equal.

A missing tool, an unsatisfiable version, or a tool whose version cannot be determined fails
the stage with a precise diagnostic — and behaves like any other failed step: a `try` block
swallows it, the stage's `on_failure` fires, and downstream stages are cancelled (unless
`allow_failure: true`). The version string in a constraint must be a dotted number; a typo
like `"latest"` is rejected at analysis time.

## Hermetic stages

Setting `hermetic: true` runs the stage's spawned commands (`$` exec and `expect`) from a
**cleared** environment instead of inheriting the parent process's, so the stage cannot
silently depend on an ambient variable that happens to be set on one machine but not
another:

```mainstage
stage package {
    hermetic: true
    steps {
        with_env { SOURCE_DATE_EPOCH: "0" } {
            $ tar -czf dist/app.tar.gz dist/
        }
    }
}
```

The cleared environment is repopulated from exactly two sources:

1. **A minimal passthrough** so executables can still be located and run — `PATH` on Unix;
   `PATH`, `SystemRoot`, `SystemDrive`, `ComSpec`, `PATHEXT`, `windir`, `TEMP`, and `TMP` on
   Windows.
2. **Your `with_env { … }` overlay** — the explicit allowlist of variables the stage is
   permitted to see.

Everything else (`HOME`, credentials, locale, CI tokens, …) is invisible to the stage's
commands. `hermetic` composes with `workdir` and `with_env`, and applies uniformly to every
`$` and `expect` in the stage.

## Reproducibility checks

`mainstage --check-reproducible [run <pipeline>]` runs the pipeline **twice from a clean
cache** and compares the content of every declared output between the two runs. Each run
starts from a clean cache so every stage actually executes and rewrites its outputs (rather
than being skipped or restored), and outputs are compared with the same content hashing the
output cache uses.

```console
$ mainstage --check-reproducible
reproducibility: run 1 of 2…
reproducibility: run 2 of 2…

reproducible: 7 declared output(s) identical across both runs
```

When an output differs between runs it is listed by name, and the command exits non-zero —
so it doubles as a CI gate against non-determinism (timestamps, embedded build paths,
unsorted archive entries, and the like):

```console
not reproducible: 1 output(s) differ between runs:
  ✗ dist/app.tar.gz
```

> **Note:** because it rebuilds from scratch twice, `--check-reproducible` clears the cache
> (including the content-addressed output store). Run it deliberately, not on every build.

## Input-completeness audit

Change detection only fingerprints a stage's **declared** `inputs`, so a stage that reads a
file it did not declare can be wrongly skipped after that file changes — the most common
cause of a stale cache. `mainstage --audit-inputs` warns about exactly this:

```console
$ mainstage --audit-inputs
▶ compile
  audit: 1 file(s) read but not declared in inputs:
      ? config.ini
✓ compile
```

The audit records a baseline before each stage runs, then afterward flags project files
whose access time advanced (they were *read*) but whose modification time did not (they were
not *written* by the stage), excluding everything the stage declared in `inputs` or
`outputs`, and skipping dotfiles/dot-directories (`.git`, `.mainstage`, …).

It is **best-effort and depends on the platform tracking file access times** ("where the
platform allows"): on a `noatime` mount the audit finds nothing, so Mainstage probes for
atime support first and prints a warning if the audit would be inert — a clean result is
then known to be meaningful rather than merely silent. Under the common `relatime` policy a
file read more than once within a short window may not re-register, so treat the audit as a
high-signal hint, not a proof of completeness. It is most accurate under `--jobs 1`, where a
read cannot be attributed to a concurrently running stage.

When the audit flags a file, either add it to the stage's `inputs` (so change detection
tracks it) or make the stage `hermetic` / model the file as its own stage's output.
