# at3emu: Cross-Platform ATRAC3/ATRAC3plus Emulator in Rust

## What is at3tool?

**at3tool** is a proprietary Sony Computer Entertainment command-line tool for encoding and decoding **ATRAC3** and **ATRAC3plus** audio — the codecs used on PlayStation Portable (PSP) and PlayStation 3. It converts between 16-bit 44.1kHz PCM WAV files and ATRAC3/ATRAC3plus bitstreams with loop-point support for game audio.

The tool is distributed as **pre-built binaries only** (no source code):

- `at3tool` — Linux 32-bit ELF executable (~28 KB)
- `libatrac.so.1.2.0` — Linux 32-bit shared library (~1 MB), the core codec
- `at3tool.exe` — Windows 32-bit PE executable

**Problem**: These are 32-bit x86 Linux/Windows binaries. They cannot run natively on modern 64-bit systems, ARM64 (Apple Silicon), or macOS without emulation layers like Wine or Docker.

**Solution**: `at3emu` loads and runs the original Sony binaries via CPU emulation — no Wine, Docker, QEMU, or 32-bit compatibility libraries required. It works on macOS (ARM64/x86_64), Linux, and Windows.

---

## How It Works

### Architecture

```
┌─────────────────────────────────────────────────┐
│                  at3emu (Rust)                  │
│                                                 │
│  ┌──────────┐   ┌────────────┐   ┌───────────┐ │
│  │ ELF Loader│   │ CPU Emulator│   │ Hostcalls │ │
│  │ (goblin) │──▶│ (unicorn)  │◀──│ (glibc    │ │
│  │          │   │            │   │  shims)   │ │
│  └──────────┘   └─────┬──────┘   └───────────┘ │
│                       │                         │
│    at3tool ELF ───────┤                         │
│    libatrac.so ───────┤                         │
│                       │                         │
│                ┌──────▼──────┐                  │
│                │ Emulated x86│                  │
│                │   Linux CPU │                  │
│                │  (32-bit)  │                  │
│                └─────────────┘                  │
└─────────────────────────────────────────────────┘
```

### Key Crates

| Crate            | Version | Purpose                                                                                                              |
| ---------------- | ------- | -------------------------------------------------------------------------------------------------------------------- |
| `goblin`         | 0.10    | Pure-Rust ELF parser — reads at3tool and libatrac.so binary headers, segments, symbols, relocations                  |
| `unicorn-engine` | 2.1     | CPU emulator based on QEMU's TCG — emulates x86 32-bit instructions including SSE/SSE2/SSE3 required by the DSP code (feature `arch_x86`). Pulls in `unicorn-engine-sys` which compiles ~290K lines of C from source via `cc-rs` — no system library needed |

---

## Reverse Engineering: Finding Critical Addresses

Since at3tool and libatrac are binary-only (no source), we had to extract addresses from the ELF files. Here's exactly how each crucial address was found.

### Tools Used

```bash
nm      # List symbols from ELF binaries (available on Linux/macOS via binutils)
readelf # Inspect ELF headers and program segments
strings # Extract embedded strings from binaries
```

### Finding the Entry Point (`_start`)

```bash
$ nm linux/at3tool | grep _start
08048ac0 T _start
```

This gives us `_start` at absolute address `0x08048AC0`. The `T` means it's in the text (code) section, globally visible. This is the ELF entry point — the first instruction the kernel jumps to when launching the binary.

We confirm with:

```bash
$ readelf -h linux/at3tool | grep Entry
Entry point address: 0x8048ac0
```

### Finding `main`

```bash
$ nm linux/at3tool | grep " main$"
08048b74 T main
```

`main` is at `0x08048B74`. The trailing space + `main$` regex avoids matching symbols like `__libc_start_main`. We originally jumped here directly to bypass `_start`, but later switched to going through `_start` + `__libc_start_main` for proper C runtime initialization.

### Finding `_start` → `main` Flow

The standard glibc `_start` on x86 Linux does:

```asm
_start:
    xor  ebp, ebp          ; clear frame pointer
    pop  esi               ; esi = argc (from stack)
    mov  ecx, esp          ; ecx = argv (esp now points to argv[0])
    and  esp, -16          ; align stack to 16 bytes
    push eax               ; padding
    push esp               ; stack_end argument
    push edx               ; rtld_fini
    push <fini>            ; __libc_csu_fini
    push <init>            ; __libc_csu_init (or ecx in some versions)
    push ecx               ; ubp_av = argv
    push esi               ; argc
    push <main>            ; main function pointer (0x08048B74)
    call __libc_start_main ; dynamic linker resolves this via PLT
    hlt
```

We intercept `__libc_start_main` as a hostcall, read `main`'s address from its first argument, set up a proper stack frame, and redirect execution to `main(argc, argv, envp)`.

### Finding All Imported Symbols (glibc + libatrac)

The binary needs ~35 external functions. We list them:

```bash
$ nm -D --undefined-only linux/at3tool | awk '{print $2}' | sort -u
__gmon_start__
__libc_start_main@GLIBC_2.0
_Jv_RegisterClasses
atoi@GLIBC_2.0
atrac_decode                     # ← from libatrac.so
atrac_encode                     # ← from libatrac.so
atrac_flush_encode               # ← from libatrac.so
atrac_free_decode                # ← from libatrac.so
atrac_free_encode                # ← from libatrac.so
atrac_free_handle                # ← from libatrac.so
atrac_get_buffer_request         # ← from libatrac.so
atrac_get_error_code             # ← from libatrac.so
atrac_get_handle                 # ← from libatrac.so
atrac_get_version                # ← from libatrac.so
atrac_init_decode                # ← from libatrac.so
atrac_init_encode                # ← from libatrac.so
atrac_set_codec_info             # ← from libatrac.so
atrac_set_encode_algorithm       # ← from libatrac.so
exit@GLIBC_2.0
fclose@GLIBC_2.1
feof@GLIBC_2.0
fgetc@GLIBC_2.0
fopen@GLIBC_2.1
fprintf@GLIBC_2.0
fread@GLIBC_2.0
free@GLIBC_2.0
fseek@GLIBC_2.0
ftell@GLIBC_2.0
fwrite@GLIBC_2.0
malloc@GLIBC_2.0
memcmp@GLIBC_2.0
memcpy@GLIBC_2.0
memset@GLIBC_2.0
printf@GLIBC_2.0
putchar@GLIBC_2.0
puts@GLIBC_2.0
strcmp@GLIBC_2.0
```

We do the same for libatrac.so's own glibc dependencies:

```bash
$ nm -D --undefined-only linux/libatrac.so.1.2.0 | awk '{print $2}' | sort -u
__assert_fail@GLIBC_2.0
abs@GLIBC_2.0
asin@GLIBC_2.0
atan@GLIBC_2.0
atan2@GLIBC_2.0
calloc@GLIBC_2.0
cos@GLIBC_2.0
fabs@GLIBC_2.0
floor@GLIBC_2.0
floorf@GLIBC_2.0
free@GLIBC_2.0
labs@GLIBC_2.0
log@GLIBC_2.0
log10@GLIBC_2.0
malloc@GLIBC_2.0
memcpy@GLIBC_2.0
memmove@GLIBC_2.0
memset@GLIBC_2.0
pow@GLIBC_2.0
sin@GLIBC_2.0
sqrt@GLIBC_2.0
sqrtf@GLIBC_2.0
```

### Finding Exported atrac\_\* Function Addresses in libatrac.so

```bash
$ nm -D --defined-only linux/libatrac.so.1.2.0 | grep "^[0-9a-f]\+ [TW] " | grep atrac_
000074a0 T atrac_get_version
000074b0 T atrac_set_codec_info
00007500 T atrac_set_aux_codec_info
00007550 T atrac_set_encode_algorithm
000075a0 T atrac_set_decode_no_enhance
000075f0 T atrac_get_decode_output_channels
00007640 T atrac_get_buffer_request
00007770 T atrac_notify_last_frame
000077d0 T atrac_get_parsed_bytes
00007840 T atrac_get_error_code
00007860 T atrac_free_decode
000079b0 T atrac_clear_decode
00007c40 T atrac_decode
000083d0 T atrac_init_decode
00008e20 T atrac_free_handle
00008e60 T atrac_free_encode
000092a0 T atrac_flush_encode
000096d0 T atrac_encode
00009ce0 T atrac_get_handle
00009d80 T atrac_init_encode
```

Since libatrac.so is a shared library (ET_DYN), these are **base-relative offsets**. The actual runtime address is `SO_BASE + offset`. With our chosen load base of `0x10000000`, `atrac_encode` lives at `0x10000000 + 0x000096D0 = 0x100096D0`.

### Finding the at3tool Binary's Base Address

at3tool is a non-PIE executable (ET_EXEC). For such binaries, the ELF program headers specify **absolute** virtual addresses:

```bash
$ readelf -l linux/at3tool
Program Headers:
  Type   Offset   VirtAddr   PhysAddr   FileSiz MemSiz  Flg Align
  LOAD   0x000000 0x08048000 0x08048000 0x05394 0x05394 R E 0x1000
  LOAD   0x000f08 0x08049f08 0x08049f08 0x00118 0x00138 RW  0x1000
```

The first `PT_LOAD` segment starts at `VirtAddr = 0x08048000`. This is our `EXE_BASE`. We load the binary at this exact address — no relocation needed for absolute references.

### Finding the libatrac.so Load Address

libatrac.so is a shared library (ET_DYN) with position-independent code:

```bash
$ readelf -l linux/libatrac.so.1.2.0
Program Headers:
  Type   Offset   VirtAddr   PhysAddr   FileSiz MemSiz  Flg Align
  LOAD   0x000000 0x00000000 0x00000000 0xf2ab0 0xf2ab0 R E 0x1000
  LOAD   0x0f3000 0x000f3000 0x000f3000 0x03650 0x09b60 RW  0x1000
```

The first segment has `VirtAddr = 0x00000000` (relative). We choose a load base of `0x10000000` (256 MB) — high enough to avoid conflicting with the executable at `0x08048000` and its segments. The actual load address is `SO_BASE + p_vaddr`. All `R_386_RELATIVE` relocations must be adjusted by `SO_BASE`.

### Finding PLT/GOT Relocation Targets

The PLT relocations tell us where each imported function's GOT entry lives:

```bash
$ readelf -r linux/at3tool | grep JUMP_SLOT
08049fe0  00000107 R_386_JUMP_SLOT   00000000   __gmon_start__
08049fe4  00000207 R_386_JUMP_SLOT   00000000   __libc_start_main
08049fe8  00000407 R_386_JUMP_SLOT   00000000   malloc
08049fec  00000507 R_386_JUMP_SLOT   00000000   printf
08049ff0  00000607 R_386_JUMP_SLOT   00000000   puts
08049ff4  00000707 R_386_JUMP_SLOT   00000000   strcmp
08049ff8  00000807 R_386_JUMP_SLOT   00000000   atoi
08049ffc  00000907 R_386_JUMP_SLOT   00000000   fopen
...
0804a018  00001407 R_386_JUMP_SLOT   00000000   atrac_set_codec_info
0804a01c  00001607 R_386_JUMP_SLOT   00000000   atrac_encode
...
```

Each entry is a GOT slot at an absolute address (since at3tool is non-PIE). For example, the GOT entry for `atrac_encode` is at `0x0804A01C`. When the emulated code calls `atrac_encode@PLT`, it jumps through this GOT entry. We write the actual libatrac.so address (`0x100096D0`) here.

For glibc symbols like `printf`, the GOT entry at `0x08049FE4` gets a trampoline address in the hostcall region (e.g., `0x71000010`). The trampoline triggers our code hook, and the Rust handler takes over.

### Finding the at3tool Version and Supported Codecs

```bash
$ strings linux/at3tool | grep -E "Version|ATRAC3|bitrate|kbps"
SCEI ATRAC3plus Codec TOOL Version %s
*** built w/ ATRAC3plus library version %d.%02d ***
ATRAC3
ATRAC3plus
kbps
```

The codec parameter table is embedded as data in the binary. We extracted the 19 supported bitrate/channel combinations by analyzing the `gAtracCodecParam` table (the binary checks bitrate against this table during argument validation) and cross-referencing with the readme changelog.

### Finding the Stack Startup Layout

On Linux x86, the kernel sets up this stack before jumping to `_start`:

```
High addresses
  [environment strings, null-terminated]
  [argument strings, null-terminated]
  [NULL]                          ← end of envp[]
  [envp[N-1], ..., envp[0]]
  [NULL]                          ← end of argv[]
  [argv[argc-1], ..., argv[0]]
  [argc]                          ← ESP on entry
Low addresses
```

`_start` pops `argc`, then `esp` naturally points to `argv[0]`. We build this exact layout by:

1. Writing all string data at the top of the stack area
2. Building the envp pointer array below the strings
3. Building the argv pointer array below that
4. Writing `argc` at the stack bottom
5. Setting ESP to point to `argc`

### Finding the Calling Convention

All functions use **x86 cdecl** (32-bit calling convention):

- Arguments pushed right-to-left on the stack
- Caller cleans up the stack
- Integer/pointer return values in `EAX`
- Floating-point return values in `ST(0)` (x87 FPU stack)
- Stack aligned to 16 bytes at call sites

For hostcall interception, we read arguments from `[ESP+4]`, `[ESP+8]`, etc. (ESP points to the return address, pushed by the `call` instruction). After handling the call, we set `EAX` to the return value, `EIP` to `[ESP]` (return address), and increment `ESP` by 4 to pop the return address.

```rust
// Hostcall dispatch for math functions returning f64
let esp = emu.reg_read(RegisterX86::ESP)?;     // get stack pointer
let arg = read_f64(emu, esp + 4);              // first argument after return addr
let result = f(arg);                           // compute in Rust
write_st0(emu, result);                        // write to x87 ST(0)
emu.reg_write(RegisterX86::EIP, ret_addr)?;    // return to caller
emu.reg_write(RegisterX86::ESP, esp + 4)?;     // pop return address
```

---

## The Emulation Pipeline

### 1. Loading the ELF Binaries

Both `at3tool` (executable) and `libatrac.so` (shared library) are standard ELF 32-bit files. We use `goblin` to parse them and `unicorn-engine` to map them into emulated memory.

**Memory layout:**

| Region      | Address      | Size   | Purpose                                            |
| ----------- | ------------ | ------ | -------------------------------------------------- |
| at3tool     | `0x08048000` | ~24 KB | The CLI binary (non-PIE, absolute addresses)       |
| libatrac.so | `0x10000000` | ~1 MB  | The ATRAC3 codec library (PIC, relative addresses) |
| Stack       | `0x70000000` | 16 MB  | Emulated Linux stack                               |
| Hostcalls   | `0x71000000` | 1 MB   | Trampoline page for glibc function hooks           |
| Heap        | `0x72000000` | 128 MB | brk()-based heap (malloc/free)                     |
| Scratch     | `0x7A000000` | 64 MB  | Temporary buffers for API calls                    |

**Key insight**: `at3tool` is a non-PIE executable (ET_EXEC), so its `p_vaddr` fields are absolute addresses. `libatrac.so` is a shared library (ET_DYN), so its `p_vaddr` fields are base-relative. The loader handles both cases.

### 2. Processing Relocations

After loading segments into memory, we process ELF relocations:

- **R_386_RELATIVE**: Adjust addresses by the load base (for libatrac.so only)
- **R_386_JMP_SLOT** (PLT relocations): These are the critical ones — they determine where function calls go

### 3. The Hostcall Mechanism

This is the core innovation. When the emulated code calls a glibc function (like `malloc`, `printf`, `sin`), it goes through the PLT (Procedure Linkage Table) → GOT (Global Offset Table). We intercept this:

1. **During loading**: For each PLT entry referencing a glibc symbol, we write a **trampoline address** into the GOT entry instead of the real glibc address.

2. **Trampoline page**: A dedicated memory region (`0x71000000`) contains 16-byte trampolines (just NOPs). Each glibc import gets a unique trampoline slot.

3. **Code hook**: Unicorn's `UC_HOOK_CODE` fires when execution reaches any trampoline address. The hook handler:
   - Computes the function index from the trampoline address: `(addr - HOSTCALL_BASE) / 16`
   - Looks up the function name
   - Reads arguments from the emulated stack (x86 cdecl: `[ESP+4]`, `[ESP+8]`, ...)
   - Calls the Rust-side implementation
   - Writes the return value to `EAX` (and `ST0` for floating-point)
   - Sets `EIP` to the return address, adjusts `ESP`

4. **For atrac\_\* functions**: These are resolved to their actual addresses inside `libatrac.so` — they execute natively in the emulator, not through hostcalls.

```
Emulated code call flow:

at3tool main()
  ├─ printf("hello")      ──▶ trampoline → Rust printf()
  ├─ fopen("song.wav")    ──▶ trampoline → Rust fopen()
  ├─ fread(buf)            ──▶ trampoline → Rust fread()
  ├─ atrac_init_encode()  ──▶ executes in libatrac.so (real code)
  │   └─ malloc(4096)     ──▶ trampoline → Rust malloc()
  │   └─ sin(0.5)         ──▶ trampoline → Rust sin()
  │   └─ sqrt(2.0)        ──▶ trampoline → Rust sqrt()
  ├─ atrac_encode()       ──▶ executes in libatrac.so (real code)
  │   └─ ... many math calls via hostcalls ...
  ├─ fwrite(data)          ──▶ trampoline → Rust fwrite()
  ├─ fclose(f)             ──▶ trampoline → Rust fclose()
  └─ exit(0)              ──▶ trampoline → stop emulation
```

### 4. glibc Function Shims

We implement ~40 glibc functions in Rust:

| Category     | Functions                                                                                                               |
| ------------ | ----------------------------------------------------------------------------------------------------------------------- |
| **Memory**   | `malloc`, `free`, `calloc`, `realloc`, `memcpy`, `memmove`, `memset`, `memcmp`                                          |
| **File I/O** | `fopen`, `fclose`, `fread`, `fwrite`, `fseek`, `ftell`, `feof`, `fgetc`                                                 |
| **Output**   | `printf`, `fprintf`, `puts`, `putchar`                                                                                  |
| **String**   | `strcmp`, `atoi`                                                                                                        |
| **Math**     | `abs`, `labs`, `fabs`, `sin`, `cos`, `asin`, `atan`, `atan2`, `log`, `log10`, `pow`, `sqrt`, `sqrtf`, `floor`, `floorf` |
| **Runtime**  | `exit`, `__libc_start_main`, `__gmon_start__`, `_Jv_RegisterClasses`, `__assert_fail`                                   |

The math functions return results via the **x87 FPU stack** (`ST0` register). We convert f64 values to the 80-bit x87 extended precision format:

```
f64 → x87 (10 bytes):
  Bit 79:    sign
  Bits 78-64: exponent (bias 16383)
  Bits 63-0:  mantissa (explicit leading 1)
```

File I/O uses Rust's `std::fs` — files are read entirely into memory on `fopen` and written on `fclose`. The `Write` file type tracks a seek position so the binary can seek back to update the WAV header after writing audio data.

### 5. Program Startup

We set up a Linux-compatible stack and jump to the `_start` entry point:

```
Stack layout (high → low):
  [string data for argv and envp]
  [NULL terminator]
  [envp[0], envp[1], ..., NULL]
  [NULL terminator]
  [argv[0], argv[1], ..., NULL]
  [argc]                  ← ESP on entry
```

`_start` parses argc/argv, calls `__libc_start_main` (which we intercept), which in turn calls `main(argc, argv, envp)`. When `main` calls `exit()`, we call `emu_stop()` and return the exit code.

---

## Supported Features

### CLI Interface (identical to at3tool)

```
Usage : at3emu [-<option>] file1 file2
  -e        : encode file1 (PCM WAV) → file2 (ATRAC3)
  -d        : decode file1 (ATRAC3) → file2 (PCM WAV)
  -br N     : bitrate in kbps
  -loop S E : loop start/end in samples
  -wholeloop: set whole file as loop
  -repeat N : repeat loop N times during decode (default 2)

Extra:
  --at3tool <path>    Path to at3tool binary
  --libatrac <path>   Path to libatrac.so.1.2.0
  --list-codecs       Show all supported bitrate/channel combinations
```

### Bitrate/Channel Combinations

**ATRAC3** (codec type 3, 1024-sample frames):
| Bitrate | Channels |
|---------|----------|
| 52 | Mono |
| 66 | Mono, Stereo |
| 105 | Stereo |
| 132 | Stereo |

**ATRAC3plus** (codec type 5, 2048-sample frames):
| Bitrate | Channels |
|---------|----------|
| 32, 48, 64, 96, 128 | Mono |
| 48, 64, 96, 128, 160, 192, 256, 320, 352 | Stereo |

---

## Building

### Prerequisites

- Rust 1.85+ (edition 2024; pinned to 1.96.0 via `rust-toolchain.toml`)
- C compiler (gcc/clang on Linux/macOS, MSVC Build Tools or MinGW on Windows)
- CMake (automatically used by `unicorn-engine-sys`)

### Build

```bash
# Debug build (slow, verbose)
cargo build

# Release build (fast, optimized)
cargo build --release
```

The first build compiles Unicorn Engine from ~290K lines of C source. This takes ~45 seconds on a modern machine. Subsequent builds are fast (~0.3s for Rust changes only).

The binary is output to `target/release/at3emu` (or `target/debug/at3emu`).

### Running

```bash
# Binary auto-discovery (looks in ./linux/ next to the executable)
cargo run --release -- -e -br 66 input.wav output.at3

# Or run the compiled binary directly
./target/release/at3emu -e -br 66 input.wav output.at3

# Explicit paths
cargo run --release -- --at3tool ./linux/at3tool --libatrac ./linux/libatrac.so.1.2.0 -e -br 66 input.wav output.at3
```

---

## Cross-Platform Support

The emulator works on **macOS (ARM64/x86_64), Linux (x86_64/ARM64), and Windows** because:

- `unicorn-engine` compiles from embedded C source via `cc-rs` — no system library needed
- `goblin` is pure Rust — ELF parsing works anywhere
- File I/O uses `std::fs` — native OS file APIs
- The emulated code is always 32-bit x86 Linux, regardless of host OS
- CPU emulation is software-based — no hardware compatibility requirements

The only requirement is a C compiler for building `unicorn-engine-sys`.

---

## Verification Against Real at3tool

To verify correctness, we compared output against the real `at3tool` running in Docker:

### Docker Setup

```dockerfile
FROM i386/ubuntu:18.04

COPY linux/at3tool /opt/at3tool/
COPY linux/libatrac.so.1.2.0 /opt/at3tool/
RUN ln -sf libatrac.so.1.2.0 /opt/at3tool/libatrac.so.1
ENV LD_LIBRARY_PATH=/opt/at3tool

ENTRYPOINT ["/opt/at3tool/at3tool"]
```

```bash
docker build -t at3tool .
docker run --platform linux/386 --rm -v "$(pwd):/data" at3tool -e -br 66 /data/test.wav /data/test.at3
```

### Results

| Test                                           | Result                                               |
| ---------------------------------------------- | ---------------------------------------------------- |
| Encode 66kbps stereo (small file)              | **Byte-for-byte identical** AT3 output               |
| Encode 66kbps stereo (4-min song)              | Same size, different bytes but valid AT3             |
| Encode 132kbps stereo LP2                      | Same size, different bytes but valid AT3             |
| Decode Docker AT3 via emulator                 | **Byte-for-byte identical** to Docker's own decode   |
| Decode emulator AT3 via Docker                 | **Byte-for-byte identical** to emulator's own decode |
| Null test (original − decoded, phase-inverted) | Near silence with occasional blips                   |

The encode output differs slightly for larger files due to floating-point precision variations in the psychoacoustic encoder math. Both produce valid, decodable ATRAC3 bitstreams. The decode path is 100% deterministic.

---

## Performance

Benchmark: 4-minute stereo 44.1kHz WAV → ATRAC3 132kbps LP2 (10,345 frames)

| Tool                  | Time     | Ratio     |
| --------------------- | -------- | --------- |
| Docker (real at3tool) | 1:57     | 1.00x     |
| **at3emu**            | **2:17** | **1.17x** |

The emulator is only 17% slower than the real binary — impressive considering it's doing full software x86 CPU emulation. The overhead comes primarily from the hostcall dispatch mechanism: each math function call (`sin`, `pow`, `log10`, etc.) fires a code hook, reads arguments from emulated memory, computes the result, and writes x87 FPU registers. This happens tens of thousands of times per second during encoding.

---

## Project Structure

```
at3tool-emu/
├── Cargo.toml              # Workspace root (resolver="3")
├── crates/
│   ├── at3emu/             # CLI binary crate
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── main.rs     # CLI entry point, argument parsing, binary auto-discovery
│   └── at3emu-core/        # Core library crate
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs      # Library root, re-exports Emulator
│           ├── emu.rs      # ELF loader, CPU emulator setup, hostcall dispatch
│           └── hostcalls.rs # glibc function shims (malloc, printf, sin, etc.)
└── linux/
    ├── at3tool             # Original Sony at3tool binary (32-bit ELF)
    └── libatrac.so.1.2.0   # Original Sony libatrac codec (32-bit ELF)
```

---

## Limitations

- **Noise-reduction encoding** (v3.0.0.0 feature for loop sources) may differ from original due to FPU precision
- **32-bit x86 only** — the emulated binary and library are 32-bit; cannot use a 64-bit version of libatrac (doesn't exist)
- **Performance ceiling** — the hostcall dispatch overhead (code hooks for every glibc call) is the fundamental bottleneck; CPU emulation itself is near optimal

---

## License Note

The `at3tool` and `libatrac.so` binaries are **SCE Confidential** software copyrighted by Sony Computer Entertainment Inc. This emulator does not include, modify, or redistribute the Sony binaries — it only loads and runs them via CPU emulation. Users must obtain the binaries through proper licensing channels.
