use std::env;
use std::path::PathBuf;

mod emu;
mod hostcalls;

use emu::Emulator;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    let mut at3tool_path = String::new();
    let mut libatrac_path = String::new();
    let mut pass_args: Vec<String> = vec![args[0].clone()];

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--at3tool" => {
                i += 1;
                if i < args.len() {
                    at3tool_path = args[i].clone();
                }
            }
            "--libatrac" => {
                i += 1;
                if i < args.len() {
                    libatrac_path = args[i].clone();
                }
            }
            "--list-codecs" => {
                eprintln!("Supported codecs:");
                eprintln!("  ATRAC3:");
                eprintln!(
                    "    52kbps mono  66kbps mono  66kbps stereo  105kbps stereo  132kbps stereo"
                );
                eprintln!("  ATRAC3plus:");
                eprintln!("    32kbps mono  48kbps mono  64kbps mono  96kbps mono  128kbps mono");
                eprintln!("    48kbps stereo  64kbps stereo  96kbps stereo  128kbps stereo");
                eprintln!("    160kbps stereo  192kbps stereo  256kbps stereo  320kbps stereo  352kbps stereo");
                return Ok(());
            }
            _ => {
                pass_args.push(args[i].clone());
            }
        }
        i += 1;
    }

    if at3tool_path.is_empty() {
        at3tool_path = find_binary("linux/at3tool")?;
    }
    if libatrac_path.is_empty() {
        libatrac_path = find_binary("linux/libatrac.so.1.2.0")?;
    }

    eprintln!("[at3emu] loading libatrac: {}", libatrac_path);
    eprintln!("[at3emu] loading at3tool: {}", at3tool_path);

    let mut emu = Emulator::new()?;
    emu.map_memory()?;

    let lib_path = PathBuf::from(&libatrac_path);
    emu.load_lib(&lib_path)?;

    let exe_path = PathBuf::from(&at3tool_path);
    let entry = emu.load_exe(&exe_path)?;

    emu.setup_hostcall_hook()?;

    eprintln!("[at3emu] running: {:?}", &pass_args[1..]);
    let exit_code = emu.run(entry, &pass_args)?;

    let state_ref = emu.state.borrow();
    let stdout = String::from_utf8_lossy(&state_ref.stdout_buf);
    if !stdout.is_empty() {
        print!("{}", stdout);
    }
    drop(state_ref);

    std::process::exit(exit_code);
}

fn find_binary(relative: &str) -> Result<String, Box<dyn std::error::Error>> {
    let relative_path = std::path::Path::new(relative);

    if let Ok(exe) = env::current_exe()
        && let Some(parent) = exe.parent() {
            let candidate = parent.join(relative_path);
            if candidate.exists() {
                return Ok(candidate.to_string_lossy().to_string());
            }
            if let Some(grandparent) = parent.parent() {
                let candidate = grandparent.join(relative_path);
                if candidate.exists() {
                    return Ok(candidate.to_string_lossy().to_string());
                }
            }
        }

    if let Ok(cwd) = env::current_dir() {
        let candidate = cwd.join(relative_path);
        if candidate.exists() {
            return Ok(candidate.to_string_lossy().to_string());
        }
    }

    Err(format!(
        "cannot find {}. Use --at3tool or --libatrac to specify paths.",
        relative_path.display()
    )
    .into())
}
