use std::fs;

use log::{error, warn};
use unicorn_engine::RegisterX86;
use unicorn_engine::Unicorn;

use crate::emu::{EmuFile, EmuState, HostcallAction, STACK_BASE, STACK_SIZE};

/// Dispatches a glibc call to the appropriate Rust shim.
///
/// Returns a `HostcallAction` that tells the code hook how to proceed:
/// - `Return(val)` — write `val` to `EAX` and return to caller
/// - `Stop` — stop emulation (for `exit`, `__assert_fail`)
/// - `Skip` — the handler already redirected execution (for `__libc_start_main`)
pub fn dispatch(
    state: &mut EmuState,
    name: &str,
    args: &[u32],
    emu: &mut Unicorn<'_, ()>,
) -> HostcallAction {
    match name {
        // -- runtime --------------------------------------------------
        "__libc_start_main" => h_libc_start_main(args, emu),
        "__gmon_start__" => HostcallAction::Return(Some(0)),
        "_Jv_RegisterClasses" => HostcallAction::Return(Some(0)),

        // -- memory ---------------------------------------------------
        "malloc" => HostcallAction::Return(Some(h_malloc(state, args))),
        "free" => HostcallAction::Return(Some(0)),
        "calloc" => HostcallAction::Return(Some(h_calloc(state, args))),
        "realloc" => HostcallAction::Return(Some(h_realloc(state, args, emu))),

        // -- string ---------------------------------------------------
        "memcpy" => HostcallAction::Return(Some(h_memcpy(args, emu))),
        "memmove" => HostcallAction::Return(Some(h_memcpy(args, emu))),
        "memset" => HostcallAction::Return(Some(h_memset(args, emu))),
        "memcmp" => HostcallAction::Return(Some(h_memcmp(args, emu))),
        "strcmp" => HostcallAction::Return(Some(h_strcmp(args, emu))),
        "atoi" => HostcallAction::Return(Some(h_atoi(args, emu))),

        // -- file I/O -------------------------------------------------
        "fopen" => HostcallAction::Return(Some(h_fopen(state, args, emu))),
        "fclose" => HostcallAction::Return(Some(h_fclose(state, args))),
        "fread" => HostcallAction::Return(Some(h_fread(state, args, emu))),
        "fwrite" => HostcallAction::Return(Some(h_fwrite(state, args, emu))),
        "fseek" => HostcallAction::Return(Some(h_fseek(state, args))),
        "ftell" => HostcallAction::Return(Some(h_ftell(state, args))),
        "feof" => HostcallAction::Return(Some(h_feof(state, args))),
        "fgetc" => HostcallAction::Return(Some(h_fgetc(state, args))),

        // -- stdio ----------------------------------------------------
        "printf" => HostcallAction::Return(Some(h_printf(args, emu, state))),
        "fprintf" => HostcallAction::Return(Some(h_fprintf(state, args, emu))),
        "puts" => HostcallAction::Return(Some(h_puts(args, emu, state))),
        "putchar" => HostcallAction::Return(Some(h_putchar(args, state))),

        // -- process control ------------------------------------------
        "exit" => {
            let code = args.first().copied().unwrap_or(0) as i32;
            state.exit_code = Some(code);
            HostcallAction::Stop
        }

        // -- math -------------------------------------------------
        "abs" => HostcallAction::Return(Some(h_iabs(args))),
        "labs" => HostcallAction::Return(Some(h_iabs(args))),
        "fabs" => HostcallAction::Return(Some(h_fabs_handler(emu))),
        "floor" => HostcallAction::Return(Some(h_f64_unary(|x| x.floor(), emu))),
        "floorf" => HostcallAction::Return(Some(h_f32_unary(|x| x.floor(), emu))),
        "sqrt" => HostcallAction::Return(Some(h_f64_unary(|x| x.sqrt(), emu))),
        "sqrtf" => HostcallAction::Return(Some(h_f32_unary(|x| x.sqrt(), emu))),
        "sin" => HostcallAction::Return(Some(h_f64_unary(|x| x.sin(), emu))),
        "cos" => HostcallAction::Return(Some(h_f64_unary(|x| x.cos(), emu))),
        "asin" => HostcallAction::Return(Some(h_f64_unary(|x| x.asin(), emu))),
        "atan" => HostcallAction::Return(Some(h_f64_unary(|x| x.atan(), emu))),
        "atan2" => HostcallAction::Return(Some(h_atan2_handler(emu))),
        "log" => HostcallAction::Return(Some(h_f64_unary(|x| x.ln(), emu))),
        "log10" => HostcallAction::Return(Some(h_f64_unary(|x| x.log10(), emu))),
        "pow" => HostcallAction::Return(Some(h_pow_handler(emu))),

        "__assert_fail" => {
            error!("__assert_fail called!");
            HostcallAction::Stop
        }

        _ => {
            warn!("unknown: {}", name);
            HostcallAction::Stop
        }
    }
}

// -- runtime helpers -------------------------------------------------------

/// Handles `__libc_start_main` by redirecting execution to `main(argc, argv, envp)`.
/// Sets up a fresh stack frame and tells the hook to skip (already jumped).
fn h_libc_start_main(args: &[u32], emu: &mut Unicorn<'_, ()>) -> HostcallAction {
    let main_ptr = args.first().copied().unwrap_or(0) as u64;
    let argc = args.get(1).copied().unwrap_or(0);
    let argv_ptr = args.get(2).copied().unwrap_or(0) as u64;

    if main_ptr == 0 {
        error!("__libc_start_main: main is null");
        return HostcallAction::Stop;
    }

    let stack_top = STACK_BASE + STACK_SIZE - 0x1000;
    let mut sp = stack_top;

    let envp = argv_ptr + (argc + 1) as u64 * 4;
    let sentinel: u32 = 0xDEADBEEF;
    sp -= 4;
    emu.mem_write(sp, &(envp as u32).to_ne_bytes()).ok();
    sp -= 4;
    emu.mem_write(sp, &(argv_ptr as u32).to_ne_bytes()).ok();
    sp -= 4;
    emu.mem_write(sp, &argc.to_ne_bytes()).ok();
    sp -= 4;
    emu.mem_write(sp, &sentinel.to_ne_bytes()).ok();

    emu.reg_write(RegisterX86::ESP, sp).ok();
    emu.reg_write(RegisterX86::EBP, sp).ok();
    emu.reg_write(RegisterX86::EIP, main_ptr).ok();

    HostcallAction::Skip
}

// -- memory helpers ---------------------------------------------------------

fn h_malloc(state: &mut EmuState, args: &[u32]) -> u32 {
    let size = args.first().copied().unwrap_or(0) as usize;
    if size == 0 {
        return 0;
    }
    let aligned = (size + 15) & !15;
    let addr = state.heap_brk;
    state.heap_brk = addr + aligned as u64;
    addr as u32
}

fn h_calloc(state: &mut EmuState, args: &[u32]) -> u32 {
    let nmemb = args.first().copied().unwrap_or(0) as usize;
    let size = args.get(1).copied().unwrap_or(0) as usize;
    let total = nmemb * size;
    if total == 0 {
        return 0;
    }
    let addr = state.heap_brk as u32;
    state.heap_brk += ((total + 15) & !15) as u64;
    addr
}

fn h_realloc(state: &mut EmuState, args: &[u32], emu: &mut Unicorn<'_, ()>) -> u32 {
    let ptr = args.first().copied().unwrap_or(0);
    let size = args.get(1).copied().unwrap_or(0) as usize;
    if size == 0 {
        return 0;
    }
    if ptr == 0 {
        return h_malloc(state, &[size as u32]);
    }
    let new_ptr = h_malloc(state, &[size as u32]);
    if new_ptr != 0 {
        let mut buf = vec![0u8; size.min(4096)];
        if emu.mem_read(ptr as u64, &mut buf).is_ok() {
            emu.mem_write(new_ptr as u64, &buf).ok();
        }
    }
    new_ptr
}

// -- string / memory utilities ----------------------------------------------

fn h_memcpy(args: &[u32], emu: &mut Unicorn<'_, ()>) -> u32 {
    let dst = args.first().copied().unwrap_or(0) as u64;
    let src = args.get(1).copied().unwrap_or(0) as u64;
    let n = args.get(2).copied().unwrap_or(0) as usize;
    if n > 0 && dst != 0 && src != 0 {
        let max_copy = n.min(16 * 1024 * 1024);
        let mut buf = vec![0u8; max_copy];
        if emu.mem_read(src, &mut buf).is_ok() {
            emu.mem_write(dst, &buf).ok();
        }
    }
    dst as u32
}

fn h_memset(args: &[u32], emu: &mut Unicorn<'_, ()>) -> u32 {
    let dst = args.first().copied().unwrap_or(0) as u64;
    let val = args.get(1).copied().unwrap_or(0) as u8;
    let n = args.get(2).copied().unwrap_or(0) as usize;
    if n > 0 && dst != 0 {
        let max_set = n.min(16 * 1024 * 1024);
        let buf = vec![val; max_set];
        emu.mem_write(dst, &buf).ok();
    }
    dst as u32
}

fn h_memcmp(args: &[u32], emu: &mut Unicorn<'_, ()>) -> u32 {
    let a = args.first().copied().unwrap_or(0) as u64;
    let b = args.get(1).copied().unwrap_or(0) as u64;
    let n = args.get(2).copied().unwrap_or(0) as usize;
    if n == 0 {
        return 0;
    }
    let max_cmp = n.min(4096);
    let mut buf_a = vec![0u8; max_cmp];
    let mut buf_b = vec![0u8; max_cmp];
    if emu.mem_read(a, &mut buf_a).is_err() || emu.mem_read(b, &mut buf_b).is_err() {
        return 0;
    }
    for i in 0..max_cmp {
        let va = buf_a[i];
        let vb = buf_b[i];
        if va < vb {
            return u32::MAX;
        }
        if va > vb {
            return 1;
        }
    }
    0
}

fn h_strcmp(args: &[u32], emu: &mut Unicorn<'_, ()>) -> u32 {
    let a = args.first().copied().unwrap_or(0) as u64;
    let b = args.get(1).copied().unwrap_or(0) as u64;
    let sa = read_cstr(emu, a);
    let sb = read_cstr(emu, b);
    match sa.cmp(&sb) {
        std::cmp::Ordering::Less => u32::MAX,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn h_atoi(args: &[u32], emu: &mut Unicorn<'_, ()>) -> u32 {
    let ptr = args.first().copied().unwrap_or(0) as u64;
    let s = read_cstr(emu, ptr);
    s.parse::<i32>().unwrap_or(0) as u32
}

/// Reads a null-terminated string from emulated memory (max 256 bytes).
fn read_cstr(emu: &Unicorn<'_, ()>, addr: u64) -> String {
    let mut buf = vec![0u8; 256];
    if emu.mem_read(addr, &mut buf).is_err() {
        return String::new();
    }
    buf.iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect()
}

// -- file I/O helpers -------------------------------------------------------

fn h_fopen(state: &mut EmuState, args: &[u32], emu: &mut Unicorn<'_, ()>) -> u32 {
    let path_ptr = args.first().copied().unwrap_or(0) as u64;
    let mode_ptr = args.get(1).copied().unwrap_or(0) as u64;
    let path = read_cstr(emu, path_ptr);
    let mode = read_cstr(emu, mode_ptr);

    let is_write = mode.contains('w') || mode.contains('a');
    let is_read = mode.contains('r') || mode.contains('+');

    if is_write {
        state
            .files
            .push(EmuFile::Write(path.clone(), Vec::new(), 0));
        state.files.len() as u32
    } else if is_read {
        match fs::read(&path) {
            Ok(data) => {
                state.files.push(EmuFile::Read(data, 0));
                state.files.len() as u32
            }
            Err(e) => {
                warn!("fopen({}): {}", path, e);
                0
            }
        }
    } else {
        0
    }
}

fn h_fclose(state: &mut EmuState, args: &[u32]) -> u32 {
    let handle = args.first().copied().unwrap_or(0) as usize;
    if handle == 0 || handle > state.files.len() {
        return u32::MAX;
    }
    let idx = handle - 1;

    if let EmuFile::Write(path, data, _pos) = &state.files[idx]
        && !path.is_empty() {
            let _ = fs::write(path, data);
        }

    0
}

fn h_fread(state: &mut EmuState, args: &[u32], emu: &mut Unicorn<'_, ()>) -> u32 {
    let buf_ptr = args.first().copied().unwrap_or(0) as u64;
    let size = args.get(1).copied().unwrap_or(0) as usize;
    let count = args.get(2).copied().unwrap_or(0) as usize;
    let handle = args.get(3).copied().unwrap_or(0) as usize;

    let total = size * count;
    if total == 0 || handle == 0 || handle > state.files.len() {
        return 0;
    }
    let idx = handle - 1;

    match &mut state.files[idx] {
        EmuFile::Read(data, pos) => {
            let remaining = data.len().saturating_sub(*pos);
            let to_read = total.min(remaining);
            if to_read > 0 {
                emu.mem_write(buf_ptr, &data[*pos..*pos + to_read]).ok();
                *pos += to_read;
            }
            (to_read / size.max(1)) as u32
        }
        EmuFile::Write(..) => 0,
    }
}

fn h_fwrite(state: &mut EmuState, args: &[u32], emu: &mut Unicorn<'_, ()>) -> u32 {
    let buf_ptr = args.first().copied().unwrap_or(0) as u64;
    let size = args.get(1).copied().unwrap_or(0) as usize;
    let count = args.get(2).copied().unwrap_or(0) as usize;
    let handle = args.get(3).copied().unwrap_or(0) as usize;

    let total = size * count;
    if total == 0 || handle == 0 || handle > state.files.len() {
        return 0;
    }
    let idx = handle - 1;

    match &mut state.files[idx] {
        EmuFile::Write(_, data, pos) => {
            let mut buf = vec![0u8; total];
            if emu.mem_read(buf_ptr, &mut buf).is_ok() {
                let end = *pos + total;
                if end > data.len() {
                    data.resize(end, 0);
                }
                data[*pos..*pos + total].copy_from_slice(&buf);
                *pos = end;
            }
            count as u32
        }
        _ => 0,
    }
}

fn h_fseek(state: &mut EmuState, args: &[u32]) -> u32 {
    let handle = args.first().copied().unwrap_or(0) as usize;
    let offset = args.get(1).copied().unwrap_or(0) as i64;
    let whence = args.get(2).copied().unwrap_or(0) as i32;

    if handle == 0 || handle > state.files.len() {
        return u32::MAX;
    }
    let idx = handle - 1;

    match &mut state.files[idx] {
        EmuFile::Read(data, pos) => {
            let new_pos = match whence {
                0 => offset,
                1 => *pos as i64 + offset,
                2 => data.len() as i64 + offset,
                _ => return u32::MAX,
            };
            if new_pos < 0 {
                return u32::MAX;
            }
            *pos = (new_pos as usize).min(data.len());
            0
        }
        EmuFile::Write(_, data, pos) => {
            let new_pos = match whence {
                0 => offset,
                1 => *pos as i64 + offset,
                2 => data.len() as i64 + offset,
                _ => return u32::MAX,
            };
            if new_pos < 0 {
                return u32::MAX;
            }
            *pos = new_pos as usize;
            0
        }
    }
}

fn h_ftell(state: &mut EmuState, args: &[u32]) -> u32 {
    let handle = args.first().copied().unwrap_or(0) as usize;
    if handle == 0 || handle > state.files.len() {
        return u32::MAX;
    }
    let idx = handle - 1;

    match &state.files[idx] {
        EmuFile::Read(_, pos) => *pos as u32,
        EmuFile::Write(_, _, pos) => *pos as u32,
    }
}

fn h_feof(state: &mut EmuState, args: &[u32]) -> u32 {
    let handle = args.first().copied().unwrap_or(0) as usize;
    if handle == 0 || handle > state.files.len() {
        return 1;
    }
    let idx = handle - 1;

    match &state.files[idx] {
        EmuFile::Read(data, pos) => (*pos >= data.len()) as u32,
        EmuFile::Write(..) => 0,
    }
}

fn h_fgetc(state: &mut EmuState, args: &[u32]) -> u32 {
    let handle = args.first().copied().unwrap_or(0) as usize;
    if handle == 0 || handle > state.files.len() {
        return u32::MAX;
    }
    let idx = handle - 1;

    match &mut state.files[idx] {
        EmuFile::Read(data, pos) => {
            if *pos < data.len() {
                let b = data[*pos] as u32;
                *pos += 1;
                b
            } else {
                u32::MAX
            }
        }
        EmuFile::Write(..) => u32::MAX,
    }
}

// -- stdio helpers ----------------------------------------------------------

fn h_printf(args: &[u32], emu: &mut Unicorn<'_, ()>, state: &mut EmuState) -> u32 {
    let fmt_ptr = args.first().copied().unwrap_or(0) as u64;
    let fmt = read_cstr(emu, fmt_ptr);
    let result = format_string(&fmt, &args[1..], emu);
    state.stdout_buf.extend_from_slice(result.as_bytes());
    result.len() as u32
}

fn h_fprintf(state: &mut EmuState, args: &[u32], emu: &mut Unicorn<'_, ()>) -> u32 {
    let handle = args.first().copied().unwrap_or(0) as usize;
    let fmt_ptr = args.get(1).copied().unwrap_or(0) as u64;
    let fmt = read_cstr(emu, fmt_ptr);
    let result = format_string(&fmt, &args[2..], emu);

    if handle == 1 || handle == 2 {
        state.stdout_buf.extend_from_slice(result.as_bytes());
    } else if handle > 2 && handle <= state.files.len() + 1 {
        let idx = handle - 1;
        if let EmuFile::Write(_, data, pos) = &mut state.files[idx] {
            let bytes = result.as_bytes();
            let end = *pos + bytes.len();
            if end > data.len() {
                data.resize(end, 0);
            }
            data[*pos..*pos + bytes.len()].copy_from_slice(bytes);
            *pos = end;
        }
    }

    result.len() as u32
}

fn h_puts(args: &[u32], emu: &mut Unicorn<'_, ()>, state: &mut EmuState) -> u32 {
    let ptr = args.first().copied().unwrap_or(0) as u64;
    let s = read_cstr(emu, ptr);
    state.stdout_buf.extend_from_slice(s.as_bytes());
    state.stdout_buf.push(b'\n');
    s.len() as u32 + 1
}

fn h_putchar(args: &[u32], state: &mut EmuState) -> u32 {
    let c = args.first().copied().unwrap_or(0) as u8;
    state.stdout_buf.push(c);
    c as u32
}

/// Minimal `printf`-style formatter supporting `%s`, `%d`, `%u`, `%x`,
/// `%c`, `%p`, `%%`. Width/precision flags are parsed but ignored.
fn format_string(fmt: &str, args: &[u32], emu: &Unicorn<'_, ()>) -> String {
    let mut result = String::new();
    let mut arg_idx = 0;
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            i += 1;
            while i < chars.len() && "-+ #0".contains(chars[i]) {
                i += 1;
            }
            let mut _width = 0usize;
            while i < chars.len() && chars[i].is_ascii_digit() {
                _width = _width * 10 + chars[i].to_digit(10).unwrap() as usize;
                i += 1;
            }
            if i < chars.len() && chars[i] == '.' {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            while i < chars.len() && "hlL".contains(chars[i]) {
                i += 1;
            }
            if i >= chars.len() {
                break;
            }
            let spec = chars[i];
            i += 1;

            match spec {
                's' => {
                    if arg_idx < args.len() {
                        let ptr = args[arg_idx] as u64;
                        result.push_str(&read_cstr(emu, ptr));
                        arg_idx += 1;
                    }
                }
                'd' | 'i' => {
                    if arg_idx < args.len() {
                        result.push_str(&format!("{}", args[arg_idx] as i32));
                        arg_idx += 1;
                    }
                }
                'u' => {
                    if arg_idx < args.len() {
                        result.push_str(&format!("{}", args[arg_idx]));
                        arg_idx += 1;
                    }
                }
                'x' | 'X' => {
                    if arg_idx < args.len() {
                        result.push_str(&format!("{:x}", args[arg_idx]));
                        arg_idx += 1;
                    }
                }
                'c' => {
                    if arg_idx < args.len() {
                        result.push(args[arg_idx] as u8 as char);
                        arg_idx += 1;
                    }
                }
                '%' => {
                    result.push('%');
                }
                'p' => {
                    if arg_idx < args.len() {
                        result.push_str(&format!("0x{:x}", args[arg_idx]));
                        arg_idx += 1;
                    }
                }
                _ => {
                    result.push('%');
                    result.push(spec);
                }
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

// -- math helpers -----------------------------------------------------------

fn h_iabs(args: &[u32]) -> u32 {
    let val = args.first().copied().unwrap_or(0) as i32;
    val.unsigned_abs()
}

/// Reads a `f64` from emulated memory at the given address.
fn read_f64(emu: &Unicorn<'_, ()>, addr: u64) -> f64 {
    let mut buf = [0u8; 8];
    if emu.mem_read(addr, &mut buf).is_ok() {
        f64::from_ne_bytes(buf)
    } else {
        0.0
    }
}

/// Reads a `f32` from emulated memory at the given address.
fn read_f32(emu: &Unicorn<'_, ()>, addr: u64) -> f32 {
    let mut buf = [0u8; 4];
    if emu.mem_read(addr, &mut buf).is_ok() {
        f32::from_ne_bytes(buf)
    } else {
        0.0
    }
}

/// Writes an `f64` to the x87 `ST0` register (80-bit extended precision).
fn write_st0(emu: &mut Unicorn<'_, ()>, val: f64) {
    let bytes = f64_to_x87(val);
    if let Ok(()) = emu.reg_write_long(RegisterX86::ST0, &bytes) {
        return;
    }
    emu.reg_write(RegisterX86::ST0, val.to_bits()).ok();
}

/// Converts an IEEE 754 `f64` to the 80-bit x87 extended-precision format.
///
/// Layout (10 bytes): 64-bit mantissa (explicit leading 1), 15-bit exponent
/// (bias 16383), 1-bit sign.
fn f64_to_x87(val: f64) -> [u8; 10] {
    if val == 0.0 {
        return [0; 10];
    }

    let bits = val.to_bits();
    let sign = (bits >> 63) as u16;
    let exponent = ((bits >> 52) & 0x7ff) as i32;

    if exponent == 0x7ff {
        let mut result = [0u8; 10];
        result[9] = (sign as u8) << 7 | 0x7f;
        result[8] = 0xff;
        result[7] = 0x80;
        return result;
    }

    let mantissa = bits & 0x000f_ffff_ffff_ffff;
    let real_exp = exponent - 1023 + 16383;
    let exp_field = real_exp as u16;
    let mantissa_80 = if exponent == 0 {
        mantissa << 11
    } else {
        (mantissa << 11) | (1u64 << 63)
    };

    let mut result = [0u8; 10];
    result[..8].copy_from_slice(&mantissa_80.to_le_bytes());
    result[8] = (exp_field & 0xff) as u8;
    result[9] = ((sign as u8) << 7) | ((exp_field >> 8) & 0x7f) as u8;

    result
}

fn h_fabs_handler(emu: &mut Unicorn<'_, ()>) -> u32 {
    let esp = emu.reg_read(RegisterX86::ESP).unwrap_or(0);
    let val = read_f64(emu, esp + 4);
    write_st0(emu, val.abs());
    0
}

fn h_f64_unary(f: fn(f64) -> f64, emu: &mut Unicorn<'_, ()>) -> u32 {
    let esp = emu.reg_read(RegisterX86::ESP).unwrap_or(0);
    let val = read_f64(emu, esp + 4);
    write_st0(emu, f(val));
    0
}

fn h_f32_unary(f: fn(f32) -> f32, emu: &mut Unicorn<'_, ()>) -> u32 {
    let esp = emu.reg_read(RegisterX86::ESP).unwrap_or(0);
    let val = read_f32(emu, esp + 4);
    write_st0(emu, f(val) as f64);
    0
}

fn h_atan2_handler(emu: &mut Unicorn<'_, ()>) -> u32 {
    let esp = emu.reg_read(RegisterX86::ESP).unwrap_or(0);
    let y = read_f64(emu, esp + 4);
    let x = read_f64(emu, esp + 12);
    write_st0(emu, y.atan2(x));
    0
}

fn h_pow_handler(emu: &mut Unicorn<'_, ()>) -> u32 {
    let esp = emu.reg_read(RegisterX86::ESP).unwrap_or(0);
    let base = read_f64(emu, esp + 4);
    let exp = read_f64(emu, esp + 12);
    write_st0(emu, base.powf(exp));
    0
}
