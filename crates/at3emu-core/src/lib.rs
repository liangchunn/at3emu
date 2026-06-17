//! ELF emulator for running x86 Linux binaries via Unicorn.
//!
//! ```no_run
//! use std::path::PathBuf;
//! use at3emu_core::Emulator;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut emu = Emulator::new()?;
//! emu.map_memory()?;
//!
//! let lib_path = PathBuf::from("linux/libatrac.so.1.2.0");
//! emu.load_lib(&lib_path)?;
//!
//! let exe_path = PathBuf::from("linux/at3tool");
//! let entry = emu.load_exe(&exe_path)?;
//!
//! emu.setup_hostcall_hook()?;
//!
//! let args = vec![
//!     "at3tool".into(),
//!     "-e".into(),
//!     "-br".into(),
//!     "132".into(),
//!     "input.wav".into(),
//!     "output.at3".into(),
//! ];
//! let exit_code = emu.run(entry, &args)?;
//!
//! let state_ref = emu.state.borrow();
//! let stdout = String::from_utf8_lossy(&state_ref.stdout_buf);
//! if !stdout.is_empty() {
//!     print!("{}", stdout);
//! }
//! drop(state_ref);
//!
//! std::process::exit(exit_code);
//! # }
//! ```

pub(crate) mod emu;
pub(crate) mod hostcalls;

pub use emu::Emulator;
