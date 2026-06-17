# at3emu

`at3tool` CPU emulation via ([Unicorn Engine](https://www.unicorn-engine.org/)) to run the original 32bit x86 Linux ELF binary.

Primarily built for macOS. Windows works as well, but is extremely slow: use `at3tool.exe` instead. Untested on Linux, but the native ELF binary should work natively as well.

```sh
# show original help
at3emu

# encode 16-bit 44.1kHz PCM WAV
# (LP2 = 132kbps, LP4 = 66kbps)
at3emu -e -br 132 song.wav song.at3

# decode
at3emu -d song.at3 song_decoded.wav
```

## Download

Prebuilt binaries for are available on the [Releases](https://github.com/liangchunn/at3emu/releases) page.

1. Download the prebuilt binary
2. Place `libatrac.so.1.2.0` and `at3tool` inside a `linux` folder next to the binary

   ```
   at3emu
   linux/
   ├── at3tool
   └── libatrac.so.1.2.0
   ```

3. Run the binary. See [Usage](#usage) below for all options.

## Setup

### 1. Prerequisites

- **Rust** 1.96.0+ ([rustup.rs](https://rustup.rs))
- **A C compiler** — gcc/clang on macOS
- **CMake** (usually included with build tools; install via `brew install cmake` if missing)

### 2. Get the Sony binaries

You need two files from the PSP SDK (not included in this repo):

```
linux/
├── at3tool              # The CLI tool (28 KB, ELF 32-bit)
└── libatrac.so.1.2.0    # The codec library (1 MB, ELF 32-bit)
```

Place them in a `linux/` directory next to the executable, or pass paths explicitly.

### 3. Build

```bash
git clone <this-repo>
cd at3tool-emu
cargo build --release
```

### 4. Run

```bash
# Auto-discovers binaries in ./linux/
./target/release/at3emu -e -br 132 input.wav output.at3

# Or specify paths explicitly
./target/release/at3emu --at3tool /path/to/at3tool --libatrac /path/to/libatrac.so.1.2.0 -e -br 132 song.wav song.at3
```

## Usage

```
at3emu [-<option>] file1 file2

Options:
  -e                   Encode: PCM WAV → ATRAC3/ATRAC3plus
  -d                   Decode: ATRAC3/ATRAC3plus → PCM WAV
  -br N                Bitrate in kbps
  -loop S E            Loop start/end in samples
  -wholeloop           Loop the entire file
  -repeat N            Repeat loop N times during decode (default: 2)

Extra:
  --at3tool <path>     Path to at3tool binary
  --libatrac <path>    Path to libatrac.so.1.2.0
  --list-codecs        Show all supported bitrate/channel combinations
```

### Supported Bitrates

| Codec      | Bitrates                                 | Channels     |
| ---------- | ---------------------------------------- | ------------ |
| ATRAC3     | 52                                       | Mono         |
| ATRAC3     | 66                                       | Mono, Stereo |
| ATRAC3     | 105, 132                                 | Stereo       |
| ATRAC3plus | 32, 48, 64, 96, 128                      | Mono         |
| ATRAC3plus | 48, 64, 96, 128, 160, 192, 256, 320, 352 | Stereo       |

Input WAV must be 16-bit linear PCM, 44100 Hz.

## Verification

Output is verified byte-for-byte against the real at3tool running in Docker:

| Test                           | Result                                    |
| ------------------------------ | ----------------------------------------- |
| Encode 66kbps (small file)     | Byte-for-byte identical                   |
| Encode 66kbps (4-min song)     | Valid AT3, imperceptible audio difference |
| Encode 132kbps LP2             | Valid AT3, imperceptible audio difference |
| Decode (any AT3 input)         | Byte-for-byte identical to real at3tool   |
| Null test (original − decoded) | Near silence                              |

Performance is ~1.17x slower than the native binary (2m17s vs 1m57s for a 4-minute song on Apple M2).

## License

This project is MIT licensed. The Sony `at3tool` and `libatrac.so` binaries are SCE Confidential and must be obtained separately through proper PSP SDK licensing channels. This emulator does not include or distribute them.

## TODO

- [ ] expose nicer core API (right now it's a bit messy to set up everything)
