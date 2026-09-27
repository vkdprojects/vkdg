# Building on Windows

VKDG builds and runs on Windows. This page covers the setup that makes it fast and the known gotchas.

**Shortcut:** If you want the fastest path to a working environment on any OS, use the devcontainer. See [Devcontainer](#devcontainer) below.

---

## Prerequisites

- [Rust](https://rustup.rs) via rustup
- [bun](https://bun.sh) for the SvelteKit console
- [just](https://just.systems/man/en/packages.html) — task runner (`cargo install just`)
- Visual Studio Build Tools 2022 or later with the **MSVC** and **Windows SDK** components

```bash
rustup component add rust-analyzer rust-src rustfmt clippy
```

---

## Windows Defender exclusions — do this first

Windows Defender scans every file written to `target/` during a build. For a 22-crate workspace this means thousands of file-system events per rebuild. **Adding exclusions is the single highest-ROI change you can make.**

Open PowerShell as administrator:

```powershell
# Exclude the project workspace and cargo registry from real-time scanning
Add-MpPreference -ExclusionPath "C:\path\to\omg\target"
Add-MpPreference -ExclusionPath "$env:USERPROFILE\.cargo\registry"
Add-MpPreference -ExclusionPath "$env:USERPROFILE\.cargo\git"
Add-MpPreference -ExclusionPath "$env:USERPROFILE\.rustup"
```

With corporate AV (not Defender): add the same paths via your AV console. This typically requires a ticket to IT — it's worth it.

**Expected improvement:** 3–10× faster incremental builds, depending on your AV configuration.

---

## Long paths

Windows limits path length to 260 characters by default. Rust deeply-nested crate paths exceed this. Enable long paths:

```powershell
# Run as Administrator
git config --system core.longpaths true
New-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem" `
  -Name "LongPathsEnabled" -Value 1 -PropertyType DWORD -Force
```

---

## Windows Dev Drive (optional, Windows 11 22H2+)

Dev Drive is a ReFS-based virtual disk that Windows exempts from real-time AV scanning by default. Moving your workspace there gives you the AV exclusion benefit without needing admin rights on every machine.

[Set up Dev Drive](https://learn.microsoft.com/en-us/windows/dev-drive/) — takes about 15 minutes. Move both the workspace and `~/.cargo` into the Dev Drive for best results.

---

## The linker

The `.cargo/config.toml` in this repo sets `rust-lld.exe` as the linker on Windows. It ships with rustup since Rust 1.80 — no extra install needed. This typically cuts link time by 2–4× vs the default `link.exe`.

If you see `STATUS_ACCESS_VIOLATION` during linking, add this to a local `.cargo/config.toml` (not committed) to override it:

```toml
[target.x86_64-pc-windows-msvc]
linker = "link.exe"
```

Then open an issue — this is rare but has been reported on some setups.

---

## Building

```bash
# Dev build (no bun step needed — rust-embed reads files from disk in dev mode)
cargo build -p vkdg

# Run the gateway
VKDG_BOOTSTRAP_TOKEN=dev cargo run -p vkdg -- serve

# Full release build (builds console + embeds it in the binary)
just build

# Run tests
just test
```

**Note:** `cargo build` (debug profile) does **not** require `bun run build`. The SvelteKit console is served from the filesystem in dev mode via rust-embed's debug path. Only `just build` (release) embeds the console into the binary.

---

## WSL2 (alternative to native)

WSL2 builds run inside an ext4 filesystem, bypassing NTFS and Windows Defender entirely. If native Windows builds are still slow after the AV exclusions, WSL2 is the next step.

**Important:** put the workspace **inside** WSL2 (`~/` or `/home/user/`), not on `/mnt/c/`. Cross-filesystem I/O between WSL2 and Windows NTFS is slow and eliminates the benefit.

```bash
# Inside WSL2
git clone https://github.com/vkdprojects/vkdg ~/vkdg
cd ~/vkdg
curl -fsSL https://bun.sh/install | bash
cargo build -p vkdg
```

---

## Devcontainer

The repo ships a `.devcontainer/` setup. This gives you a Linux environment (ext4, mold linker, no AV overhead) regardless of your host OS.

**Requirements:** VS Code + [Dev Containers extension](https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-containers), or [GitHub Codespaces](https://codespaces.new/vkdprojects/vkdg).

```
1. Clone the repo
2. Open in VS Code
3. "Reopen in Container" (VS Code will prompt, or use Ctrl+Shift+P)
4. Wait for the image to build (first time ~5 minutes, cached after that)
5. just dev
```

This is also how GitHub Codespaces works — click "Code → Codespaces → Create codespace on main" to get a fully configured browser-based environment with zero local setup.

---

## Expected build times

| Setup | Clean build | Incremental |
|---|---|---|
| Windows, no exclusions | 10–20 min | 2–5 min |
| Windows + AV exclusions | 3–5 min | 30–90s |
| Windows + Dev Drive | 2–3 min | 20–60s |
| WSL2 / devcontainer | 1.5–2.5 min | 10–30s |
| macOS M-series | ~40s | 5–15s |

Times are for the full 22-crate workspace. Individual crate rebuilds (`cargo build -p vkdg-http`) are much faster.
