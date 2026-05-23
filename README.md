# dumpster_fire_engine

## Getting started

Run the setup script once after cloning. It installs all required tools automatically (LLVM 18, Vulkan SDK, CMake, Valgrind, iai-callgrind-runner) and writes an env file with the correct paths.

### Linux / macOS

```sh
bash setup.sh
source .env.toolchain
cargo build --workspace
```

### Windows

Open PowerShell and run:

```powershell
.\setup.ps1
cargo build --workspace
```

The script self-elevates to Administrator to install LLVM and the Vulkan SDK. The env file (`.env.toolchain.ps1`) is sourced automatically at the end of the script.

To make the environment permanent, add the following to your shell profile:

**Linux / macOS** (`~/.bashrc` or `~/.zshrc`):
```sh
source "/path/to/dumpster_fire_engine/.env.toolchain"
```

**Windows** (`$PROFILE`):
```powershell
. "C:\path\to\dumpster_fire_engine\.env.toolchain.ps1"
```

The setup script is safe to re-run after `git pull` — it skips anything already installed.
