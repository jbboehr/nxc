// SPDX-License-Identifier: AGPL-3.0-only WITH romic-exception

use nxc::{Diagnostic, MAX_SOURCE_BYTES, emit, nix, syntax};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
enum Stage {
    Read,
    NativeParse,
    NativeLower,
    NxcEmit,
    NxcParse,
    NxcLower,
    NxcEquality,
    NixEmit,
    GeneratedNativeParse,
    GeneratedNativeLower,
    NativeEquality,
}

impl Stage {
    const ALL: [Self; 11] = [
        Self::Read,
        Self::NativeParse,
        Self::NativeLower,
        Self::NxcEmit,
        Self::NxcParse,
        Self::NxcLower,
        Self::NxcEquality,
        Self::NixEmit,
        Self::GeneratedNativeParse,
        Self::GeneratedNativeLower,
        Self::NativeEquality,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::NativeParse => "native parse",
            Self::NativeLower => "native lower",
            Self::NxcEmit => "nxc emit",
            Self::NxcParse => "nxc parse",
            Self::NxcLower => "nxc lower",
            Self::NxcEquality => "nxc equality",
            Self::NixEmit => "nix emit",
            Self::GeneratedNativeParse => "generated native parse",
            Self::GeneratedNativeLower => "generated native lower",
            Self::NativeEquality => "native equality",
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Count {
    successes: usize,
    failures: usize,
}

#[derive(Default)]
struct Report {
    discovered: usize,
    selected: usize,
    processed: usize,
    counts: [Count; Stage::ALL.len()],
}

struct Failure {
    stage: Stage,
    message: String,
}

impl Report {
    fn step<T>(&mut self, stage: Stage, result: Result<T, String>) -> Result<T, Failure> {
        let count = &mut self.counts[stage as usize];
        match result {
            Ok(value) => {
                count.successes += 1;
                Ok(value)
            }
            Err(message) => {
                count.failures += 1;
                Err(Failure { stage, message })
            }
        }
    }

    fn write(&self, output: &mut impl Write) -> std::io::Result<()> {
        writeln!(output, "files discovered: {}", self.discovered)?;
        writeln!(output, "files selected: {}", self.selected)?;
        writeln!(output, "files processed: {}", self.processed)?;
        for stage in Stage::ALL {
            let count = self.counts[stage as usize];
            writeln!(output, "{} successes: {}", stage.name(), count.successes)?;
            writeln!(output, "{} failures: {}", stage.name(), count.failures)?;
        }
        Ok(())
    }
}

pub fn run(
    root: &Path,
    filter: Option<&str>,
    fail_fast: bool,
    output: &mut impl Write,
    errors: &mut impl Write,
) -> Result<bool, String> {
    let files = discover(root)?;
    let selected: Vec<_> = files
        .iter()
        .filter(|path| {
            filter.is_none_or(|filter| {
                path.strip_prefix(root)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .contains(filter)
            })
        })
        .collect();
    let mut report = Report {
        discovered: files.len(),
        selected: selected.len(),
        ..Report::default()
    };
    let mut success = true;
    for path in selected {
        report.processed += 1;
        if let Err(failure) = roundtrip(path, &mut report) {
            success = false;
            let relative = path.strip_prefix(root).unwrap_or(path);
            writeln!(
                errors,
                "{} [{}]: {}",
                relative.to_string_lossy().escape_debug(),
                failure.stage.name(),
                failure.message
            )
            .map_err(|e| format!("stderr: {e}"))?;
            if fail_fast {
                break;
            }
        }
    }
    report.write(output).map_err(|e| format!("stdout: {e}"))?;
    if report.selected == 0 {
        return Err(format!(
            "{} [selection]: no .nix files selected",
            root.display()
        ));
    }
    Ok(success)
}

fn roundtrip(path: &Path, report: &mut Report) -> Result<(), Failure> {
    let source = report.step(Stage::Read, read_source(path))?;
    let parsed = report.step(Stage::NativeParse, nix::parse(&source).map_err(diagnostics))?;
    let original = report.step(Stage::NativeLower, parsed.lower().map_err(diagnostics))?;
    let nxc_source = report.step(
        Stage::NxcEmit,
        emit::nxc(&original).map_err(|e| diagnostic(&e)),
    )?;
    let nxc_parsed = syntax::parse(&nxc_source);
    report.step(
        Stage::NxcParse,
        if nxc_parsed.diagnostics().is_empty() {
            Ok(())
        } else {
            Err(diagnostics(nxc_parsed.diagnostics().to_vec()))
        },
    )?;
    let from_nxc = report.step(Stage::NxcLower, nxc_parsed.lower().map_err(diagnostics))?;
    report.step(
        Stage::NxcEquality,
        equal(original.canonical() == from_nxc.canonical()),
    )?;
    let native_source = report.step(
        Stage::NixEmit,
        nix::emit(&from_nxc).map_err(|e| diagnostic(&e)),
    )?;
    let native_parsed = report.step(
        Stage::GeneratedNativeParse,
        nix::parse(&native_source).map_err(diagnostics),
    )?;
    let from_native = report.step(
        Stage::GeneratedNativeLower,
        native_parsed.lower().map_err(diagnostics),
    )?;
    report.step(
        Stage::NativeEquality,
        equal(original.canonical() == from_native.canonical()),
    )
}

fn equal(matches: bool) -> Result<(), String> {
    if matches {
        Ok(())
    } else {
        Err("canonical semantic IR differs from the original native input".into())
    }
}

fn diagnostics(errors: Vec<Diagnostic>) -> String {
    match errors.first() {
        Some(first) if errors.len() > 1 => format!(
            "{} ({} additional diagnostics)",
            diagnostic(first),
            errors.len() - 1
        ),
        Some(first) => diagnostic(first),
        None => "conversion failed without diagnostics".into(),
    }
}

fn diagnostic(error: &Diagnostic) -> String {
    format!(
        "bytes {}..{}: {}",
        error.span.start, error.span.end, error.message
    )
}

fn read_source(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take((MAX_SOURCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err("source exceeds the 1 MiB limit".into());
    }
    String::from_utf8(bytes).map_err(|e| format!("input is not UTF-8: {e}"))
}

fn discover(root: &Path) -> Result<Vec<PathBuf>, String> {
    let error =
        |path: &Path, error: std::io::Error| format!("{} [discovery]: {error}", path.display());
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|e| error(&directory, e))? {
            let entry = entry.map_err(|e| error(&directory, e))?;
            let path = entry.path();
            let kind = entry.file_type().map_err(|e| error(&path, e))?;
            if kind.is_dir() && entry.file_name() != ".git" {
                pending.push(path);
            } else if kind.is_file() && path.extension().is_some_and(|extension| extension == "nix")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}
