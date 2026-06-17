use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::rc::Rc;

use goblin::elf::Elf;
use unicorn_engine::unicorn_const::{Arch, Mode, Prot};
use unicorn_engine::{RegisterX86, Unicorn};

const TRAMPOLINE_SIZE: u64 = 16;

pub const SO_BASE: u64 = 0x10000000;
pub const STACK_BASE: u64 = 0x70000000;
pub const STACK_SIZE: u64 = 0x01000000;
pub const HOSTCALL_BASE: u64 = 0x71000000;
pub const HOSTCALL_SIZE: u64 = 0x00100000;
pub const HEAP_BASE: u64 = 0x72000000;
pub const HEAP_SIZE: u64 = 0x08000000;
pub const SCRATCH_BASE: u64 = 0x7a000000;
pub const SCRATCH_SIZE: u64 = 0x04000000;

pub struct EmuState {
    pub heap_brk: u64,
    pub files: Vec<EmuFile>,
    pub stdout_buf: Vec<u8>,
    pub exit_code: Option<i32>,
}

pub enum EmuFile {
    Write(String, Vec<u8>, usize),
    Read(Vec<u8>, usize),
}

impl EmuState {
    pub fn new() -> Self {
        Self {
            heap_brk: HEAP_BASE,
            files: Vec::new(),
            stdout_buf: Vec::new(),
            exit_code: None,
        }
    }
}

pub struct Emulator {
    pub emu: Unicorn<'static, ()>,
    pub state: Rc<RefCell<EmuState>>,
    pub lib_exports: HashMap<String, u64>,
    pub exe_exports: HashMap<String, u64>,
    hostcall_names: Vec<String>,
}

impl Emulator {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let emu = Unicorn::new(Arch::X86, Mode::MODE_32)?;
        Ok(Emulator {
            emu,
            state: Rc::new(RefCell::new(EmuState::new())),
            lib_exports: HashMap::new(),
            exe_exports: HashMap::new(),
            hostcall_names: Vec::new(),
        })
    }

    pub fn map_memory(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let p = Prot::ALL;
        self.emu.mem_map(STACK_BASE, STACK_SIZE, p)?;
        self.emu.mem_map(HOSTCALL_BASE, HOSTCALL_SIZE, p)?;
        self.emu.mem_map(HEAP_BASE, HEAP_SIZE, p)?;
        self.emu.mem_map(SCRATCH_BASE, SCRATCH_SIZE, p)?;
        Ok(())
    }

    pub fn load_lib(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let data = fs::read(path)?;
        let elf = Elf::parse(&data)?;

        for sym in &elf.syms {
            if sym.st_shndx > 0 && sym.st_value > 0
                && let Some(name) = elf.strtab.get_at(sym.st_name)
                    && !name.is_empty() {
                        self.lib_exports
                            .insert(name.to_string(), SO_BASE + sym.st_value);
                    }
        }

        self.load_elf(&elf, &data, SO_BASE, false)?;

        eprintln!(
            "[emu] loaded lib {} ({} exports, {} imports)",
            path.file_name().unwrap().to_string_lossy(),
            self.lib_exports.len(),
            self.hostcall_names.len()
        );

        Ok(())
    }

    pub fn load_exe(&mut self, path: &Path) -> Result<u64, Box<dyn std::error::Error>> {
        let data = fs::read(path)?;
        let elf = Elf::parse(&data)?;
        let entry = elf.header.e_entry;

        for sym in &elf.syms {
            if sym.st_shndx > 0 && sym.st_value > 0
                && let Some(name) = elf.strtab.get_at(sym.st_name)
                    && !name.is_empty() {
                        self.exe_exports.insert(name.to_string(), sym.st_value);
                    }
        }

        self.load_elf(&elf, &data, 0, true)?;

        eprintln!(
            "[emu] loaded exe {} (entry=0x{:x}, {} symbols)",
            path.file_name().unwrap().to_string_lossy(),
            entry,
            self.exe_exports.len()
        );

        Ok(entry)
    }

    fn load_elf(
        &mut self,
        elf: &Elf,
        data: &[u8],
        base: u64,
        is_exe: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.load_segments(elf, data, base, is_exe)?;
        self.process_relocs(elf, base, is_exe)?;
        self.process_plt(elf, base, is_exe)?;
        Ok(())
    }

    fn load_segments(
        &mut self,
        elf: &Elf,
        data: &[u8],
        base: u64,
        is_exe: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for ph in &elf.program_headers {
            if ph.p_type == goblin::elf::program_header::PT_LOAD {
                let vaddr = if is_exe {
                    ph.p_vaddr
                } else {
                    base + ph.p_vaddr
                };
                let mem_size = ph.p_memsz.max(ph.p_filesz);
                if mem_size == 0 {
                    continue;
                }
                let aligned_addr = vaddr & !0xfff;
                let aligned_size = ((vaddr + mem_size + 0xfff) & !0xfff) - aligned_addr;

                self.emu
                    .mem_map(aligned_addr, aligned_size, Prot::ALL)
                    .map_err(|e| {
                        format!(
                            "mem_map 0x{:x} size 0x{:x}: {:?}",
                            aligned_addr, aligned_size, e
                        )
                    })?;

                let offset_in_page = (vaddr - aligned_addr) as usize;
                let mut buf = vec![0u8; aligned_size as usize];
                let file_start = ph.p_offset as usize;
                let file_size = ph.p_filesz as usize;
                if file_size > 0 {
                    let copy_len = file_size.min(buf.len() - offset_in_page);
                    buf[offset_in_page..offset_in_page + copy_len]
                        .copy_from_slice(&data[file_start..file_start + copy_len]);
                }
                self.emu.mem_write(aligned_addr, &buf)?;
            }
        }
        Ok(())
    }

    fn process_relocs(
        &mut self,
        elf: &Elf,
        base: u64,
        is_exe: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let resolve = |offset: u64| -> u64 {
            if is_exe {
                offset
            } else {
                base + offset
            }
        };

        if !elf.dynrelas.is_empty() {
            for rel in elf.dynrelas.iter() {
                if rel.r_type == goblin::elf::reloc::R_386_RELATIVE {
                    let addr = resolve(rel.r_offset);
                    let addend = rel.r_addend.unwrap_or(0) as u64;
                    let new_val = if is_exe {
                        addend as u32
                    } else {
                        (base as i64 + addend as i64) as u32
                    };
                    self.emu.mem_write(addr, &new_val.to_ne_bytes())?;
                }
            }
        } else {
            for rel in elf.dynrels.iter() {
                if rel.r_type == goblin::elf::reloc::R_386_RELATIVE {
                    let addr = resolve(rel.r_offset);
                    let mut val_bytes = [0u8; 4];
                    self.emu.mem_read(addr, &mut val_bytes)?;
                    let val = u32::from_ne_bytes(val_bytes) as u64;
                    let new_val = if is_exe {
                        val as u32
                    } else {
                        (val + base) as u32
                    };
                    self.emu.mem_write(addr, &new_val.to_ne_bytes())?;
                }
            }
        }

        Ok(())
    }

    fn process_plt(
        &mut self,
        elf: &Elf,
        base: u64,
        is_exe: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if elf.pltrelocs.is_empty() {
            return Ok(());
        }

        let resolve = |offset: u64| -> u64 {
            if is_exe {
                offset
            } else {
                base + offset
            }
        };

        for rel in elf.pltrelocs.iter() {
            let sym_idx = rel.r_sym;
            let sym = match elf.dynsyms.get(sym_idx) {
                Some(s) => s,
                None => continue,
            };
            let sym_name = elf.dynstrtab.get_at(sym.st_name).unwrap_or("").to_string();

            if sym_name.is_empty() {
                continue;
            }

            let got_addr = resolve(rel.r_offset);
            let target_addr: u32;

            if let Some(&lib_addr) = self.lib_exports.get(&sym_name) {
                target_addr = lib_addr as u32;
            } else {
                let idx = self.hostcall_names.len() as u64;
                target_addr = (HOSTCALL_BASE + idx * TRAMPOLINE_SIZE) as u32;

                let tramp: [u8; 16] = [0x90; 16];
                self.emu.mem_write(target_addr as u64, &tramp)?;

                self.hostcall_names.push(sym_name);
            }

            self.emu.mem_write(got_addr, &target_addr.to_ne_bytes())?;
        }

        Ok(())
    }

    pub fn setup_hostcall_hook(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let names = self.hostcall_names.clone();
        let state_ref = self.state.clone();

        let end = HOSTCALL_BASE + (names.len() as u64).max(1) * TRAMPOLINE_SIZE;

        self.emu.add_code_hook(
            HOSTCALL_BASE,
            end.max(HOSTCALL_BASE + 1),
            move |emu, addr, _size| {
                let idx = ((addr - HOSTCALL_BASE) / TRAMPOLINE_SIZE) as usize;
                if idx >= names.len() {
                    eprintln!("[hostcall] unknown trampoline 0x{:x}", addr);
                    emu.emu_stop().ok();
                    return;
                }

                let func_name = names[idx].clone();

                let esp = emu.reg_read(RegisterX86::ESP).unwrap_or(0);

                let mut ret_bytes = [0u8; 4];
                emu.mem_read(esp, &mut ret_bytes).ok();
                let ret_addr = u32::from_ne_bytes(ret_bytes) as u64;

                let mut args: Vec<u32> = Vec::new();
                for i in 0..20 {
                    let arg_addr = esp.wrapping_add(4).wrapping_add((i * 4) as u64);
                    let mut buf = [0u8; 4];
                    if emu.mem_read(arg_addr, &mut buf).is_ok() {
                        args.push(u32::from_ne_bytes(buf));
                    }
                }

                let mut st = state_ref.borrow_mut();
                let action = crate::hostcalls::dispatch(&mut st, &func_name, &args, emu);
                drop(st);

                match action {
                    HostcallAction::Return(val) => {
                        if let Some(v) = val {
                            emu.reg_write(RegisterX86::EAX, v as u64).ok();
                        }
                        emu.reg_write(RegisterX86::EIP, ret_addr).ok();
                        emu.reg_write(RegisterX86::ESP, esp + 4).ok();
                    }
                    HostcallAction::Stop => {
                        emu.emu_stop().ok();
                    }
                    HostcallAction::Skip => {}
                }
            },
        )?;

        Ok(())
    }

    pub fn run(&mut self, entry: u64, args: &[String]) -> Result<i32, Box<dyn std::error::Error>> {
        let env_strings: Vec<&str> = vec!["HOME=/", "PATH=/usr/bin"];
        let arg_strings: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let argc = args.len() as u32;

        let top = STACK_BASE + STACK_SIZE;
        let mut sp = top;

        let mut str_addrs: Vec<(u32, &[u8])> = Vec::new();
        for s in env_strings.iter().chain(arg_strings.iter()) {
            sp -= (s.len() + 1) as u64;
            str_addrs.push((sp as u32, s.as_bytes()));
        }

        for (addr, bytes) in &str_addrs {
            self.emu.mem_write(*addr as u64, bytes)?;
            self.emu
                .mem_write(*addr as u64 + bytes.len() as u64, &[0])?;
        }

        let num_env = env_strings.len();
        let num_args = arg_strings.len();

        sp -= 4;
        self.emu.mem_write(sp, &0u32.to_ne_bytes())?;
        for i in 0..num_env {
            sp -= 4;
            let addr = str_addrs[num_env - 1 - i].0;
            self.emu.mem_write(sp, &addr.to_ne_bytes())?;
        }

        sp -= 4;
        self.emu.mem_write(sp, &0u32.to_ne_bytes())?;
        for i in 0..num_args {
            sp -= 4;
            let addr = str_addrs[num_env + num_args - 1 - i].0;
            self.emu.mem_write(sp, &addr.to_ne_bytes())?;
        }

        sp -= 4;
        self.emu.mem_write(sp, &argc.to_ne_bytes())?;

        self.emu.reg_write(RegisterX86::ESP, sp)?;
        self.emu.reg_write(RegisterX86::EBP, 0)?;
        self.emu.reg_write(RegisterX86::EIP, entry)?;

        eprintln!(
            "[emu] jumping to _start at 0x{:x}, argc={}, esp=0x{:x}",
            entry, argc, sp
        );

        match self.emu.emu_start(entry, top, 0, 0) {
            Ok(_) => Ok(0),
            Err(e) => {
                let ec = self.state.borrow().exit_code.unwrap_or(1);
                if ec != 0 || matches!(e, unicorn_engine::unicorn_const::uc_error::FETCH_UNMAPPED) {
                    Ok(ec)
                } else {
                    Err(format!("emulation error: {:?}", e).into())
                }
            }
        }
    }
}

pub enum HostcallAction {
    Return(Option<u32>),
    Stop,
    Skip,
}
