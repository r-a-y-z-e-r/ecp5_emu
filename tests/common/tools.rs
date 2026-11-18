//! Shared FPGA tool discovery for integration tests.
use std::{env, path::Path, process::Command};

/// Resolve FPGA tools using their override first, then PATH. Other programs
/// retain normal `Command` lookup so fixture runners can also invoke the CLI.
pub fn command(program: &str) -> Command {
    let variable = match program {
        "yosys" => "YOSYS",
        "nextpnr-ecp5" => "NEXTPNR_ECP5",
        "ecppack" => "ECPPACK",
        _ => return Command::new(program),
    };
    let executable = env::var_os(variable).unwrap_or_else(|| program.into());
    let path = Path::new(&executable);
    let resolved = if path.components().count() > 1 || path.is_absolute() {
        executable_file(path).then(|| path.to_path_buf())
    } else {
        env::var_os("PATH").and_then(|paths| {
            env::split_paths(&paths)
                .map(|dir| dir.join(path))
                .find(|candidate| executable_file(candidate))
        })
    };
    let resolved = resolved.unwrap_or_else(|| {
        panic!(
            "required tool {program} not found or not executable; put it on PATH or set {variable} to its executable"
        )
    });
    // Resolve relative PATH entries/overrides before callers change the child directory.
    Command::new(env::current_dir().unwrap().join(resolved))
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    true
}
